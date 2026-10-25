//! LogOS hosted Core (ADR-0096): runs the unchanged service images as OS
//! processes, hands each one its fixed ABI windows as shared memory, serves the
//! `int 49` syscalls over the child's stdin/stdout, and presents the Display
//! framebuffer in a native window.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use logos_abi as abi;
use logos_host_shm::{Mapping, env_name, region};

const SYSCALL_YIELD: usize = 1;
const SYSCALL_HEARTBEAT: usize = 10;
const FRAME_BYTES: usize = 24;
const WIDTH: usize = abi::DEFAULT_SCREEN_WIDTH;
const HEIGHT: usize = abi::DEFAULT_SCREEN_HEIGHT;

/// Boot order and per-service windows beyond the implicit bootstrap + ipc pair.
const IMAGES: &[(&str, &[&str])] = &[
    ("input", &["input"]),
    ("display", &["display", "display_config"]),
    ("terminal", &[]),
    ("session", &[]),
    ("flow", &[]),
    ("storage", &[]),
    ("network", &[]),
    ("fetch", &[]),
    ("device", &[]),
    ("user", &[]),
    ("shell", &[]),
    ("lockscreen", &[]),
    ("atrium", &[]),
    ("system", &[]),
];

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

struct Clock(Instant);

impl Clock {
    fn ticks(&self) -> u64 {
        self.0.elapsed().as_millis() as u64 * abi::WALL_CLOCK_TICKS_PER_SECOND / 1000
    }
}

/// Civil UTC date from a Unix timestamp (Howard Hinnant's days-to-civil).
fn wall_time(unix_seconds: u64) -> abi::WallTime {
    let days = (unix_seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    let seconds = unix_seconds % 86_400;
    abi::WallTime {
        year,
        month,
        day,
        hour: (seconds / 3600) as u8,
        minute: (seconds % 3600 / 60) as u8,
        second: (seconds % 60) as u8,
    }
}

struct Link {
    name: &'static str,
    ipc: Mapping,
    _bootstrap: Mapping,
}

impl Link {
    fn staging(&self) -> *mut u8 {
        unsafe { self.ipc.as_ptr().add(abi::IPC_STAGING_BASE - abi::SERVICE_IPC_BASE) }
    }
}

fn dispatch(link: &Link, clock: &Clock, number: usize, rdi: usize, rsi: usize) -> usize {
    match number {
        SYSCALL_YIELD | SYSCALL_HEARTBEAT | abi::PROGRAM_EXIT_SYSCALL => 0,
        abi::CURRENT_TICKS_SYSCALL => clock.ticks() as usize,
        abi::WALL_TIME_SYSCALL => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
            // ponytail: UTC only; local-time offset lands with the Linux-distro work.
            wall_time(now.as_secs()).pack() as usize
        }
        abi::SYSTEM_INFO_SYSCALL => {
            let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()).min(255) as u8;
            // ponytail: memory totals read 0 until a host probe exists.
            abi::SystemInfo { cpus, mem_total_mib: 0, mem_used_mib: 0 }.pack() as usize
        }
        abi::DEBUG_LINE_SYSCALL => {
            let length = rdi.min(abi::IPC_PAGE_BYTES);
            let bytes = unsafe { std::slice::from_raw_parts(link.staging(), length) };
            println!("[{}] {}", link.name, String::from_utf8_lossy(bytes));
            0
        }
        abi::POWER_SYSCALL => {
            println!("[core] {} requested power action {rdi}", link.name);
            SHUTDOWN.store(true, Ordering::Release);
            0
        }
        abi::SERVICE_HEAP_GROW_SYSCALL | abi::SERVICE_HEAP_SHRINK_SYSCALL => {
            abi::IpcStatus::Malformed as usize
        }
        abi::SERVICE_DIRECTORY_SYSCALL => {
            // ponytail: no service graph on the host yet; see the hosted IPC graph issue.
            abi::DirectoryStatus::NotFound as usize
        }
        abi::EVENT_SYSCALL => {
            let request = unsafe { (link.staging() as *const abi::EventRequest).read_unaligned() };
            let response =
                abi::EventResponse::empty(abi::EventStatus::Capacity, request.request_id);
            unsafe { (link.staging() as *mut abi::EventResponse).write_unaligned(response) };
            abi::EventStatus::Capacity as usize
        }
        abi::IPC_SYSCALL_SEND | abi::IPC_SYSCALL_RECEIVE => abi::IpcStatus::Disconnected as usize,
        abi::MANAGER_SYSCALL => abi::IpcStatus::Unauthorized as usize,
        _ => {
            println!("[core] {}: unknown syscall {number} ({rdi}, {rsi})", link.name);
            abi::IpcStatus::Malformed as usize
        }
    }
}

fn serve(link: Link, clock: Arc<Clock>, mut from: ChildStdout, mut to: ChildStdin) {
    let mut frame = [0u8; FRAME_BYTES];
    let mut count = 0u64;
    while from.read_exact(&mut frame).is_ok() {
        let word = |index: usize| {
            u64::from_le_bytes(frame[index * 8..index * 8 + 8].try_into().unwrap()) as usize
        };
        let (number, rdi, rsi) = (word(0), word(1), word(2));
        count += 1;
        if count == 1 {
            println!("[core] {}: first syscall {number}", link.name);
        }
        let rax = dispatch(&link, &clock, number, rdi, rsi);
        if to.write_all(&(rax as u64).to_le_bytes()).is_err() {
            break;
        }
    }
    println!("[core] {}: link closed after {count} syscalls", link.name);
}

