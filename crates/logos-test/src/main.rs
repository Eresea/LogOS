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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixture {
    Shared,
    Fresh,
    Persistence,
}

#[derive(Clone, Copy)]
struct Scenario {
    id: &'static str,
    suite: &'static str,
    timeout: u64,
    implemented: bool,
    setup: &'static [&'static str],
    fixture: Fixture,
}

const SCENARIOS: &[Scenario] = &[
    configured("core/boot-normal", "core", &[], Fixture::Fresh),
    scenario("core/boot-recovery", "core", Fixture::Fresh),
    scenario("core/ipc-request-reply", "core", Fixture::Fresh),
    scenario("core/ipc-cancellation", "core", Fixture::Fresh),
    scenario("core/task-block-wake", "core", Fixture::Fresh),
    scenario("core/capability-denied", "core", Fixture::Fresh),
    scenario("core/capability-revoked", "core", Fixture::Fresh),
    scenario("core/driver-reset-recovery", "core", Fixture::Fresh),
    scenario("core/resource-reclamation", "core", Fixture::Fresh),
    scenario("core/panic-diagnostics", "core", Fixture::Fresh),
    scenario("console/input-qwerty", "console", Fixture::Shared),
    scenario("console/input-azerty", "console", Fixture::Shared),
    scenario("console/editing-utf8", "console", Fixture::Shared),
    scenario("console/history", "console", Fixture::Shared),
    configured(
        "console/structured-command",
        "console",
        &["assert-tasks", "assert-sessions"],
        Fixture::Shared,
    ),
    configured("console/capability-denied", "console", &["deny-recovery"], Fixture::Shared),
    configured("console/input-capability-denied", "console", &["deny-layout"], Fixture::Shared),
    configured("console/display-capability-denied", "console", &["deny-display"], Fixture::Shared),
    configured("console/session-capability-denied", "console", &["deny-session"], Fixture::Shared),
    configured("console/cancellation", "console", &["assert-cancel"], Fixture::Shared),
    scenario("console/display-restart", "console", Fixture::Shared),
    configured("console/input-service-restart", "console", &["assert-restart"], Fixture::Shared),
    configured(
        "console/terminal-service-restart",
        "console",
        &["assert-terminal-service-restart"],
        Fixture::Shared,
    ),
    configured(
        "console/sessions-service-restart",
        "console",
        &["assert-sessions-service-restart"],
        Fixture::Shared,
    ),
    configured(
        "persistence/storage-service-restart",
        "persistence",
        &["assert-storage-service-restart"],
        Fixture::Shared,
    ),
    configured("persistence/block-read-flush", "persistence", &[], Fixture::Persistence),
    configured(
        "persistence/terminal-history",
        "persistence",
        &["layout azerty", "layout qwerty", "persistence/terminal-history"],
        Fixture::Persistence,
    ),
    configured(
        "persistence/capability-denied",
        "persistence",
        &["persistence/capability-denied"],
        Fixture::Persistence,
    ),
    scenario("console/recovery-handoff", "console", Fixture::Fresh),
    scenario("platform/manifest-valid", "platform", Fixture::Fresh),
    scenario("platform/manifest-invalid", "platform", Fixture::Fresh),
    scenario("platform/dependency-order", "platform", Fixture::Shared),
    scenario("platform/dependency-cycle-rejected", "platform", Fixture::Shared),
    scenario("platform/startup-failure", "platform", Fixture::Fresh),
    configured(
        "platform/runtime-crash-restart",
        "platform",
        &["assert-crash-restart"],
        Fixture::Shared,
    ),
    scenario("platform/dependency-loss", "platform", Fixture::Shared),
    configured(
        "platform/restart-backoff",
        "platform",
        &["assert-restart-backoff"],
        Fixture::Shared,
    ),
    scenario("platform/resource-reclamation", "platform", Fixture::Shared),
    scenario("platform/protocol-compatible", "platform", Fixture::Shared),
    scenario("platform/protocol-incompatible", "platform", Fixture::Shared),
    scenario("platform/unauthorized-capability", "platform", Fixture::Shared),
    scenario("platform/diagnostics", "platform", Fixture::Shared),
    scenario("platform/native-payload-staged", "platform", Fixture::Fresh),
    scenario("platform/service-address-space", "platform", Fixture::Fresh),
    scenario("platform/native-image-mapped", "platform", Fixture::Fresh),
    scenario("platform/service-privilege-setup", "platform", Fixture::Fresh),
    scenario("platform/service-ring3-transition", "platform", Fixture::Fresh),
    configured(
        "platform/native-service-ready",
        "platform",
        &[
            "health",
            "assert-ping",
            "tasks",
            "assert-services",
            "assert-drivers",
            "trace",
            "assert-inspect",
            "restart virtio-balloon",
            "cancel virtio-balloon",
            "layout azerty",
            "layout qwerty",
            "echo hello",
            "help clear",
            "commands",
            "clear",
        ],
        Fixture::Shared,
    ),
    future("persistence/write-interruption", "persistence", Fixture::Fresh),
    future("persistence/recovery", "persistence", Fixture::Fresh),
    future("persistence/capability-denied", "persistence", Fixture::Fresh),
    future("persistence/corruption-detected", "persistence", Fixture::Fresh),
    future("network/packet-loss", "network", Fixture::Fresh),
    future("network/timeout", "network", Fixture::Fresh),
    future("network/reset-reconnect", "network", Fixture::Fresh),
    future("network/unauthorized-operation", "network", Fixture::Fresh),
];

