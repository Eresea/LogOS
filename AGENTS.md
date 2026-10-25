# LogOS Agent Guide

LogOS v3 is a hosted Core (ADR-0096): the unchanged `no_std` service images run as OS processes on
Windows and Linux under `host/` (`logos-host`), with the fixed ABI windows as shared memory and
`int 49` as a pipe. The active milestone is the hosted service graph: directory, capabilities,
endpoints, IPC queues, event sets, heartbeats and restart served by `logos-host` so the services
discover each other and Atrium reaches Home in the native window.

## Rules

- Service and shared crates stay `no_std`, allocation-bounded, and buildable for
  `x86_64-unknown-none`. Host-only code lives in `host/` and in `common::host`; `common::trap` is
  the only target-specific line of the service facade.
- Keep changes small and independently runnable through `scripts/host.ps1`; new host dependencies
  go in `host/` only and need a documented milestone.
- Run `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and `cargo build --target x86_64-unknown-none -p logos-service-images --bins` after
  Rust changes. Run the smallest relevant host test first.
- `src/` is the frozen v2 kernel. Its policy modules (`runtime_ipc`, `runtime_events`,
  `runtime_services`, `service_manager`, `service_lifecycle`, `supervisor`) may be extracted for
  the hosted Core; its machine modules get no new features. Bare-metal history lives on `v2`.
- Document new subsystem boundaries in `docs/architecture.md`; add them only when a proof requires
  them.
- Use `docs/README.md` to select the smallest relevant documentation set; optional annexes are not
  default project context.
- Create and index an ADR for irreversible or cross-ring decisions. Routine reversible changes need
  no ADR.
- Keep historical evidence in reviewed documents or ADRs, not in active checklists.

## Ownership

`host/src/main.rs` owns the hosted Core: image spawn, shared windows, the syscall server, and the
native window. `host/shm` owns the fixed-address window table. `services/images/src/common.rs`
owns the service facade and the trap seam. Services, IPC, capabilities, allocation and the UI stack
keep the bounded vNext contracts documented in `docs/architecture.md`; new work must preserve those
ownership boundaries. `v1_docs/` and stale v1 evidence are historical reference only.

Plan before coding, verify the result, and report remaining issues. Do not bundle unrelated changes.
