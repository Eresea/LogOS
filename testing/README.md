# Testing

`cargo run -p logos-test -- list`, `run <scenario>`, and `suite <name>` are the canonical interface.

## Levels

Host tests prove bounded algorithms and state machines: validation, capability/generation rejection,
ownership/reclamation, timeout/cancellation, and fixed-capacity exhaustion. QEMU scenarios prove
assembled Core, Console, and Platform contracts: boot, target behavior, devices, shared pages,
scheduler composition, replacement/recovery, and fault containment. A changed isolation seam needs
both its focused host acceptance test and the applicable named QEMU proof; then run the broader
affected suite. Nightly and weekly suites repeat scenarios and fault matrices; unavailable
Persistence and Network proofs are explicit skips.

## Protocol

Test builds use bounded ASCII `LOGOS/1` frames over COM2. Requests are `HELLO`, `RUN`, `INJECT`, `INPUT`, `QUERY`, `ADVANCE`, `RESET`, and `SHUTDOWN`; responses are `READY`, `EVENT`, `RESULT`, and `ERROR`. Readiness is semantic: `QUERY network/configured` reports the authoritative NetworkRuntime cache. Human debugcon text is diagnostic only and is never a synchronization primitive.

## Proof rules

- Use `<module>/<behavior>` IDs and explicit readiness.
- Use bounded timeouts, deterministic seeds, semantic fault names, and bounded polling waits.
- Let a successful bounded `logosctl` operation prove Gateway listening; do not wait for Gateway
  debug text.
- Remote scenarios have one authority: the host operation, or a structured postcondition query;
  Core does not run a second label-only copy of the scenario.
- Test-driven Terminal input uses the same `RemoteRuntime::local_command` path as production input.
- Register the roadmap criterion and retain completed v1 proofs until its contract is deprecated.
- Future contracts must skip, never pass or expected-fail.

Artifacts live under `target/logos-test/<run-id>` with command, profile, image hash, debug/control/QMP logs, seed, JSON, JUnit, and failure diagnostics.
Successful fixtures are removed by default; failed fixtures keep diagnostics without `.raw` disks.
Set `LOGOS_TEST_ARTIFACTS=all` to retain every fixture file for forensic reruns.