const fn scenario(id: &'static str, suite: &'static str, fixture: Fixture) -> Scenario {
    future(id, suite, fixture)
}

const fn configured(
    id: &'static str,
    suite: &'static str,
    setup: &'static [&'static str],
    fixture: Fixture,
) -> Scenario {
    Scenario { id, suite, timeout: 20, implemented: true, setup, fixture }
}

const fn future(id: &'static str, suite: &'static str, fixture: Fixture) -> Scenario {
    Scenario { id, suite, timeout: 20, implemented: false, setup: &[], fixture }
}

struct ResultRecord {
    id: String,
    status: Status,
    duration_ms: u128,
    seed: u64,
    failure: Option<String>,
    artifacts: PathBuf,
}

#[derive(Clone)]
struct ImageProfile {
    efi: PathBuf,
    terminal: PathBuf,
    sessions: PathBuf,
    storage: PathBuf,
    block_probe: bool,
}

struct Harness {
    child: Child,
    stream: TcpStream,
    debug_log: PathBuf,
    qmp_port: u16,
    qmp_log: PathBuf,
    transcript: fs::File,
    offset: usize,
    deadline: Instant,
}

impl Harness {
    fn boot(
        qemu: &str,
        ovmf: &str,
        profile: &ImageProfile,
        fixture_dir: &Path,
        timeout: u64,
        startup_marker: &str,
    ) -> Result<Self, String> {
        let esp = fixture_dir.join("esp/EFI/BOOT");
        fs::create_dir_all(&esp).map_err(io_error)?;
        fs::copy(&profile.efi, esp.join("BOOTX64.EFI")).map_err(io_error)?;
        let payload_dir = fixture_dir.join("esp/EFI/LOGOS");
        fs::create_dir_all(&payload_dir).map_err(io_error)?;
        fs::copy(&profile.terminal, payload_dir.join("TERMINAL.EFI")).map_err(io_error)?;
        fs::copy(&profile.sessions, payload_dir.join("SESSIONS.EFI")).map_err(io_error)?;
        fs::copy(&profile.storage, payload_dir.join("STORAGE.EFI")).map_err(io_error)?;
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
        let debug_log = fixture_dir.join("debug.log");
        let qmp_log = fixture_dir.join("qmp.log");
        let stderr_log = fixture_dir.join("qemu.stderr.log");
        let stderr = fs::File::create(&stderr_log).map_err(io_error)?;
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
            qmp_port,
            qmp_log,
            transcript: fs::File::create(fixture_dir.join("control.log")).map_err(io_error)?,
            offset: 0,
            deadline,
        };
        harness.wait(startup_marker)?;
        harness.offset = 0;
        harness.wait("LOGOS/1 READY")?;
        harness.send("LOGOS/1 HELLO\n")?;
        harness.wait("LOGOS/1 RESULT hello=ok")?;
        Ok(harness)
    }

    fn reset(&mut self, scenario: &str) -> Result<(), String> {
        self.send(&format!("LOGOS/1 RESET {scenario}\n"))?;
        self.wait("LOGOS/1 RESULT reset=accepted")
    }

    fn run(&mut self, scenario: Scenario) -> Result<(), String> {
        for command in scenario.setup {
            self.send(&format!("LOGOS/1 INPUT {command}\n"))?;
            self.wait("LOGOS/1 RESULT input=accepted")?;
        }
        self.send(&format!("LOGOS/1 RUN {}\n", scenario.id))?;
        self.wait(&format!("LOGOS/1 RESULT scenario={} status=passed", scenario.id))
    }

    fn shutdown(&mut self) -> Result<(), String> {
        self.send("LOGOS/1 SHUTDOWN\n")?;
        wait_child(&mut self.child, self.deadline)
    }

    fn send(&mut self, value: &str) -> Result<(), String> {
        write!(self.transcript, "> {value}").map_err(io_error)?;
        self.stream.write_all(value.as_bytes()).map_err(io_error)
    }

    fn wait(&mut self, expected: &str) -> Result<(), String> {
        wait_file(&self.debug_log, &mut self.offset, self.deadline, expected)
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
            let _ = write_reports(&run_dir, &results);
            for result in &results {
                report(result);
            }
            return 1;
        }
    };
    let mut results: Vec<ResultRecord> = Vec::new();
    let shared: Vec<_> = selected
        .iter()
        .filter(|item| item.implemented && item.fixture == Fixture::Shared)
        .copied()
        .collect();
    if !shared.is_empty() {
        run_fixture(&root, &run_dir, &profiles.standard, "shared", &shared, seed, &mut results);
    }
    for item in selected.iter().filter(|item| item.implemented && item.fixture != Fixture::Shared) {
        let profile = if item.fixture == Fixture::Persistence {
            &profiles.persistence
        } else {
            &profiles.standard
        };
        run_fixture(
            &root,
            &run_dir,
            profile,
            item.id,
            std::slice::from_ref(item),
            seed,
            &mut results,
        );
    }
    for item in selected.iter().filter(|item| !item.implemented) {
        results.push(ResultRecord {
            id: item.id.into(),
            status: Status::Skipped,
            duration_ms: 0,
            seed,
            failure: Some("semantic proof unavailable".into()),
            artifacts: run_dir.clone(),
        });
    }
    if let Err(error) = write_reports(&run_dir, &results) {
        eprintln!("FAILED report: {error}");
    }
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
    if !scenario.implemented {
        let result = ResultRecord {
            id: id.into(),
            status: Status::Skipped,
            duration_ms: 0,
            seed,
            failure: Some("semantic proof unavailable".into()),
            artifacts: run_dir.clone(),
        };
        let _ = write_reports(&run_dir, std::slice::from_ref(&result));
        report(&result);
        return 0;
    }
    let root = repo_root();
    let profiles = match build_profiles(&root, &run_dir, scenario.fixture == Fixture::Persistence) {
        Ok(profiles) => profiles,
        Err(error) => {
            eprintln!("FAILED {id}: {error}");
            return 1;
        }
    };
    let profile = if scenario.fixture == Fixture::Persistence {
        &profiles.persistence
    } else {
        &profiles.standard
    };
    let mut results = Vec::new();
    if scenario.fixture == Fixture::Persistence {
        run_persistence_fixture(&root, &run_dir, profile, scenario, seed, &mut results);
    } else {
        run_fixture(
            &root,
            &run_dir,
            profile,
            id,
            std::slice::from_ref(&scenario),
            seed,
            &mut results,
        );
    }
    let _ = write_reports(&run_dir, &results);
    cleanup_bulk_artifacts(&run_dir);
    results.first().map_or(1, report)
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
        let first_outcome = first.run(scenario);
        first.shutdown()?;
        let mut second = Harness::boot(
            &qemu,
            &ovmf,
            profile,
            &fixture_dir,
            scenario.timeout,
            "LogOS: storage recovered",
        )?;
        let second_outcome = second.run(scenario);
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
    results: &mut Vec<ResultRecord>,
) {
    let fixture_dir = run_dir.join("fixtures").join(name.replace('/', "-"));
    if let Err(error) = fs::create_dir_all(&fixture_dir) {
        for scenario in scenarios {
            results.push(failed(scenario, seed, run_dir, &error.to_string()));
        }
        return;
    }
    let timeout = scenarios.iter().map(|item| item.timeout).max().unwrap_or(20);
    let qemu = match qemu_path() {
        Some(path) => path,
        None => {
            for scenario in scenarios {
                results.push(failed(scenario, seed, run_dir, "qemu-system-x86_64 not found"));
            }
            return;
        }
    };
    let ovmf = match ovmf_path() {
        Some(path) => path,
        None => {
            for scenario in scenarios {
                results.push(failed(scenario, seed, run_dir, "OVMF_CODE not found"));
            }
            return;
        }
    };
    let mut harness = match Harness::boot(
        &qemu,
        &ovmf,
        profile,
        &fixture_dir,
        timeout,
        "LogOS: storage formatted",
    ) {
        Ok(harness) => harness,
        Err(error) => {
            capture_failure(&fixture_dir, 0, &PathBuf::new());
            for scenario in scenarios {
                results.push(failed(scenario, seed, run_dir, &error));
            }
            return;
        }
    };
    let mut last_result: Option<usize> = None;
    for scenario in scenarios {
        if let Some(index) = last_result.take() {
            if let Err(error) = harness.reset(scenario.id) {
                results[index].status = Status::Failed;
                results[index].failure =
                    Some(format!("reset failed before next scenario: {error}"));
                capture_failure(&fixture_dir, harness.qmp_port, &harness.qmp_log);
                let _ = harness.child.kill();
                let _ = harness.child.wait();
                let _ = fs::remove_dir_all(&fixture_dir);
                let _ = fs::create_dir_all(&fixture_dir);
                harness = match Harness::boot(
                    &qemu,
                    &ovmf,
                    profile,
                    &fixture_dir,
                    timeout,
                    "LogOS: storage formatted",
                ) {
                    Ok(harness) => harness,
                    Err(error) => {
                        results.push(failed(scenario, seed, run_dir, &error));
                        continue;
                    }
                };
            }
        } else if scenario.fixture == Fixture::Persistence {
            // The persistence fixture boots into the proof directly.
        } else if let Err(error) = harness.reset(scenario.id) {
            results.push(failed(
                scenario,
                seed,
                run_dir,
                &format!("initial reset failed: {error}"),
            ));
            capture_failure(&fixture_dir, harness.qmp_port, &harness.qmp_log);
            break;
        }
        let started = Instant::now();
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
    let keep = artifacts_all()
        || results
            .iter()
            .any(|result| result.status == Status::Failed && result.artifacts == run_dir);
    if !keep {
        let _ = fs::remove_dir_all(&fixture_dir);
    }
    let _ = root;
}

struct Profiles {
    standard: ImageProfile,
    persistence: ImageProfile,
}

fn build_profiles(root: &Path, run_dir: &Path, persistence: bool) -> Result<Profiles, String> {
    let profiles_dir = run_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).map_err(io_error)?;
    let standard = build_profile(root, &profiles_dir.join("standard"), false)?;
    let persistence_profile = if persistence {
        build_profile(root, &profiles_dir.join("persistence"), true)?
    } else {
        standard.clone()
    };
    Ok(Profiles { standard, persistence: persistence_profile })
}

