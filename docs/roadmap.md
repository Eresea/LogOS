# LogOS Roadmap

> **Updated:** 2026-08-06
>
> **Current milestone:** ABI v4 stabilization and migration closeout
>
> **Target:** A remotely operable, capability-based Rust OS with replaceable native services and sandboxed WASM applications.

This file tracks system capabilities and their required module slices. Module scope and evidence live in linked documents.

## Direction

- Keep hardware, memory, scheduling, IPC, and capability enforcement in Core.
- Put policy and durable state in replaceable services.
- Use typed, capability-scoped contracts for local users, remote clients, applications, and agents.
- End each milestone with permanent automated QEMU proofs.

See [Architecture](architecture.md), [security constraints](security.md), and [boot constraints](boot-sequence.md).

## Post-ABI-v4 execution sequence

The project spends one bounded cycle per step, then stops restructuring ABI v4:

1. Stabilize and repair the completed typed layers, using the main suite as the gate.
2. Complete one bounded Network-client migration. **Complete:** typed Terminal/Gateway transport,
   one global transaction, rollback, and replacement cleanup.
3. Complete one bounded Remote migration.
4. Freeze ABI v4 structure and resume capability development—first Remote verification if the
   QEMU environment is available, otherwise Safe System Artifacts / Persistence v2.

No new ABI-wide abstraction or parallel transport is introduced after step 3 without a new ADR
and an explicit compatibility milestone.

## Capability roadmap

| Order | System capability | Required module slices | Status / exit proof |
| ---: | --- | --- | --- |
| 1 | Bootable core | [Core v1](CORE.md) | Complete: privileged mechanisms boot and recover |
| 2 | Local typed operation | [Console v1](CONSOLE.md), [Sessions v1](SESSIONS.md) | Complete: normal terminal and recovery console are independent |
| 3 | Replaceable services | [Platform v1](PLATFORM.md) | Complete: services negotiate, fail, and restart independently |
| 4 | Durable bounded state | [Persistence v1](PERSISTENCE.md) | Complete: scoped state survives interrupted writes and resets |
| 5 | Bounded packet connectivity | [Network v1](NETWORK.md) | Typed transport implementation complete; four QEMU Network-client proofs remain open at the service/device scheduling boundary |
| 6 | Remote foundation | [Remote Foundation v1](REMOTE.md#foundation-v1): Network v2 transport slice, [Platform v2](PLATFORM.md) trust slice, [Persistence v2](PERSISTENCE.md) protected-state slice, [Sessions v2](SESSIONS.md) attachment slice | Network client transport implementation complete; Network operational closure and the Remote/Gateway proof boundary remain |
| 7 | Remote administration | [Remote v1](REMOTE.md) | Headless authenticated administration works through existing contracts |
| 8 | Safe system artifacts | Complete Persistence v2 | Large signed artifacts and durable configuration are safe to stage |
| 9 | Atomic system evolution | [Update v1](UPDATE.md) | A signed system bundle activates or rolls back atomically |
| 10 | Sandboxed execution | Required [Core v2](CORE.md) resource slice, [Applications v1](APPLICATIONS.md) | A bounded WASM component runs and persists without joining kernel correctness |
| 11 | Local graphical operation | [Experience v1](EXPERIENCE.md) | A replaceable compositor and graphical terminal use published contracts |
| 12 | Workspaces and objects | Persistence v3, Applications v2 | Portable application workspaces survive upgrade and restore |
| 13 | Operational maturity | Remote v2, Update v2, Platform v3 | Components migrate, hand off, and update independently |
| 14 | AI-native operation and rich UX | Applications v3, Sessions v3, Experience v2 | Typed agent and human clients share audited operations |
| 15 | Hardware scale | Core v3, Network v3, Experience v3 | Added hardware is justified by a target and permanent proof |

Each row is an integration target, not a lockstep release train. A module slice may land and be proven independently before the capability that consumes it. Module versions change only when their published contract or guaranteed behavior changes; implementation-only changes retain the existing version. See [ADR-0016](adr/0016-capability-slices.md).

Detailed scope lives in each module document. Shared principles, completion rules, migration history, and roadmap maintenance live in [Milestone policy](MILESTONE-POLICY.md). Architectural intent remains in [Architecture](architecture.md).

Core hardening continues when an outward contract or measurement requires it; see [Core](CORE.md). Working implementation notes belong in [TODO](TODO.md), not here.

## Milestone rules

Each milestone defines ownership, capabilities, bounded resources, cancellation, versioning, recovery, diagnostics, explicit non-goals, and QEMU proofs. Completed proof IDs remain regression contracts until their public contract is deprecated; list them with `cargo run -p logos-test -- list`.

Cross-ring or irreversible decisions require an [ADR](adr/README.md). Subsystem names follow the [naming register](NAMING.md).
