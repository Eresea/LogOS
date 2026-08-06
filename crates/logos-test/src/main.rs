use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::{
    cell::Cell,
    env, fs,
    io::{self, BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use logos_abi::{NamespaceId, TERMINAL_NAMESPACE};
use logos_store::{Error as StoreError, SECTOR_SIZE, SectorBackend, Store};
use logos_terminal::terminal::{HISTORY_BYTES, Model, Submission};

mod network_peer;
mod suites;
use network_peer::NetworkPeer;
use suites::{Fixture, Runner, SCENARIOS, Scenario};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Passed,
    Failed,
    Skipped,
}

struct ResultRecord {
    id: String,
    status: Status,
    duration_ms: u128,
    seed: u64,
    failure: Option<String>,
    artifacts: PathBuf,
}

struct Progress {
    total: usize,
    interactive: bool,
    state: Mutex<ProgressState>,
}

#[derive(Default)]
struct ProgressState {
    completed: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    current: Option<String>,
}

impl Progress {
    fn new(total: usize) -> Self {
        let progress = Self {
            total,
            interactive: io::stdout().is_terminal(),
            state: Mutex::new(ProgressState::default()),
        };
        progress.print();
        progress
    }

    fn record(&self, result: &ResultRecord) {
        let mut state = self.state.lock().expect("progress mutex poisoned");
        state.completed += 1;
        match result.status {
            Status::Passed => state.passed += 1,
            Status::Failed => state.failed += 1,
            Status::Skipped => state.skipped += 1,
        }
        self.render(&state);
    }

    fn start(&self, id: &str) {
        let mut state = self.state.lock().expect("progress mutex poisoned");
        state.current = Some(id.into());
        self.render(&state);
    }

    fn print(&self) {
        let state = self.state.lock().expect("progress mutex poisoned");
        self.render(&state);
    }

    fn finish(&self) {
        if self.interactive {
            println!();
        }
    }

    fn render(&self, state: &ProgressState) {
        let line = format!(
            "[{}/{}] {} passed={} failed={}, skipped={}",
            state.completed,
            self.total,
            state.current.as_deref().unwrap_or("-"),
            state.passed,
            state.failed,
            state.skipped
        );
        if self.interactive {
            print!("\r\x1b[2K{line}");
            let _ = io::stdout().flush();
        } else {
            println!("{line}");
        }
    }
}

#[derive(Clone)]
struct ImageProfile {
    efi: PathBuf,
    terminal: Option<PathBuf>,
    sessions: Option<PathBuf>,
    storage: Option<PathBuf>,
    network: Option<PathBuf>,
    gateway: Option<PathBuf>,
    incompatible_sessions: bool,
    block_probe: bool,
    logosctl: PathBuf,
}

impl ImageProfile {
    fn for_fixture(&self, fixture: Fixture) -> Self {
        let mut profile = self.clone();
        match fixture {
            Fixture::MissingSessions => profile.sessions = None,
            Fixture::MissingTerminal => profile.terminal = None,
            Fixture::IncompatibleSessions => profile.incompatible_sessions = true,
            Fixture::MissingStore => profile.storage = None,
            Fixture::MissingNetwork => profile.network = None,
            Fixture::Shared | Fixture::Fresh | Fixture::Persistence => {}
        }
        profile
    }
}

const DISK_SECTORS: usize = 16 * 1024 * 1024 / SECTOR_SIZE;
const HISTORY_NAME: &[u8] = b"history";
static BOOT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
struct BootReport {
    session: u64,
    storage: String,
}

fn parse_boot_report(line: &str) -> Result<BootReport, String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("LOGOS/1") && words.next() == Some("BOOT"))
        .then_some(())
        .ok_or_else(|| format!("invalid boot report: {line}"))?;
    let session = words
        .next()
        .and_then(|word| word.strip_prefix("session="))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("invalid boot report: {line}"))?;
    let storage = words
        .next()
        .and_then(|word| word.strip_prefix("storage="))
        .filter(|value| {
            matches!(
                *value,
                "formatted"
                    | "recovered"
                    | "recovered-incomplete"
                    | "corrupt"
                    | "io-failed"
                    | "unavailable"
            )
        })
        .ok_or_else(|| format!("invalid boot report: {line}"))?;
    words
        .next()
        .is_none()
        .then_some(BootReport { session, storage: storage.into() })
        .ok_or_else(|| format!("invalid boot report: {line}"))
}

struct FaultCounter {
    cut: Cell<Option<usize>>,
    step: Cell<usize>,
}

impl FaultCounter {
    fn reset(&self, cut: usize) {
        self.cut.set(Some(cut));
        self.step.set(0);
    }

    fn checkpoint(&self) -> Result<(), StoreError> {
        let step = self.step.get() + 1;
        self.step.set(step);
        (self.cut.get() != Some(step)).then_some(()).ok_or(StoreError::Interrupted)
    }
}

struct FileBackend {
    file: fs::File,
    sectors: usize,
    fault: Rc<FaultCounter>,
}

impl FileBackend {
    fn open(path: &Path, fault: Rc<FaultCounter>) -> Result<Self, String> {
        let file = fs::OpenOptions::new().read(true).write(true).open(path).map_err(io_error)?;
        Ok(Self { file, sectors: DISK_SECTORS, fault })
    }
}

impl SectorBackend for FileBackend {
    fn sectors(&self) -> usize {
        self.sectors
    }

    fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StoreError> {
        self.file
            .seek(SeekFrom::Start((sector * SECTOR_SIZE) as u64))
            .map_err(|_| StoreError::Io)?;
        self.file.read_exact(output).map_err(|_| StoreError::Io)
    }

    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), StoreError> {
        self.file
            .seek(SeekFrom::Start((sector * SECTOR_SIZE) as u64))
            .map_err(|_| StoreError::Io)?;
        self.file.write_all(input).map_err(|_| StoreError::Io)?;
        self.fault.checkpoint()
    }

    fn flush(&mut self) -> Result<(), StoreError> {
        self.file.sync_all().map_err(|_| StoreError::Io)?;
        self.fault.checkpoint()
    }
}

fn history_fixture(entry: &[u8]) -> [u8; HISTORY_BYTES] {
    let mut model = Model::new();
    let submission = Submission::from_bytes(entry).expect("fixture entry");
    assert!(model.restore_history(&[submission]));
    model.export_history()
}

fn prepare_baseline(path: &Path) -> Result<([u8; HISTORY_BYTES], [u8; HISTORY_BYTES]), String> {
    let old = history_fixture(b"old");
    let new = history_fixture(b"new");
    let file = fs::File::create(path).map_err(io_error)?;
    file.set_len((DISK_SECTORS * SECTOR_SIZE) as u64).map_err(io_error)?;
    drop(file);
    let fault = Rc::new(FaultCounter { cut: Cell::new(None), step: Cell::new(0) });
    let mut store = Store::format_with_backend(FileBackend::open(path, fault.clone())?)
        .map_err(|error| format!("format baseline: {error:?}"))?;
    store
        .replace(NamespaceId(TERMINAL_NAMESPACE.0), HISTORY_NAME, &history_fixture(b"older"))
        .map_err(|error| format!("write older: {error:?}"))?;
    store
        .replace(TERMINAL_NAMESPACE, HISTORY_NAME, &old)
        .map_err(|error| format!("write old: {error:?}"))?;
    Ok((old, new))
}

#[derive(Clone, Copy)]
enum FaultOperation {
    Replace,
    Compact,
}

fn make_interrupted_case(
    baseline: &Path,
    case: &Path,
    operation: FaultOperation,
    cut: usize,
    new: &[u8; HISTORY_BYTES],
) -> Result<bool, String> {
    fs::copy(baseline, case).map_err(io_error)?;
    let fault = Rc::new(FaultCounter { cut: Cell::new(None), step: Cell::new(0) });
    let mut store = Store::recover_backend(FileBackend::open(case, fault.clone())?)
        .map_err(|error| format!("recover case {cut}: {error:?}"))?;
    fault.reset(cut);
    let result = match operation {
        FaultOperation::Replace => store.replace(TERMINAL_NAMESPACE, HISTORY_NAME, new).map(|_| ()),
        FaultOperation::Compact => store.compact(None),
    };
    match result {
        Err(StoreError::Interrupted) => Ok(false),
        Ok(()) => Ok(true),
        Err(error) => Err(format!("fault case {cut}: {error:?}")),
    }
}

