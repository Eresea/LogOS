# LogOS Roadmap

> **Updated:** 2026-08-02
>
> **Current milestone:** [Network v1](NETWORK.md)
>
> **Target:** A remotely operable, capability-based Rust OS with replaceable native services and sandboxed WASM applications.

This file tracks sequence and status only. Milestone scope and evidence live in linked documents so they are loaded only when needed.

## Direction

- Keep hardware, memory, scheduling, IPC, and capability enforcement in Core.
- Put policy and durable state in replaceable services.
- Use typed, capability-scoped contracts for local users, remote clients, applications, and agents.
- End each milestone with permanent automated QEMU proofs.

See [Architecture](architecture.md), [security constraints](security.md), and [boot constraints](boot-sequence.md).

## Milestones

| Order | Milestone | Status | Exit proof |
| ---: | --- | --- | --- |
| 1 | [Core v1](CORE.md) | Complete | Privileged mechanisms boot and recover under QEMU |
| 2 | [Console v1](CONSOLE.md) | Complete | Normal terminal and independent recovery console work |
| 3 | [Platform v1](PLATFORM.md) | Complete | Isolated native services negotiate, fail, and restart independently |
| 4 | [Persistence v1](PERSISTENCE.md) | Complete | Capability-scoped state survives interrupted writes and resets |
| 5 | [Network v1](NETWORK.md) | **Current** | Capability-controlled ICMP and UDP survive denial and device reset |
| 6 | [Remote v1](REMOTE.md) | Planned | The machine is operable without local input or display |
| 7 | [Update v1](UPDATE.md) | Planned | Signed updates activate atomically and roll back after failure |
| 8 | [Applications v1](APPLICATIONS.md) | Planned | Sandboxed WASM applications install, run, communicate, and persist |
| 9 | [Experience v1](EXPERIENCE.md) | Planned | A replaceable graphical environment uses existing contracts |

Detailed scope is lazy-loaded from each milestone document. Shared principles, completion rules, migration history, and roadmap maintenance live in [Milestone policy](MILESTONE-POLICY.md). Architectural intent remains in [Architecture](architecture.md).

Core hardening continues when an outward contract or measurement requires it; see [Core](CORE.md). Working implementation notes belong in [TODO](TODO.md), not here.

## Milestone rules

Each milestone defines ownership, capabilities, bounded resources, cancellation, versioning, recovery, diagnostics, explicit non-goals, and QEMU proofs. Completed proof IDs remain regression contracts until their public contract is deprecated; list them with `cargo run -p logos-test -- list`.

Cross-ring or irreversible decisions require an [ADR](adr/README.md). Subsystem names follow the [naming register](NAMING.md).
