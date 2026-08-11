# LogOS vNext Core architecture

The package has two targets and no allocator: the UEFI binary in `src/main.rs` calls `boot()` in
`src/lib.rs`. Core owns mechanisms only; Runtime and all services are deferred.

| Boundary | Owner | Invariant / proof |
| --- | --- | --- |
| UEFI handoff | `arch::boot` | discovers 1–8 healthy CPUs, stages fixed AP trampoline, exits boot services |
| Per-CPU state | `arch::CpuLocal` via `GS_BASE` | private scheduler/idle stacks, cursor/current task, ticks, online state |
| Context boundary | `context_common` | timer and voluntary entries save the same GPR/RIP/RSP/RFLAGS/CS and x87/SSE frame, then switch to scheduler stack |
| Publication | `Scheduler::save_context` + `finish` | outgoing task is claimable only after context-save publication |
| Scheduler | `Scheduler` | eight generation-safe slots, atomic lifecycle/wake word, CAS `Runnable → Running`, no task-body lock |
| Task primitives | `spawn`, `wake`, `yield_current`, `block_current`, `reclaim_completed` | explicit runnable/blocked/completed states and cheap wake-pending race handling |
| Fatal path | `arch::fatal` | one debug marker, interrupts disabled, every CPU halts |
| Proof workload | `qemu-proof` feature | assembly CPU-bound canaries, timer/switch counters, cross-CPU block/wake, structured PASS |

AP startup is deliberately narrow: xAPIC IDs, low-memory trampoline, current CR3, fixed stacks, and
sequential INIT/SIPI/SIPI. x2APIC IDs, malformed topology, more than eight CPUs, allocators, APIC
IPIs for wakeups, affinity, priorities, AVX/XSAVE, user mode, Runtime, services, and IPC are not
part of this milestone.

`v1_docs/` is historical and is not an active architecture contract.