fn validate_host_case(path: &Path, expected: &[[u8; HISTORY_BYTES]]) -> Result<(), String> {
    let mut store = Store::recover_backend(FileBackend::open(
        path,
        Rc::new(FaultCounter { cut: Cell::new(None), step: Cell::new(0) }),
    )?)
    .map_err(|error| format!("recover generated case: {error:?}"))?;
    let mut output = [0; HISTORY_BYTES];
    let (_, length) = store
        .read(TERMINAL_NAMESPACE, HISTORY_NAME, logos_abi::VersionSelector::Current, &mut output)
        .map_err(|error| format!("read generated case: {error:?}"))?;
    (expected.contains(&output) && length == HISTORY_BYTES)
        .then_some(())
        .ok_or_else(|| "generated case exposed a mixed history value".into())
}

fn host_recovery(path: &Path) -> Result<String, String> {
    match Store::recover_backend(FileBackend::open(
        path,
        Rc::new(FaultCounter { cut: Cell::new(None), step: Cell::new(0) }),
    )?) {
        Ok(store) => Ok(match store.recovery() {
            logos_store::Recovery::Clean => "recovered".into(),
            logos_store::Recovery::Incomplete => "recovered-incomplete".into(),
            logos_store::Recovery::Corrupt => "corrupt".into(),
        }),
        Err(StoreError::Corrupt) => Ok("corrupt".into()),
        Err(error) => Err(format!("host recovery: {error:?}")),
    }
}

fn validate_terminal_history(path: &Path) -> Result<(), String> {
    let mut store = Store::recover_backend(FileBackend::open(
        path,
        Rc::new(FaultCounter { cut: Cell::new(None), step: Cell::new(0) }),
    )?)
    .map_err(|error| format!("recover terminal history: {error:?}"))?;
    let mut bytes = [0; HISTORY_BYTES];
    let (_, length) = store
        .read(TERMINAL_NAMESPACE, HISTORY_NAME, logos_abi::VersionSelector::Current, &mut bytes)
        .map_err(|error| format!("read terminal history: {error:?}"))?;
    let mut history = Model::new();
    (length == HISTORY_BYTES && history.restore_history_bytes(&bytes))
        .then_some(())
        .ok_or("terminal history was not persisted")?;
    (history.scrollback_len() == 2
        && history.history_entry(0).is_some_and(|entry| entry.as_bytes() == b"layout qwerty")
        && history.history_entry(1).is_some_and(|entry| entry.as_bytes() == b"layout azerty"))
    .then_some(())
    .ok_or_else(|| "terminal history commands differ after recovery".into())
}

fn superblocks(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(io_error)?;
    (bytes.len() >= SECTOR_SIZE * 2).then_some(()).ok_or("raw disk is too small")?;
    Ok((0..2)
        .map(|index| {
            let block = &bytes[index * SECTOR_SIZE..(index + 1) * SECTOR_SIZE];
            format!(
                "superblock{index}=magic:{:02x?} checksum:{:02x?}",
                &block[..4],
                &block[SECTOR_SIZE - 4..SECTOR_SIZE]
            )
        })
        .collect::<Vec<_>>()
        .join(" "))
}

struct Harness {
    child: Child,
    stream: TcpStream,
    debug_log: PathBuf,
    disk: PathBuf,
    qmp_port: u16,
    qmp_log: PathBuf,
    transcript: fs::File,
    report: BootReport,
    serial: String,
    deadline: Instant,
    timeout: Duration,
    network_peer: Option<NetworkPeer>,
    remote_port: u16,
    logosctl: PathBuf,
}

impl Harness {
    fn boot(
        qemu: &str,
        ovmf: &str,
        profile: &ImageProfile,
        fixture_dir: &Path,
        timeout: u64,
        expected_storage: &str,
    ) -> Result<Self, String> {
        Self::boot_inner(qemu, ovmf, profile, fixture_dir, timeout, expected_storage, None)
    }

    fn boot_with_peer(
        qemu: &str,
        ovmf: &str,
        profile: &ImageProfile,
        fixture_dir: &Path,
        timeout: u64,
        expected_storage: &str,
    ) -> Result<Self, String> {
        Self::boot_inner(
            qemu,
            ovmf,
            profile,
            fixture_dir,
            timeout,
            expected_storage,
            Some(NetworkPeer::start(&fixture_dir.join("peer-frames.log"))?),
        )
    }

