use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy)]
struct Scenario {
    id: &'static str,
    suite: &'static str,
    timeout: u64,
    implemented: bool,
}

const SCENARIOS: &[Scenario] = &[
    scenario("core/boot-normal", "core"),
    scenario("core/boot-recovery", "core"),
    scenario("core/ipc-request-reply", "core"),
    scenario("core/ipc-cancellation", "core"),
    scenario("core/task-block-wake", "core"),
    scenario("core/capability-denied", "core"),
    scenario("core/capability-revoked", "core"),
    scenario("core/driver-reset-recovery", "core"),
    scenario("core/resource-reclamation", "core"),
    scenario("core/panic-diagnostics", "core"),
    scenario("console/input-qwerty", "console"),
    scenario("console/input-azerty", "console"),
    scenario("console/editing-utf8", "console"),
    scenario("console/history", "console"),
    scenario("console/structured-command", "console"),
    scenario("console/cancellation", "console"),
    scenario("console/display-restart", "console"),
    scenario("console/input-service-restart", "console"),
    scenario("console/recovery-handoff", "console"),
    scenario("platform/manifest-valid", "platform"),
    scenario("platform/manifest-invalid", "platform"),
    scenario("platform/dependency-order", "platform"),
    scenario("platform/dependency-cycle-rejected", "platform"),
    scenario("platform/startup-failure", "platform"),
    scenario("platform/runtime-crash-restart", "platform"),
    scenario("platform/dependency-loss", "platform"),
    scenario("platform/restart-backoff", "platform"),
    scenario("platform/resource-reclamation", "platform"),
    scenario("platform/protocol-compatible", "platform"),
    scenario("platform/protocol-incompatible", "platform"),
    scenario("platform/unauthorized-capability", "platform"),
    scenario("platform/diagnostics", "platform"),
    scenario("platform/native-payload-staged", "platform"),
    scenario("platform/service-address-space", "platform"),
    scenario("platform/native-image-mapped", "platform"),
    scenario("platform/service-privilege-setup", "platform"),
    scenario("platform/service-ring3-transition", "platform"),
    scenario("platform/native-service-ready", "platform"),
    future("persistence/write-interruption", "persistence"),
    future("persistence/recovery", "persistence"),
    future("persistence/capability-denied", "persistence"),
    future("persistence/corruption-detected", "persistence"),
    future("network/packet-loss", "network"),
    future("network/timeout", "network"),
    future("network/reset-reconnect", "network"),
    future("network/unauthorized-operation", "network"),
];

const fn scenario(id: &'static str, suite: &'static str) -> Scenario {
    Scenario { id, suite, timeout: 20, implemented: true }
}
const fn future(id: &'static str, suite: &'static str) -> Scenario {
    Scenario { id, suite, timeout: 20, implemented: false }
}

struct ResultRecord {
    id: String,
    status: Status,
    duration_ms: u128,
    seed: u64,
    failure: Option<String>,
    artifacts: PathBuf,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = match args.as_slice() {
        [command] if command == "list" => {
            list();
            0
        }
        [command, id] if command == "run" => run_one(id).map_or_else(
            |error| {
                eprintln!("FAILED {id}: {error}");
                1
            },
            |result| report(&result),
        ),
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
    let accepted = matches!(
        name,
        "pr" | "main"
            | "nightly"
            | "weekly"
            | "core"
            | "console"
            | "platform"
            | "persistence"
            | "network"
    );
    if !accepted {
        eprintln!("unknown suite: {name}");
        return 2;
    }
    let selected: Vec<_> =
        SCENARIOS.iter().filter(|item| suite_contains(name, item.suite)).collect();
    let mut failed = false;
    for item in selected {
        match run_scenario(*item) {
            Ok(result) => failed |= report(&result) != 0,
            Err(error) => {
                eprintln!("FAILED {}: {error}", item.id);
                failed = true;
            }
        }
    }
    i32::from(failed)
}

fn suite_contains(requested: &str, actual: &str) -> bool {
    requested == actual
        || matches!(requested, "main" | "nightly" | "weekly")
        || requested == "pr" && matches!(actual, "core" | "console" | "platform")
}

fn run_one(id: &str) -> Result<ResultRecord, String> {
    let scenario = SCENARIOS
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "unknown scenario".to_string())?;
    run_scenario(*scenario)
}