fn shared(session: u32, service: Option<&str>, name: &str) -> std::io::Result<Mapping> {
    let region = region(name).expect("known region");
    let token = match service {
        Some(service) => format!("Local\\logos-{session}-{service}-{name}"),
        None => format!("Local\\logos-{session}-{name}"),
    };
    logos_host_shm::create(&token, region.bytes)
}

fn main() {
    let session = std::process::id();
    let clock = Arc::new(Clock(Instant::now()));
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_owned()));
    let exe_dir = exe_dir.expect("host binary directory");

    let display = shared(session, None, "display").expect("display window");
    let display_config = shared(session, None, "display_config").expect("display config");
    let input = shared(session, None, "input").expect("input rings");
    unsafe {
        (display_config.as_ptr() as *mut abi::FramebufferConfig).write(
            abi::FramebufferConfig::new(
                (WIDTH * HEIGHT * 4) as u64,
                WIDTH as u32,
                HEIGHT as u32,
                (WIDTH * 4) as u32,
                abi::FramebufferFormat::Bgr8,
            ),
        );
        (display_config.as_ptr().add(abi::DISPLAY_PRESENT_BASE - abi::DISPLAY_CONFIG_BASE)
            as *mut abi::FramebufferPresentState)
            .write(abi::FramebufferPresentState::new());
        (input.as_ptr() as *mut abi::KeyboardByteRing).write(abi::KeyboardByteRing::new());
        (input.as_ptr().add(abi::INPUT_POINTER_RING_BASE - abi::INPUT_KEYBOARD_RING_BASE)
            as *mut abi::PointerByteRing)
            .write(abi::PointerByteRing::new());
    }
    let present = unsafe {
        &*(display_config.as_ptr().add(abi::DISPLAY_PRESENT_BASE - abi::DISPLAY_CONFIG_BASE)
            as *const abi::FramebufferPresentState)
    };
    let shared_tokens =
        [("display", &display), ("display_config", &display_config), ("input", &input)];

    let children: Arc<Mutex<Vec<Child>>> = Arc::new(Mutex::new(Vec::new()));
    for (index, (name, extra)) in IMAGES.iter().enumerate() {
        let bootstrap = shared(session, Some(name), "bootstrap").expect("bootstrap page");
        let ipc = shared(session, Some(name), "ipc").expect("ipc window");
        let generation = 1;
        unsafe {
            (bootstrap.as_ptr() as *mut abi::ServiceBootstrapPage).write(
                abi::ServiceBootstrapPage {
                    abi_version: abi::ABI_VERSION,
                    flags: 0,
                    ipc_generation: generation as u16,
                    reserved: 0,
                    service_epoch: 1,
                    service: abi::ServiceHandle::new(index as u32, generation).unwrap(),
                    control: abi::CapabilityHandle::new(index as u32, generation).unwrap(),
                    directory: abi::CapabilityHandle::new(index as u32 + 64, generation).unwrap(),
                    heap: abi::CapabilityHandle::EMPTY,
                    heap_base: 0,
                    heap_pages: 0,
                    heap_quota_pages: 0,
                },
            );
        }
        let mut command =
            Command::new(exe_dir.join(format!("logos-{name}{}", std::env::consts::EXE_SUFFIX)));
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        command.env(env_name("bootstrap"), bootstrap.token());
        command.env(env_name("ipc"), ipc.token());
        for (region_name, mapping) in shared_tokens {
            if extra.contains(&region_name) {
                command.env(env_name(region_name), mapping.token());
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                println!("[core] {name}: spawn failed: {error}");
                continue;
            }
        };
        let (from, to) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
        println!("[core] {name}: started pid {}", child.id());
        children.lock().unwrap().push(child);
        let link = Link { name, ipc, _bootstrap: bootstrap };
        let clock = Arc::clone(&clock);
        std::thread::spawn(move || serve(link, clock, from, to));
    }
    println!("LogOS host: core ready");

    let mut window = minifb::Window::new("LogOS", WIDTH, HEIGHT, minifb::WindowOptions::default())
        .expect("native window");
    window.set_target_fps(60);
    let mut pixels = vec![0u32; WIDTH * HEIGHT];
    let mut presented = u32::MAX;
    while window.is_open() && !SHUTDOWN.load(Ordering::Acquire) {
        let sequence = present.sequence();
        if sequence != presented {
            presented = sequence;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    display.as_ptr() as *const u32,
                    pixels.as_mut_ptr(),
                    pixels.len(),
                );
            }
            window.update_with_buffer(&pixels, WIDTH, HEIGHT).expect("present");
        } else {
            window.update();
        }
    }
    println!("[core] shutting down");
    for child in children.lock().unwrap().iter_mut() {
        let _ = child.kill();
    }
    std::thread::sleep(Duration::from_millis(50));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_time_matches_known_dates() {
        let t = wall_time(0);
        assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (1970, 1, 1, 0, 0, 0));
        // 2026-10-25T12:34:56Z
        let t = wall_time(1_792_931_696);
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (2026, 10, 25, 12, 34, 56)
        );
        // Leap day.
        let t = wall_time(1_709_164_800);
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));
    }
}