    fn boot_inner(
        qemu: &str,
        ovmf: &str,
        profile: &ImageProfile,
        fixture_dir: &Path,
        timeout: u64,
        expected_storage: &str,
        network_peer: Option<NetworkPeer>,
    ) -> Result<Self, String> {
        let esp = fixture_dir.join("esp/EFI/BOOT");
        fs::create_dir_all(&esp).map_err(io_error)?;
        fs::copy(&profile.efi, esp.join("BOOTX64.EFI")).map_err(io_error)?;
        let payload_dir = fixture_dir.join("esp/EFI/LOGOS");
        fs::create_dir_all(&payload_dir).map_err(io_error)?;
        for (source, name) in [
            (&profile.terminal, "TERMINAL.EFI"),
            (&profile.sessions, "SESSIONS.EFI"),
            (&profile.storage, "STORAGE.EFI"),
            (&profile.network, "NETWORK.EFI"),
            (&profile.gateway, "GATEWAY.EFI"),
        ] {
            if let Some(source) = source {
                fs::copy(source, payload_dir.join(name)).map_err(io_error)?;
            }
        }
        if profile.incompatible_sessions {
            make_protocol_incompatible(&payload_dir.join("SESSIONS.EFI"))?;
        }
        let disk = fixture_dir.join("store.raw");
        if !disk.exists() {
            fs::File::create(&disk)
                .and_then(|file| file.set_len(16 * 1024 * 1024))
                .map_err(io_error)?;
        }

        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let port = listener.local_addr().map_err(io_error)?.port();
        let qmp_port = free_port()?;
        let remote_port = free_port()?;
        let boot_id = BOOT_ID.fetch_add(1, Ordering::Relaxed);
        let debug_log = fixture_dir.join(format!("debug-{boot_id}.log"));
        let qmp_log = fixture_dir.join(format!("qmp-{boot_id}.log"));
        let stderr_log = fixture_dir.join(format!("qemu-{boot_id}.stderr.log"));
        fs::File::create(&qmp_log).map_err(io_error)?;
        let stderr = fs::File::create(&stderr_log).map_err(io_error)?;
        let netdev = network_peer.as_ref().map_or_else(
            || format!("user,id=logos-net,net=10.0.2.0/24,dhcpstart=10.0.2.15,hostfwd=tcp:127.0.0.1:{remote_port}-:7443"),
            |peer| {
                format!(
                    "dgram,id=logos-net,local.type=inet,local.host=127.0.0.1,local.port={},remote.type=inet,remote.host=127.0.0.1,remote.port={}",
                    peer.qemu_port, peer.peer_port
                )
            },
        );
        let mut child = Command::new(qemu)
            .args([
                "-machine",
                "q35",
                "-m",
                "256M",
                "-drive",
                &format!("if=pflash,format=raw,readonly=on,file={ovmf}"),
                "-drive",
                &format!("format=raw,file=fat:rw:{}", fixture_dir.join("esp").display()),
                "-device",
                "virtio-balloon-pci,disable-modern=on,id=logos-virtio",
                "-netdev",
                &netdev,
                "-device",
                "virtio-net-pci,disable-modern=on,netdev=logos-net,id=logos-network,mac=52:54:00:12:34:56",
                "-drive",
                &format!(
                    "if=none,format=raw,cache=writeback,file={},id=logos-store",
                    disk.display()
                ),
                "-device",
                "virtio-blk-pci,disable-modern=on,drive=logos-store,id=logos-block",
                "-device",
                "isa-debug-exit,iobase=0xf4,iosize=0x04",
                "-display",
                "none",
                "-debugcon",
                &format!("file:{}", debug_log.display()),
                "-global",
                "isa-debugcon.iobase=0xe9",
                "-chardev",
                &format!("socket,id=test,host=127.0.0.1,port={port}"),
                "-serial",
                "null",
                "-serial",
                "chardev:test",
                "-qmp",
                &format!("tcp:127.0.0.1:{qmp_port},server=on,wait=off"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(io_error)?;
        let deadline = Instant::now() + Duration::from_secs(timeout);
        let stream = match accept_until(&listener, &mut child, deadline) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut harness = Self {
            child,
            stream,
            debug_log,
            disk,
            qmp_port,
            qmp_log,
            transcript: fs::File::create(fixture_dir.join(format!("control-{boot_id}.log")))
                .map_err(io_error)?,
            report: BootReport { session: 0, storage: String::new() },
            serial: String::new(),
            deadline,
            timeout: Duration::from_secs(timeout),
            network_peer,
            remote_port,
            logosctl: profile.logosctl.clone(),
        };
        let report = harness.wait_boot_report()?;
        let expected_storage =
            expected_storage.strip_prefix("LogOS: storage ").unwrap_or(expected_storage);
        let accepted = report.storage == expected_storage
            || expected_storage == "recovered" && report.storage == "recovered-incomplete";
        if !accepted {
            return Err(format!(
                "boot report storage mismatch: expected {expected_storage}, got {}",
                report.storage
            ));
        }
        harness.report = report;
        harness.wait("LOGOS/1 READY")?;
        harness.send("LOGOS/1 HELLO\n")?;
        harness.wait("LOGOS/1 RESULT hello=ok")?;
        Ok(harness)
    }

    fn reset(&mut self, scenario: &str) -> Result<(), String> {
        self.renew_deadline();
        self.send(&format!("LOGOS/1 RESET {scenario}\n"))?;
        self.wait("LOGOS/1 RESULT reset=accepted")
    }

    fn run(&mut self, scenario: Scenario) -> Result<(), String> {
        self.renew_deadline();
        if let Some(peer) = &self.network_peer {
            peer.set_scenario(scenario.id);
        }
        for command in scenario.setup {
            self.send(&format!("LOGOS/1 INPUT {command}\n"))?;
            self.wait("LOGOS/1 RESULT input=accepted")?;
        }
        if scenario.suite == "remote" {
            self.run_remote(scenario)?;
        }
        self.run_id(scenario.id)
    }

    fn run_remote(&mut self, scenario: Scenario) -> Result<(), String> {
        let client_secret = suites::remote::client_secret(scenario.runner);
        let enrolled_secret = [8; 32];
        let client_public =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(enrolled_secret));
        self.send(&format!("LOGOS/1 INPUT enroll {}\n", hex_bytes(client_public.as_bytes())))?;
        self.wait("LOGOS/1 RESULT input=accepted")?;
        self.wait_network_bound()?;
        self.wait("LogOS: Gateway started")?;
        let bootstrap = logos_remote::Bootstrap::from_root(&[9; 32], &[9; 32])
            .map_err(|error| format!("bootstrap: {error:?}"))?;
        let machine =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(bootstrap.device_key));
        let fixture = self.disk.parent().ok_or("fixture path")?;
        let key = fixture.join("logosctl.key");
        fs::write(&key, hex_bytes(&client_secret)).map_err(io_error)?;
        let descriptor = format!("{}:2", hex_bytes(machine.as_bytes()));
        let input = suites::remote::typed_input(scenario.runner);
        let mut child = Command::new(&self.logosctl)
            .args([
                "session",
                &format!("127.0.0.1:{}", self.remote_port),
                key.to_str().ok_or("key path")?,
                &descriptor,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io_error)?;
        child
            .stdin
            .as_mut()
            .ok_or("logosctl stdin")?
            .write_all(input.as_bytes())
            .map_err(io_error)?;
        while child.try_wait().map_err(io_error)?.is_none() {
            if Instant::now() >= self.deadline {
                let _ = child.kill();
                break;
            }
            self.send("LOGOS/1 ADVANCE 64\n")?;
            let _ = self.wait("LOGOS/1 RESULT advance=accepted");
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().map_err(io_error)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let passed = if suites::remote::auth_denied(scenario.runner) {
            !output.status.success()
        } else {
            output.status.success() && stdout.contains("pong")
        };
        passed.then_some(()).ok_or_else(|| {
            format!(
                "logosctl failed: stdout={} stderr={}",
                stdout,
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn run_id(&mut self, id: &str) -> Result<(), String> {
        if id.starts_with("network/") {
            self.wait_network_bound()?;
        }
        self.send(&format!("LOGOS/1 RUN {id}\n"))?;
        self.wait(&format!("LOGOS/1 RESULT scenario={id} status=passed"))
    }

    fn run_input(&mut self, id: &str) -> Result<(), String> {
        self.send(&format!("LOGOS/1 INPUT {id}\n"))?;
        self.wait("LOGOS/1 RESULT input=accepted")?;
        self.run_id(id)
    }

    fn shutdown(&mut self) -> Result<(), String> {
        self.renew_deadline();
        self.send("LOGOS/1 SHUTDOWN\n")?;
        wait_child(&mut self.child, self.deadline)?;
        wait_disk_available(&self.disk, self.deadline)
    }

    fn send(&mut self, value: &str) -> Result<(), String> {
        self.drain_serial()?;
        self.serial.clear();
        write!(self.transcript, "> {value}").map_err(io_error)?;
        self.stream.write_all(value.as_bytes()).map_err(io_error)
    }

    fn renew_deadline(&mut self) {
        self.deadline = Instant::now() + self.timeout;
    }

    fn wait(&mut self, expected: &str) -> Result<(), String> {
        while Instant::now() < self.deadline {
            self.drain_serial()?;
            if self.serial.lines().any(|line| line.starts_with(expected)) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(format!("timeout waiting for {expected}"))
    }

    fn drain_serial(&mut self) -> Result<(), String> {
        self.stream.set_nonblocking(true).map_err(io_error)?;
        let mut bytes = [0; 1024];
        loop {
            match self.stream.read(&mut bytes) {
                Ok(0) => break,
                Ok(length) => {
                    let text = String::from_utf8_lossy(&bytes[..length]);
                    write!(self.transcript, "< {text}").map_err(io_error)?;
                    self.serial.push_str(&text);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(())
    }

    fn wait_network_bound(&mut self) -> Result<(), String> {
        let expected = "LOGOS/1 NETWORK transport-dhcp status=bound ipv4=10.0.2.15 mask=255.255.255.0 router=10.0.2.2";
        while Instant::now() < self.deadline {
            if fs::read_to_string(&self.debug_log)
                .map_err(io_error)?
                .lines()
                .any(|line| line.starts_with(expected))
            {
                return Ok(());
            }
            self.send("LOGOS/1 ADVANCE 64\n")?;
            let _ = self.wait("LOGOS/1 RESULT advance=accepted");
        }
        Err(format!("timeout waiting for {expected}"))
    }

    fn wait_boot_report(&mut self) -> Result<BootReport, String> {
        let mut report = None;
        while Instant::now() < self.deadline {
            if let Ok(contents) = fs::read_to_string(&self.debug_log) {
                for line in contents
                    .split_inclusive('\n')
                    .filter(|line| line.ends_with('\n'))
                    .map(str::trim_end)
                    .filter(|line| line.starts_with("LOGOS/1 BOOT "))
                {
                    let parsed = parse_boot_report(line)?;
                    if report.replace(parsed).is_some() {
                        return Err("duplicate boot report/session ID".into());
                    }
                }
                if let Some(report) = report {
                    return (report.session == 1)
                        .then_some(report)
                        .ok_or_else(|| "mismatched boot/session ID".into());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("timeout waiting for structured boot report".into())
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = match args.as_slice() {
        [command] if command == "list" => {
            list();
            0
        }
        [command, id] if command == "run" => run_one(id),
        [command, suite] if command == "suite" => run_suite(suite),
        _ => {
            eprintln!("usage: logos-test list | run <scenario> | suite <name>");
            2
        }
    };
    std::process::exit(code);
}

fn list() {
    for item in SCENARIOS {
        println!(
            "{}\t{}\t{}",
            item.id,
            item.suite,
            if item.implemented { "ready" } else { "skipped" }
        );
    }
}

fn run_suite(name: &str) -> i32 {
    if !matches!(
        name,
        "pr" | "main"
            | "nightly"
            | "weekly"
            | "core"
            | "console"
            | "platform"
            | "persistence"
            | "network"
            | "remote"
    ) {
        eprintln!("unknown suite: {name}");
        return 2;
    }
    let selected: Vec<_> =
        SCENARIOS.iter().filter(|item| suite_contains(name, item.suite)).copied().collect();
    let root = repo_root();
    let run_dir = match artifact_dir(name) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("FAILED {name}: {error}");
            return 1;
        }
    };
    let seed = test_seed();
    let progress = Progress::new(selected.len());
    let profiles = match build_profiles(
        &root,
        &run_dir,
        selected.iter().any(|item| item.implemented && item.fixture == Fixture::Persistence),
    ) {
        Ok(profiles) => profiles,
        Err(error) => {
            let results = selected
                .iter()
                .filter(|item| item.implemented)
                .map(|item| failed(item, seed, &run_dir, &error))
                .collect::<Vec<_>>();
            for result in &results {
                progress.start(&result.id);
                progress.record(result);
            }
            for item in selected.iter().filter(|item| !item.implemented) {
                let result = ResultRecord {
                    id: item.id.into(),
                    status: Status::Skipped,
                    duration_ms: 0,
                    seed,
                    failure: Some("semantic proof unavailable".into()),
                    artifacts: run_dir.clone(),
                };
                progress.start(&result.id);
                progress.record(&result);
            }
            let _ = write_reports(&run_dir, &results);
            progress.finish();
            for result in &results {
                report(result);
            }
            return 1;
        }
    };
    let mut results: Vec<ResultRecord> = Vec::new();
    for suite in ["core", "console", "platform", "persistence", "network", "remote"] {
        let shared: Vec<_> = selected
            .iter()
            .filter(|item| {
                item.implemented && item.fixture == Fixture::Shared && item.suite == suite
            })
            .copied()
            .collect();
        if !shared.is_empty() {
            results.extend(run_fixture(
                &root,
                &run_dir,
                &profiles.standard,
                &format!("{suite}-shared"),
                &shared,
                seed,
                &progress,
            ));
        }
    }
    results.extend(run_independent(&root, &run_dir, &profiles, &selected, seed, &progress));
    for item in selected.iter().filter(|item| !item.implemented) {
        let result = ResultRecord {
            id: item.id.into(),
            status: Status::Skipped,
            duration_ms: 0,
            seed,
            failure: Some("semantic proof unavailable".into()),
            artifacts: run_dir.clone(),
        };
        progress.start(&result.id);
        progress.record(&result);
        results.push(result);
    }
    if let Err(error) = write_reports(&run_dir, &results) {
        eprintln!("FAILED report: {error}");
    }
    progress.finish();
    let mut failed = false;
    for result in &results {
        failed |= report(result) != 0;
    }
    let _ = fs::write(
        run_dir.join("profiles.txt"),
        format!(
            "standard={}\npersistence={}\n",
            profiles.standard.block_probe, profiles.persistence.block_probe
        ),
    );
    cleanup_bulk_artifacts(&run_dir);
    i32::from(failed)
}

fn run_one(id: &str) -> i32 {
    let Some(scenario) = SCENARIOS.iter().find(|item| item.id == id).copied() else {
        eprintln!("unknown scenario: {id}");
        return 2;
    };
    let run_dir = match artifact_dir("run") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("FAILED {id}: {error}");
            return 1;
        }
    };
    let seed = test_seed();
    let progress = Progress::new(1);
    if !scenario.implemented {
        let result = ResultRecord {
            id: id.into(),
            status: Status::Skipped,
            duration_ms: 0,
            seed,
            failure: Some("semantic proof unavailable".into()),
            artifacts: run_dir.clone(),
        };
        progress.start(&result.id);
        progress.record(&result);
        progress.finish();
        let _ = write_reports(&run_dir, std::slice::from_ref(&result));
        report(&result);
        return 0;
    }
    let root = repo_root();
    let profiles = match build_profiles(&root, &run_dir, scenario.fixture == Fixture::Persistence) {
        Ok(profiles) => profiles,
        Err(error) => {
            progress.finish();
            eprintln!("FAILED {id}: {error}");
            return 1;
        }
    };
    let profile = if scenario.suite == "remote" {
        if scenario.fixture == Fixture::Persistence {
            &profiles.remote_persistence
        } else {
            &profiles.remote
        }
    } else if scenario.fixture == Fixture::Persistence {
        &profiles.persistence
    } else {
        &profiles.standard
    };
    if suites::network::is_configuration(scenario.runner) {
        progress.start(scenario.id);
        let result = run_network_configuration(&run_dir, profile, scenario, seed);
        progress.record(&result);
        progress.finish();
        let _ = write_reports(&run_dir, std::slice::from_ref(&result));
        cleanup_bulk_artifacts(&run_dir);
        return report(&result);
    }
    let mut results = Vec::new();
    if suites::persistence::is_proof(scenario.runner) {
        results.push(run_persistence_proof(&root, &run_dir, profile, scenario, seed, &progress));
    } else {
        results.extend(run_fixture(
            &root,
            &run_dir,
            profile,
            id,
            std::slice::from_ref(&scenario),
            seed,
            &progress,
        ));
    }
    progress.finish();
    let _ = write_reports(&run_dir, &results);
    cleanup_bulk_artifacts(&run_dir);
    results.first().map_or(1, report)
}

fn run_network_configuration(
    run_dir: &Path,
    profile: &ImageProfile,
    scenario: Scenario,
    seed: u64,
) -> ResultRecord {
    let started = Instant::now();
    let fixture_dir = run_dir.join("fixtures").join("network-configuration");
    let result = (|| -> Result<(), String> {
        fs::create_dir_all(&fixture_dir).map_err(io_error)?;
        let (qemu, ovmf) = qemu_paths()?;
        let mut harness = Harness::boot_with_peer(
            &qemu,
            &ovmf,
            profile,
            &fixture_dir,
            scenario.timeout,
            "LogOS: storage formatted",
        )?;
        harness.wait_network_bound()?;
        harness.run_id(scenario.id)?;
        harness.shutdown()
    })();
    cleanup_fixture_artifacts(&fixture_dir, result.is_err());
    ResultRecord {
        id: scenario.id.into(),
        status: if result.is_ok() { Status::Passed } else { Status::Failed },
        duration_ms: started.elapsed().as_millis(),
        seed,
        failure: result.err(),
        artifacts: run_dir.to_path_buf(),
    }
}

fn run_persistence_proof(
    root: &Path,
    run_dir: &Path,
    profile: &ImageProfile,
    scenario: Scenario,
    seed: u64,
    progress: &Progress,
) -> ResultRecord {
    let started = Instant::now();
    progress.start(scenario.id);
    let result = match scenario.runner {
        Runner::PersistenceWriteInterruption => run_write_interruption(run_dir, profile, scenario),
        Runner::PersistenceRecovery => run_recovery(run_dir, profile, scenario),
        Runner::PersistenceCorruption => run_corruption(run_dir, profile, scenario),
        _ => {
            let mut results = Vec::new();
            run_persistence_fixture(root, run_dir, profile, scenario, 0, &mut results);
            results.pop().map_or_else(
                || Err("persistence runner returned no result".into()),
                |result| result.failure.map_or(Ok(()), Err),
            )
        }
    };
    let failure = result.err();
    cleanup_fixture_artifacts(
        &run_dir.join("fixtures").join(scenario.id.replace('/', "-")),
        failure.is_some(),
    );
    let result = ResultRecord {
        id: scenario.id.into(),
        status: if failure.is_none() { Status::Passed } else { Status::Failed },
        duration_ms: started.elapsed().as_millis(),
        seed,
        failure,
        artifacts: run_dir.to_path_buf(),
    };
    progress.record(&result);
    result
}

fn run_independent(
    root: &Path,
    run_dir: &Path,
    profiles: &Profiles,
    selected: &[Scenario],
    seed: u64,
    progress: &Progress,
) -> Vec<ResultRecord> {
    let tasks: Vec<_> = selected
        .iter()
        .enumerate()
        .filter(|(_, item)| item.implemented && item.fixture != Fixture::Shared)
        .map(|(index, item)| (index, *item))
        .collect();
    if tasks.len() < 2 {
        return tasks
            .into_iter()
            .flat_map(|(_, item)| {
                run_independent_one(root, run_dir, profiles, item, seed, progress)
            })
            .collect();
    }

    let jobs = env::var("LOGOS_TEST_JOBS").ok();
    let jobs = test_jobs(tasks.len(), jobs.as_deref());
    let next = AtomicUsize::new(0);
    let mut outputs = thread::scope(|scope| {
        let mut workers = (0..jobs)
            .map(|_| {
                scope.spawn(|| {
                    let mut output = Vec::new();
                    while let Some(task) = next
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |index| {
                            (index < tasks.len()).then_some(index + 1)
                        })
                        .ok()
                        .and_then(|index| tasks.get(index).copied())
                    {
                        output.push((
                            task.0,
                            run_independent_one(root, run_dir, profiles, task.1, seed, progress),
                        ));
                    }
                    output
                })
            })
            .collect::<Vec<_>>();
        workers
            .drain(..)
            .flat_map(|worker| worker.join().expect("test worker panicked"))
            .collect::<Vec<_>>()
    });
    outputs.sort_by_key(|(index, _)| *index);
    outputs.into_iter().flat_map(|(_, results)| results).collect()
}

fn test_jobs(count: usize, configured: Option<&str>) -> usize {
    configured
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|jobs| *jobs > 0)
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, |count| count.get().min(4)))
        .min(count)
}

fn run_independent_one(
    root: &Path,
    run_dir: &Path,
    profiles: &Profiles,
    item: Scenario,
    seed: u64,
    progress: &Progress,
) -> Vec<ResultRecord> {
    let profile = if item.suite == "remote" {
        if item.fixture == Fixture::Persistence {
            &profiles.remote_persistence
        } else {
            &profiles.remote
        }
    } else if item.fixture == Fixture::Persistence {
        &profiles.persistence
    } else {
        &profiles.standard
    };
    if item.fixture == Fixture::Persistence {
        vec![run_persistence_proof(root, run_dir, profile, item, seed, progress)]
    } else {
        run_fixture(root, run_dir, profile, item.id, std::slice::from_ref(&item), seed, progress)
    }
}

fn qemu_paths() -> Result<(String, String), String> {
    Ok((
        qemu_path().ok_or("qemu-system-x86_64 not found")?,
        ovmf_path().ok_or("OVMF_CODE not found")?,
    ))
}

fn boot_proof(
    fixture_dir: &Path,
    profile: &ImageProfile,
    scenario: &str,
    marker: &str,
) -> Result<(), String> {
    let expected = marker.strip_prefix("LogOS: storage ").unwrap_or(marker);
    let host = host_recovery(&fixture_dir.join("store.raw"))?;
    if host != expected && !(expected == "recovered" && host == "recovered-incomplete") {
        return Err(format!(
            "host {host}, expected guest {expected}; {}",
            superblocks(&fixture_dir.join("store.raw"))?
        ));
    }
    let (qemu, ovmf) = qemu_paths()?;
    let mut harness = Harness::boot(&qemu, &ovmf, profile, fixture_dir, 60, marker)?;
    harness.send(&format!("LOGOS/1 INPUT {scenario}\n"))?;
    harness.wait("LOGOS/1 RESULT input=accepted")?;
    let result = harness.run_id(scenario);
    let shutdown = harness.shutdown();
    result.and(shutdown)
}

fn clean_reboot(
    mut first: Harness,
    qemu: &str,
    ovmf: &str,
    profile: &ImageProfile,
    fixture_dir: &Path,
    timeout: u64,
) -> Result<Harness, String> {
    first.shutdown()?;
    let host = host_recovery(&first.disk)?;
    let second = Harness::boot(qemu, ovmf, profile, fixture_dir, timeout, &host)?;
    if second.report.storage != host {
        return Err(format!(
            "host {}, guest {}; {}",
            host,
            second.report.storage,
            superblocks(&first.disk)?
        ));
    }
    Ok(second)
}

fn run_write_interruption(
    run_dir: &Path,
    profile: &ImageProfile,
    scenario: Scenario,
) -> Result<(), String> {
    let fixture_dir = run_dir.join("fixtures").join("persistence-write-interruption");
    fs::create_dir_all(&fixture_dir).map_err(io_error)?;
    let baseline = fixture_dir.join("baseline.raw");
    let (old, new) = prepare_baseline(&baseline)?;
    for (label, operation, expected) in [
        ("replace", FaultOperation::Replace, vec![old, new]),
        ("compact", FaultOperation::Compact, vec![old]),
    ] {
        let mut cases = Vec::new();
        for cut in 1..64 {
            let case_dir = fixture_dir.join(format!("{label}-{cut:02}"));
            fs::create_dir_all(&case_dir).map_err(io_error)?;
            let disk = case_dir.join("store.raw");
            if make_interrupted_case(&baseline, &disk, operation, cut, &new)? {
                break;
            }
            validate_host_case(&disk, &expected)?;
            cases.push(case_dir);
        }
        let jobs = test_jobs(cases.len(), env::var("LOGOS_TEST_JOBS").ok().as_deref());
        let next = AtomicUsize::new(0);
        let failures = thread::scope(|scope| {
            let workers = (0..jobs)
                .map(|_| {
                    scope.spawn(|| {
                        let mut failures = Vec::new();
                        while let Ok(index) =
                            next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |index| {
                                (index < cases.len()).then_some(index + 1)
                            })
                        {
                            let case_dir = &cases[index];
                            let outcome = boot_proof(
                                case_dir,
                                profile,
                                scenario.id,
                                "LogOS: storage recovered",
                            );
                            if let Err(error) = outcome {
                                failures.push(error);
                            } else if env::var("LOGOS_TEST_ARTIFACTS").as_deref() != Ok("all") {
                                let _ = fs::remove_dir_all(case_dir);
                            }
                        }
                        failures
                    })
                })
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().expect("persistence worker panicked"))
                .collect::<Vec<_>>()
        });
        if let Some(error) = failures.into_iter().next() {
            return Err(format!("{label}: {error}"));
        }
    }
    Ok(())
}

fn run_recovery(run_dir: &Path, profile: &ImageProfile, scenario: Scenario) -> Result<(), String> {
    let fixture_dir = run_dir.join("fixtures").join("persistence-recovery");
    fs::create_dir_all(&fixture_dir).map_err(io_error)?;
    let baseline = fixture_dir.join("store.raw");
    let _ = prepare_baseline(&baseline)?;
    let (qemu, ovmf) = qemu_paths()?;
    let mut first = Harness::boot(
        &qemu,
        &ovmf,
        profile,
        &fixture_dir,
        scenario.timeout,
        "LogOS: storage recovered",
    )?;
    first.send(&format!("LOGOS/1 INPUT {}\n", scenario.id))?;
    first.wait("LOGOS/1 RESULT input=accepted")?;
    first.run_id(scenario.id)?;
    first.reset(scenario.id)?;
    first.run_input(scenario.id)?;
    let mut second = clean_reboot(first, &qemu, &ovmf, profile, &fixture_dir, scenario.timeout)?;
    second.send(&format!("LOGOS/1 INPUT {}\n", scenario.id))?;
    second.wait("LOGOS/1 RESULT input=accepted")?;
    second.run_id(scenario.id)?;
    second.shutdown()?;

    let incomplete_dir = fixture_dir.join("incomplete");
    fs::create_dir_all(&incomplete_dir).map_err(io_error)?;
    let incomplete_disk = incomplete_dir.join("store.raw");
    make_interrupted_case(
        &baseline,
        &incomplete_disk,
        FaultOperation::Replace,
        1,
        &history_fixture(b"new"),
    )?;
    let (qemu, ovmf) = qemu_paths()?;
    let mut harness = Harness::boot(
        &qemu,
        &ovmf,
        profile,
        &incomplete_dir,
        scenario.timeout,
        "LogOS: storage recovered",
    )?;
    harness.send("LOGOS/1 INPUT layout qwerty\n")?;
    harness.wait("LOGOS/1 RESULT input=accepted")?;
    harness.run_input(scenario.id)?;
    harness.shutdown()
}

fn run_corruption(
    run_dir: &Path,
    profile: &ImageProfile,
    scenario: Scenario,
) -> Result<(), String> {
    let fixture_dir = run_dir.join("fixtures").join("persistence-corruption-detected");
    fs::create_dir_all(&fixture_dir).map_err(io_error)?;
    let disk = fixture_dir.join("store.raw");
    let _ = prepare_baseline(&disk)?;
    let before = fs::read(&disk).map_err(io_error)?;
    let mut file = fs::OpenOptions::new().read(true).write(true).open(&disk).map_err(io_error)?;
    file.seek(SeekFrom::Start((3 * SECTOR_SIZE) as u64)).map_err(io_error)?;
    let mut byte = [0; 1];
    file.read_exact(&mut byte).map_err(io_error)?;
    file.seek(SeekFrom::Start((3 * SECTOR_SIZE) as u64)).map_err(io_error)?;
    file.write_all(&[byte[0] ^ 1]).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    boot_proof(&fixture_dir, profile, scenario.id, "LogOS: storage corrupt")?;
    let after = fs::read(&disk).map_err(io_error)?;
    (before[..3 * SECTOR_SIZE] == after[..3 * SECTOR_SIZE]
        && before[3 * SECTOR_SIZE + 1..] == after[3 * SECTOR_SIZE + 1..])
        .then_some(())
        .ok_or_else(|| "corrupt disk was rewritten".into())
}

fn run_persistence_fixture(
    root: &Path,
    run_dir: &Path,
    profile: &ImageProfile,
    scenario: Scenario,
    seed: u64,
    results: &mut Vec<ResultRecord>,
) {
    let fixture_dir = run_dir.join("fixtures").join(scenario.id.replace('/', "-"));
    let result = (|| -> Result<ResultRecord, String> {
        fs::create_dir_all(&fixture_dir).map_err(io_error)?;
        let qemu = qemu_path().ok_or("qemu-system-x86_64 not found")?;
        let ovmf = ovmf_path().ok_or("OVMF_CODE not found")?;
        let started = Instant::now();
        let mut first = Harness::boot(
            &qemu,
            &ovmf,
            profile,
            &fixture_dir,
            scenario.timeout,
            "LogOS: storage formatted",
        )?;
        let first_outcome = if matches!(scenario.runner, Runner::PersistenceTimeoutReset) {
            first.run_input(scenario.id)
        } else {
            first.run(scenario)
        };
        if matches!(scenario.runner, Runner::PersistenceTerminalHistory) {
            first.shutdown()?;
            validate_terminal_history(&fixture_dir.join("store.raw"))?;
            let host = host_recovery(&fixture_dir.join("store.raw"))?;
            let mut second =
                Harness::boot(&qemu, &ovmf, profile, &fixture_dir, scenario.timeout, &host)?;
            let second_outcome = second.run_input(scenario.id);
            let shutdown = second.shutdown();
            let failure =
                first_outcome.err().or_else(|| second_outcome.err()).or_else(|| shutdown.err());
            return Ok(ResultRecord {
                id: scenario.id.into(),
                status: if failure.is_none() { Status::Passed } else { Status::Failed },
                duration_ms: started.elapsed().as_millis(),
                seed,
                failure,
                artifacts: run_dir.to_path_buf(),
            });
        }
        if matches!(
            scenario.runner,
            Runner::PersistenceTimeoutReset | Runner::PersistenceCapabilityDenied
        ) {
            let shutdown = first.shutdown();
            let failure = first_outcome.err().or_else(|| shutdown.err());
            return Ok(ResultRecord {
                id: scenario.id.into(),
                status: if failure.is_none() { Status::Passed } else { Status::Failed },
                duration_ms: started.elapsed().as_millis(),
                seed,
                failure,
                artifacts: run_dir.to_path_buf(),
            });
        }
        let mut second =
            clean_reboot(first, &qemu, &ovmf, profile, &fixture_dir, scenario.timeout)?;
        let second_outcome = second.run_input(scenario.id);
        let shutdown = second.shutdown();
        let failure =
            first_outcome.err().or_else(|| second_outcome.err()).or_else(|| shutdown.err());
        Ok(ResultRecord {
            id: scenario.id.into(),
            status: if failure.is_none() { Status::Passed } else { Status::Failed },
            duration_ms: started.elapsed().as_millis(),
            seed,
            failure,
            artifacts: run_dir.to_path_buf(),
        })
    })();
    results.push(result.unwrap_or_else(|error| failed(&scenario, seed, run_dir, &error)));
    let _ = root;
}

fn run_fixture(
    root: &Path,
    run_dir: &Path,
    profile: &ImageProfile,
    name: &str,
    scenarios: &[Scenario],
    seed: u64,
    progress: &Progress,
) -> Vec<ResultRecord> {
    let mut results = Vec::new();
    let fixture = scenarios.first().map_or(Fixture::Fresh, |scenario| scenario.fixture);
    let profile = profile.for_fixture(fixture);
    let expected_storage = if matches!(fixture, Fixture::MissingStore | Fixture::MissingTerminal) {
        "unavailable"
    } else {
        "LogOS: storage formatted"
    };
    let fixture_dir = run_dir.join("fixtures").join(name.replace('/', "-"));
    if let Err(error) = fs::create_dir_all(&fixture_dir) {
        for scenario in scenarios {
            let result = failed(scenario, seed, run_dir, &error.to_string());
            progress.start(&result.id);
            progress.record(&result);
            results.push(result);
        }
        return results;
    }
    let timeout = scenarios.iter().map(|item| item.timeout).max().unwrap_or(20);
    let qemu = match qemu_path() {
        Some(path) => path,
        None => {
            for scenario in scenarios {
                let result = failed(scenario, seed, run_dir, "qemu-system-x86_64 not found");
                progress.start(&result.id);
                progress.record(&result);
                results.push(result);
            }
            return results;
        }
    };
    let ovmf = match ovmf_path() {
        Some(path) => path,
        None => {
            for scenario in scenarios {
                let result = failed(scenario, seed, run_dir, "OVMF_CODE not found");
                progress.start(&result.id);
                progress.record(&result);
                results.push(result);
            }
            return results;
        }
    };
    let network_peer = scenarios.iter().all(|scenario| scenario.suite == "network");
    let mut harness = match if network_peer {
        Harness::boot_with_peer(&qemu, &ovmf, &profile, &fixture_dir, timeout, expected_storage)
    } else {
        Harness::boot(&qemu, &ovmf, &profile, &fixture_dir, timeout, expected_storage)
    } {
        Ok(harness) => harness,
        Err(error) => {
            capture_failure(&fixture_dir, 0, &PathBuf::new());
            cleanup_fixture_artifacts(&fixture_dir, true);
            for scenario in scenarios {
                let result = failed(scenario, seed, run_dir, &error);
                progress.start(&result.id);
                progress.record(&result);
                results.push(result);
            }
            return results;
        }
    };
    let mut last_result: Option<usize> = None;
    for scenario in scenarios {
        if let Some(index) = last_result.take() {
            if let Err(error) = harness.reset(scenario.id) {
                results[index].status = Status::Failed;
                results[index].failure =
                    Some(format!("reset failed before next scenario: {error}"));
                progress.record(&results[index]);
                capture_failure(&fixture_dir, harness.qmp_port, &harness.qmp_log);
                let _ = harness.child.kill();
                let _ = harness.child.wait();
                let _ = fs::remove_dir_all(&fixture_dir);
                let _ = fs::create_dir_all(&fixture_dir);
                harness = match Harness::boot(
                    &qemu,
                    &ovmf,
                    &profile,
                    &fixture_dir,
                    timeout,
                    expected_storage,
                ) {
                    Ok(harness) => harness,
                    Err(error) => {
                        let result = failed(scenario, seed, run_dir, &error);
                        progress.start(&result.id);
                        progress.record(&result);
                        results.push(result);
                        continue;
                    }
                };
            } else {
                progress.record(&results[index]);
            }
        } else if matches!(
            scenario.fixture,
            Fixture::Persistence
                | Fixture::MissingSessions
                | Fixture::MissingTerminal
                | Fixture::IncompatibleSessions
                | Fixture::MissingStore
                | Fixture::MissingNetwork
        ) {
            // The persistence fixture boots into the proof directly.
        } else if let Err(error) = harness.reset(scenario.id) {
            let result = failed(scenario, seed, run_dir, &format!("initial reset failed: {error}"));
            progress.start(&result.id);
            progress.record(&result);
            results.push(result);
            capture_failure(&fixture_dir, harness.qmp_port, &harness.qmp_log);
            break;
        }
        let started = Instant::now();
        progress.start(scenario.id);
        let outcome = harness.run(*scenario);
        let result = ResultRecord {
            id: scenario.id.into(),
            status: if outcome.is_ok() { Status::Passed } else { Status::Failed },
            duration_ms: started.elapsed().as_millis(),
            seed,
            failure: outcome.err(),
            artifacts: run_dir.to_path_buf(),
        };
        results.push(result);
        last_result = Some(results.len() - 1);
    }
    if let Err(error) = harness.shutdown() {
        if let Some(index) = last_result {
            results[index].status = Status::Failed;
            results[index].failure = Some(error);
        }
        capture_failure(&fixture_dir, harness.qmp_port, &harness.qmp_log);
    }
    if let Some(index) = last_result {
        progress.record(&results[index]);
    }
    let failed =
        results.iter().any(|result| result.status == Status::Failed && result.artifacts == run_dir);
    cleanup_fixture_artifacts(&fixture_dir, failed);
    let _ = root;
    results
}

struct Profiles {
    standard: ImageProfile,
    persistence: ImageProfile,
    remote: ImageProfile,
    remote_persistence: ImageProfile,
}

fn build_profiles(root: &Path, run_dir: &Path, persistence: bool) -> Result<Profiles, String> {
    let profiles_dir = run_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).map_err(io_error)?;
    let standard = build_profile(root, &profiles_dir.join("standard"), false, false)?;
    let persistence_profile = if persistence {
        build_profile(root, &profiles_dir.join("persistence"), true, false)?
    } else {
        standard.clone()
    };
    let remote = build_profile(root, &profiles_dir.join("remote"), false, true)?;
    let remote_persistence = if persistence {
        build_profile(root, &profiles_dir.join("remote-persistence"), true, true)?
    } else {
        remote.clone()
    };
    Ok(Profiles { standard, persistence: persistence_profile, remote, remote_persistence })
}

fn build_profile(
    root: &Path,
    destination: &Path,
    block_probe: bool,
    usernet: bool,
) -> Result<ImageProfile, String> {
    fs::create_dir_all(destination).map_err(io_error)?;
    if !Command::new("cargo")
        .current_dir(root)
        .args(["build", "-p", "logosctl"])
        .status()
        .map_err(io_error)?
        .success()
    {
        return Err("logosctl build failed".into());
    }
    let features = if block_probe { "test-hooks,block-probe" } else { "test-hooks" };
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "logos-uefi",
            "--target",
            "x86_64-unknown-uefi",
            "--features",
            features,
        ])
        .status()
        .map_err(io_error)?;
    if !status.success() {
        return Err("kernel build failed".into());
    }
    for package in [
        "logos-terminal-service",
        "logos-sessions-service",
        "logos-network-service",
        "logos-gateway-service",
    ] {
        let mut command = Command::new("cargo");
        let package_features = if package == "logos-network-service" && usernet {
            "test-hooks,test-usernet"
        } else {
            "test-hooks"
        };
        command.current_dir(root).args([
            "build",
            "-p",
            package,
            "--target",
            "x86_64-unknown-uefi",
            "--features",
            package_features,
        ]);
        let status = command.status().map_err(io_error)?;
        if !status.success() {
            return Err(format!("{package} build failed"));
        }
    }
    let mut storage = Command::new("cargo");
    storage.current_dir(root).args([
        "build",
        "-p",
        "logos-storage-service",
        "--target",
        "x86_64-unknown-uefi",
        "--features",
        "test-hooks",
    ]);
    if !storage.status().map_err(io_error)?.success() {
        return Err("storage service build failed".into());
    }
    let target = root.join("target/x86_64-unknown-uefi/debug");
    let copy = |name: &str| -> Result<PathBuf, String> {
        let source = target.join(name);
        let destination = destination.join(name);
        fs::copy(&source, &destination).map_err(io_error)?;
        Ok(destination)
    };
    Ok(ImageProfile {
        efi: copy("logos-uefi.efi")?,
        terminal: Some(copy("logos-terminal-service.efi")?),
        sessions: Some(copy("logos-sessions-service.efi")?),
        storage: Some(copy("logos-storage-service.efi")?),
        network: Some(copy("logos-network-service.efi")?),
        gateway: Some(copy("logos-gateway-service.efi")?),
        incompatible_sessions: false,
        block_probe,
        logosctl: root.join("target/debug").join(format!("logosctl{}", env::consts::EXE_SUFFIX)),
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0xf) as usize] as char);
    }
    output
}