fn run_scenario(scenario: Scenario) -> Result<ResultRecord, String> {
    let seed =
        env::var("LOGOS_TEST_SEED").ok().and_then(|value| value.parse().ok()).unwrap_or_else(seed);
    let artifacts = artifact_dir(scenario.id)?;
    if !scenario.implemented {
        let result = ResultRecord {
            id: scenario.id.into(),
            status: Status::Skipped,
            duration_ms: 0,
            seed,
            failure: Some("milestone unavailable".into()),
            artifacts,
        };
        write_reports(&result)?;
        return Ok(result);
    }
    let started = Instant::now();
    let outcome = launch(scenario, &artifacts);
    let (status, failure) = match outcome {
        Ok(()) => (Status::Passed, None),
        Err(error) => (Status::Failed, Some(error)),
    };
    let result = ResultRecord {
        id: scenario.id.into(),
        status,
        duration_ms: started.elapsed().as_millis(),
        seed,
        failure,
        artifacts,
    };
    write_reports(&result)?;
    Ok(result)
}

fn launch(scenario: Scenario, artifacts: &Path) -> Result<(), String> {
    let root = repo_root();
    build(&root)?;
    let efi = root.join("target/x86_64-unknown-uefi/debug/logos-uefi.efi");
    let esp = artifacts.join("esp/EFI/BOOT");
    fs::create_dir_all(&esp).map_err(io_error)?;
    fs::copy(&efi, esp.join("BOOTX64.EFI")).map_err(io_error)?;
    let payload = root.join("target/x86_64-unknown-uefi/debug/logos-terminal-service.efi");
    let payload_dir = artifacts.join("esp/EFI/LOGOS");
    fs::create_dir_all(&payload_dir).map_err(io_error)?;
    fs::copy(payload, payload_dir.join("TERMINAL.EFI")).map_err(io_error)?;
    fs::write(
        artifacts.join("image.hash"),
        format!("{:016x}\n", fnv(&fs::read(&efi).map_err(io_error)?)),
    )
    .map_err(io_error)?;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let port = listener.local_addr().map_err(io_error)?.port();
    let qmp_port = free_port()?;
    let qemu = qemu_path().ok_or("qemu-system-x86_64 not found")?;
    let ovmf = ovmf_path().ok_or("OVMF_CODE not found")?;
    let debug_log = artifacts.join("debug.log");
    let qmp_log = artifacts.join("qmp.log");
    let command_line = format!(
        "{qemu} -machine q35 -m 256M -drive if=pflash,format=raw,readonly=on,file={ovmf} ..."
    );
    fs::write(artifacts.join("command.txt"), &command_line).map_err(io_error)?;
    fs::write(artifacts.join("profile.txt"), "profile=debug\nfeatures=test-hooks\n")
        .map_err(io_error)?;

    let stderr = fs::File::create(artifacts.join("qemu.stderr.log")).map_err(io_error)?;
    let mut child = Command::new(&qemu)
        .args([
            "-machine",
            "q35",
            "-m",
            "256M",
            "-drive",
            &format!("if=pflash,format=raw,readonly=on,file={ovmf}"),
            "-drive",
            &format!("format=raw,file=fat:rw:{}", artifacts.join("esp").display()),
            "-device",
            "virtio-balloon-pci,disable-modern=on,id=logos-virtio",
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

    let deadline = Instant::now() + Duration::from_secs(scenario.timeout);
    capture_qmp(qmp_port, &qmp_log, artifacts);
    let result = (|| {
        let mut stream = accept_until(&listener, &mut child, deadline)?;
        let mut transcript_file =
            fs::File::create(artifacts.join("control.log")).map_err(io_error)?;
        wait_file(&debug_log, deadline, "LOGOS/1 READY")?;
        send(&mut stream, &mut transcript_file, "LOGOS/1 HELLO\n")?;
        wait_file(&debug_log, deadline, "LOGOS/1 RESULT hello=ok")?;
        if scenario.id == "platform/native-service-ready" {
            send(&mut stream, &mut transcript_file, "LOGOS/1 INPUT clear\n")?;
            wait_file(&debug_log, deadline, "LOGOS/1 RESULT input=accepted")?;
        }
        send(&mut stream, &mut transcript_file, &format!("LOGOS/1 RUN {}\n", scenario.id))?;
        wait_file(
            &debug_log,
            deadline,
            &format!("LOGOS/1 RESULT scenario={} status=passed", scenario.id),
        )?;
        wait_child(&mut child, deadline)
    })();
    if result.is_err() {
        capture_qmp(qmp_port, &qmp_log, artifacts);
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn build(root: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "--package",
            "logos-uefi",
            "--target",
            "x86_64-unknown-uefi",
            "--features",
            "test-hooks",
        ])
        .status()
        .map_err(io_error)?;
    if !status.success() {
        return Err("kernel build failed".into());
    }
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["build", "--package", "logos-terminal-service", "--target", "x86_64-unknown-uefi"])
        .status()
        .map_err(io_error)?;
    if status.success() { Ok(()) } else { Err("terminal service build failed".into()) }
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

fn wait_file(path: &Path, deadline: Instant, expected: &str) -> Result<(), String> {
    while Instant::now() < deadline {
        if fs::read_to_string(path)
            .is_ok_and(|contents| contents.lines().any(|line| line.starts_with(expected)))
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("timeout waiting for {expected}"))
}

fn send(stream: &mut TcpStream, log: &mut fs::File, value: &str) -> Result<(), String> {
    write!(log, "> {value}").map_err(io_error)?;
    stream.write_all(value.as_bytes()).map_err(io_error)
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

fn capture_qmp(port: u16, log: &Path, artifacts: &Path) {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else { return };
    let commands = format!(
        "{{\"execute\":\"qmp_capabilities\"}}\n{{\"execute\":\"query-status\"}}\n{{\"execute\":\"screendump\",\"arguments\":{{\"filename\":\"{}\"}}}}\n",
        artifacts.join("framebuffer.ppm").display().to_string().replace('\\', "\\\\")
    );
    let _ = stream.write_all(commands.as_bytes());
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut reader = BufReader::new(stream);
    let mut output = String::new();
    while reader.read_line(&mut output).is_ok_and(|read| read > 0) {}
    let _ = fs::write(log, output);
}

#[allow(dead_code)]
fn send_qmp_key(port: u16, key: &str) -> Result<(), String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(io_error)?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).map_err(io_error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(io_error)?);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(io_error)?;
    stream.write_all(b"{\"execute\":\"qmp_capabilities\"}\n").map_err(io_error)?;
    response.clear();
    reader.read_line(&mut response).map_err(io_error)?;
    if response.contains("error") {
        return Err(format!("QMP capabilities rejected: {response}"));
    }
    let command = format!(
        "{{\"execute\":\"input-send-event\",\"arguments\":{{\"events\":[{{\"type\":\"key\",\"data\":{{\"down\":true,\"key\":{{\"type\":\"qcode\",\"data\":\"{key}\"}}}}}},{{\"type\":\"key\",\"data\":{{\"down\":false,\"key\":{{\"type\":\"qcode\",\"data\":\"{key}\"}}}}}}]}}}}\n"
    );
    stream.write_all(command.as_bytes()).map_err(io_error)?;
    response.clear();
    reader.read_line(&mut response).map_err(io_error)?;
    (!response.contains("error"))
        .then_some(())
        .ok_or_else(|| format!("QMP key rejected: {response}"))
}

fn write_reports(result: &ResultRecord) -> Result<(), String> {
    let status = status_name(result.status);
    let failure = result.failure.as_deref().unwrap_or("");
    let json = format!(
        "{{\"scenario\":\"{}\",\"status\":\"{status}\",\"duration_ms\":{},\"seed\":{},\"failure\":\"{}\",\"artifacts\":\"{}\"}}\n",
        escape(&result.id),
        result.duration_ms,
        result.seed,
        escape(failure),
        escape(&result.artifacts.display().to_string())
    );
    fs::write(result.artifacts.join("result.json"), json).map_err(io_error)?;
    let body = match result.status {
        Status::Passed => String::new(),
        Status::Skipped => format!("<skipped message=\"{}\"/>", xml(failure)),
        Status::Failed => format!("<failure message=\"{}\"/>", xml(failure)),
    };
    fs::write(result.artifacts.join("junit.xml"), format!("<testsuite tests=\"1\"><testcase name=\"{}\" time=\"{:.3}\">{body}</testcase></testsuite>\n", xml(&result.id), result.duration_ms as f64 / 1000.0)).map_err(io_error)
}

fn report(result: &ResultRecord) -> i32 {
    println!(
        "{} {} ({} ms, seed {}, {})",
        status_name(result.status).to_uppercase(),
        result.id,
        result.duration_ms,
        result.seed,
        result.artifacts.display()
    );
    i32::from(result.status == Status::Failed)
}

fn artifact_dir(id: &str) -> Result<PathBuf, String> {
    let path =
        repo_root().join("target/logos-test").join(format!("{}-{}", id.replace('/', "-"), seed()));
    fs::create_dir_all(&path).map_err(io_error)?;
    Ok(path)
}
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}
fn seed() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
}
fn free_port() -> Result<u16, String> {
    Ok(TcpListener::bind("127.0.0.1:0").map_err(io_error)?.local_addr().map_err(io_error)?.port())
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
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn future_scenarios_never_pass() {
        assert!(
            SCENARIOS
                .iter()
                .filter(|item| matches!(item.suite, "persistence" | "network"))
                .all(|item| !item.implemented)
        );
    }
    #[test]
    fn pr_suite_is_bounded_to_implemented_milestones() {
        assert!(suite_contains("pr", "core"));
        assert!(!suite_contains("pr", "network"));
    }
    #[test]
    fn report_escaping_is_valid() {
        assert_eq!(escape("a\n\"b"), "a\\n\\\"b");
        assert_eq!(xml("a&b"), "a&amp;b");
    }
}
