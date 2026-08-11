# LogOS Agent Guide

LogOS vNext is a clean-slate `no_std` Rust OS. Keep the implementation small, explicit, and independently verifiable.

## Rules

- Make the smallest change that satisfies the task. Do not refactor or fix unrelated code.
- A previously passing test becoming failing is a **stop condition**: diagnose and rethink the implementation instead of fixing around the regression.
- Tests may remain failing only when explicitly identified as expected-to-fail for unfinished work. Never weaken an existing test to accommodate an implementation.
- If a small task starts requiring broad changes, new abstractions, compatibility paths, or substantial extra code, **stop and reconsider the design**.
- Prefer reverting a bad approach over adding code to compensate for it.
- Preserve `no_std`, bounded resources, strict subsystem ownership, and minimal dependencies.
- Core contains mechanisms, not service or application policy. Hardware exposes actions/events; Runtime coordinates; services own their state.
- Run the smallest relevant tests first. Run `cargo fmt --check` and `cargo clippy -- -D warnings` after Rust changes.
- Use QEMU tests only for behavior that genuinely requires boot or hardware integration.
- Do not bundle unrelated changes.

Plan before coding, inspect the final diff, verify it, and report unresolved issues rather than hiding them.