fn write_reports(run_dir: &Path, results: &[ResultRecord]) -> Result<(), String> {
    let mut json = String::from("[\n");
    for (index, result) in results.iter().enumerate() {
        let comma = if index + 1 == results.len() { "" } else { "," };
        json.push_str(&format!("  {{\"scenario\":\"{}\",\"status\":\"{}\",\"duration_ms\":{},\"seed\":{},\"failure\":\"{}\"}}{}\n", escape(&result.id), status_name(result.status), result.duration_ms, result.seed, escape(result.failure.as_deref().unwrap_or("")), comma));
    }
    json.push(']');
    fs::write(run_dir.join("results.json"), json).map_err(io_error)?;
    let mut junit = format!("<testsuite tests=\"{}\">", results.len());
    for result in results {
        let failure = result.failure.as_deref().unwrap_or("");
        let body = match result.status {
            Status::Passed => String::new(),
            Status::Skipped => format!("<skipped message=\"{}\"/>", xml(failure)),
            Status::Failed => format!("<failure message=\"{}\"/>", xml(failure)),
        };
        junit.push_str(&format!(
            "<testcase name=\"{}\" time=\"{:.3}\">{body}</testcase>",
            xml(&result.id),
            result.duration_ms as f64 / 1000.0
        ));
    }
    junit.push_str("</testsuite>\n");
    fs::write(run_dir.join("junit.xml"), junit).map_err(io_error)
}

