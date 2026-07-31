# Testing

`cargo run -p logos-test -- list`, `run <scenario>`, and `suite <name>` are the canonical interface.

## Levels

Host tests prove bounded algorithms and state machines. QEMU scenarios prove assembled Core, Console, and Platform contracts. Nightly and weekly suites repeat scenarios and fault matrices; unavailable Persistence and Network proofs are explicit skips.

## Protocol

Test builds use bounded ASCII `LOGOS/1` frames over COM2. Requests are `HELLO`, `RUN`, `INJECT`, `INPUT`, `QUERY`, `ADVANCE`, `RESET`, and `SHUTDOWN`; responses are `READY`, `EVENT`, `RESULT`, and `ERROR`. Human debugcon text is not asserted.

## Proof rules

- Use `<module>/<behavior>` IDs and explicit readiness.
- Use bounded timeouts, deterministic seeds, semantic fault names, and bounded polling waits.
- Register the roadmap criterion and retain completed v1 proofs until its contract is deprecated.
- Future contracts must skip, never pass or expected-fail.

Artifacts live under `target/logos-test/<run-id>` with command, profile, image hash, debug/control/QMP logs, seed, JSON, JUnit, and failure diagnostics.