fn build_profile(
    root: &Path,
    destination: &Path,
    block_probe: bool,
) -> Result<ImageProfile, String> {
    fs::create_dir_all(destination).map_err(io_error)?;
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
    for package in ["logos-terminal-service", "logos-sessions-service"] {
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["build", "-p", package, "--target", "x86_64-unknown-uefi"])
            .status()
            .map_err(io_error)?;
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
        terminal: copy("logos-terminal-service.efi")?,
        sessions: copy("logos-sessions-service.efi")?,
        storage: copy("logos-storage-service.efi")?,
        block_probe,
    })
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
    let path = repo_root().join("target/logos-test").join(format!(
        "{}-{}",
        name.replace('/', "-"),
        test_seed()
    ));
    fs::create_dir_all(&path).map_err(io_error)?;
    Ok(path)
}

fn suite_contains(requested: &str, actual: &str) -> bool {
    requested == actual
        || matches!(requested, "main" | "nightly" | "weekly")
        || requested == "pr" && matches!(actual, "core" | "console" | "platform")
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

fn wait_file(
    path: &Path,
    offset: &mut usize,
    deadline: Instant,
    expected: &str,
) -> Result<(), String> {
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            let start = (*offset).min(contents.len());
            if contents[start..].lines().any(|line| line.starts_with(expected)) {
                *offset = contents.len();
                return Ok(());
            }
            *offset = contents.len();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("timeout waiting for {expected}"))
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
                        | "persistence/capability-denied"
                )
        }));
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