fn failed(scenario: &Scenario, seed: u64, artifacts: &Path, failure: &str) -> ResultRecord {
    ResultRecord {
        id: scenario.id.into(),
        status: Status::Failed,
        duration_ms: 0,
        seed,
        failure: Some(failure.into()),
        artifacts: artifacts.to_path_buf(),
    }
}

fn report(result: &ResultRecord) -> i32 {
    println!(
        "{} {} ({} ms, seed {})",
        status_name(result.status).to_uppercase(),
        result.id,
        result.duration_ms,
        result.seed
    );
    i32::from(result.status == Status::Failed)
}

fn capture_failure(_fixture: &Path, port: u16, log: &Path) {
    if port != 0 && !log.as_os_str().is_empty() {
        capture_qmp(port, log, _fixture);
    }
}

fn artifact_dir(name: &str) -> Result<PathBuf, String> {
    let root = repo_root().join("target/logos-test");
    let base = root.join(format!("{}-{}", name.replace('/', "-"), test_seed()));
    let path =
        (0..)
            .map(|attempt| {
                if attempt == 0 {
                    base.clone()
                } else {
                    base.with_extension(format!("run{attempt}"))
                }
            })
            .find(|path| !path.exists())
            .ok_or_else(|| "unable to allocate a unique test artifact directory".to_string())?;
    fs::create_dir_all(&path).map_err(io_error)?;
    Ok(path)
}

fn suite_contains(requested: &str, actual: &str) -> bool {
    requested == actual
        || matches!(requested, "main" | "nightly" | "weekly")
        || requested == "pr" && matches!(actual, "core" | "console" | "platform" | "network")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}
fn test_seed() -> u64 {
    env::var("LOGOS_TEST_SEED").ok().and_then(|value| value.parse().ok()).unwrap_or_else(|| {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
    })
}
fn artifacts_all() -> bool {
    env::var("LOGOS_TEST_ARTIFACTS").is_ok_and(|value| value == "all")
}

fn cleanup_fixture_artifacts(path: &Path, failed: bool) {
    if artifacts_all() {
        return;
    }
    if failed {
        remove_raw_artifacts(path);
    } else {
        let _ = fs::remove_dir_all(path);
    }
}

fn remove_raw_artifacts(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_raw_artifacts(&path);
        } else if path.extension().is_some_and(|extension| extension == "raw") {
            let _ = fs::remove_file(path);
        }
    }
}

fn cleanup_bulk_artifacts(run_dir: &Path) {
    if !artifacts_all() {
        let _ = fs::remove_dir_all(run_dir.join("profiles"));
        let _ = fs::remove_file(run_dir.join("profiles.txt"));
    }
}
fn free_port() -> Result<u16, String> {
    Ok(TcpListener::bind("127.0.0.1:0").map_err(io_error)?.local_addr().map_err(io_error)?.port())
}

fn accept_until(
    listener: &TcpListener,
    child: &mut Child,
    deadline: Instant,
) -> Result<TcpStream, String> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(io_error(error)),
        }
        if child.try_wait().map_err(io_error)?.is_some() {
            return Err("QEMU exited before control connection".into());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err("timeout waiting for control connection".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_child(child: &mut Child, deadline: Instant) -> Result<(), String> {
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().map_err(io_error)? {
            return if status.code() == Some(1) {
                Ok(())
            } else {
                Err(format!("unexpected QEMU exit: {status}"))
            };
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    Err("timeout waiting for QEMU exit".into())
}

fn wait_disk_available(path: &Path, deadline: Instant) -> Result<(), String> {
    while Instant::now() < deadline {
        if fs::OpenOptions::new().read(true).write(true).open(path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("QEMU exited but Windows still holds raw disk {}", path.display()))
}

fn capture_qmp(port: u16, log: &Path, artifacts: &Path) {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else { return };
    let commands = format!(
        "{{\"execute\":\"qmp_capabilities\"}}\n{{\"execute\":\"query-status\"}}\n{{\"execute\":\"human-monitor-command\",\"arguments\":{{\"command-line\":\"info pci\"}}}}\n{{\"execute\":\"screendump\",\"arguments\":{{\"filename\":\"{}\"}}}}\n",
        artifacts.join("framebuffer.ppm").display().to_string().replace('\\', "\\\\")
    );
    let _ = stream.write_all(commands.as_bytes());
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut reader = BufReader::new(stream);
    let mut output = String::new();
    while reader.read_line(&mut output).is_ok_and(|read| read > 0) {}
    let _ = fs::write(log, output);
}

fn qemu_path() -> Option<String> {
    env::var("LOGOS_QEMU")
        .ok()
        .or_else(|| command_exists("qemu-system-x86_64").then(|| "qemu-system-x86_64".into()))
        .or_else(|| {
            Path::new("C:/Program Files/qemu/qemu-system-x86_64.exe")
                .is_file()
                .then(|| "C:/Program Files/qemu/qemu-system-x86_64.exe".into())
        })
}
fn ovmf_path() -> Option<String> {
    env::var("OVMF_CODE").ok().filter(|path| Path::new(path).is_file()).or_else(|| {
        [
            "C:/Program Files/qemu/share/edk2-x86_64-code.fd",
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/edk2/x64/OVMF_CODE.fd",
        ]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .map(str::to_string)
    })
}
fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
fn status_name(status: Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Skipped => "skipped",
    }
}
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}
fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}
fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

fn make_protocol_incompatible(path: &Path) -> Result<(), String> {
    let mut image = fs::read(path).map_err(io_error)?;
    let mut changed = false;
    for offset in 0..image.len().saturating_sub(28) {
        if image[offset..offset + 4] == *b"LGSV" && image[offset + 8..offset + 16] == *b"sessions" {
            image[offset + 24..offset + 26].copy_from_slice(&2u16.to_le_bytes());
            changed = true;
        }
    }
    changed.then_some(()).ok_or_else(|| "sessions service header not found".to_string())?;
    fs::write(path, image).map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_only_scenarios_are_not_implemented() {
        assert!(SCENARIOS.iter().filter(|item| item.setup.is_empty()).all(|item| {
            !item.implemented
                || matches!(
                    item.id,
                    "core/boot-normal"
                        | "persistence/block-read-flush"
                        | "persistence/block-timeout-reset"
                        | "persistence/capability-denied"
                        | "persistence/write-interruption"
                        | "persistence/recovery"
                        | "persistence/corruption-detected"
                        | "network/transport-dhcp"
                        | "network/device-bind"
                        | "network/configuration"
                        | "network/unauthorized-operation"
                        | "network/icmp-echo"
                        | "network/udp-round-trip"
                        | "network/backpressure-cancel"
                        | "network/packet-loss"
                        | "network/timeout"
                        | "network/reset-reconnect"
                        | "network/tcp-stream"
                        | "remote/enrollment-persistence"
                        | "remote/auth-denied"
                        | "remote/typed-invoke"
                        | "remote/reconnect-replay"
                        | "remote/pending-after-reset"
                        | "remote/gateway-restart"
                        | "remote/protected-state-corrupt"
                        | "platform/missing-sessions"
                        | "platform/missing-terminal"
                        | "platform/incompatible-sessions"
                        | "platform/missing-store"
                        | "platform/missing-network"
                )
        }));
    }

    #[test]
    fn pr_suite_is_bounded_to_implemented_milestones() {
        assert!(suite_contains("pr", "core"));
        assert!(suite_contains("pr", "network"));
    }

    #[test]
    fn report_escaping_is_valid() {
        assert_eq!(escape("a\n\"b"), "a\\n\\\"b");
        assert_eq!(xml("a&b"), "a&amp;b");
    }

    #[test]
    fn boot_reports_are_strict() {
        assert_eq!(
            parse_boot_report("LOGOS/1 BOOT session=1 storage=recovered").unwrap(),
            BootReport { session: 1, storage: "recovered".into() }
        );
        assert!(parse_boot_report("LOGOS/1 BOOT session=1 storage=unknown").is_err());
    }
}
