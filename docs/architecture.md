# LogOS vNext Core architecture

The package has two targets and no allocator: the UEFI binary in `src/main.rs` calls `boot()` in
`src/lib.rs`. Core owns mechanisms only; Runtime and all services are deferred.

| Boundary | Owner | Invariant / proof |
| --- | --- | --- |
| UEFI handoff | `arch::boot` | discovers 1–8 healthy CPUs, stages fixed AP trampoline, exits boot services |
| Boot resources | `boot_resources` | copies bounded memory-map, GOP framebuffer, and PS/2 identities before UEFI handles are discarded |
| Physical frames | `frame_pool` | fixed frame addresses from copied conventional memory, capped at 65,536 frames with explicit reuse/exhaustion |
| Control plane | `syscall` + `logos-abi` | fixed scalar syscall requests, typed service-generation capabilities, and explicit status results |
| Per-CPU state | `arch::CpuLocal` via `GS_BASE` | private scheduler/idle stacks, TSS ring-transition fallback, cursor/current task, ticks, online state |
| Context boundary | `arch/context.rs` | timer, voluntary, ring-3 syscall dispatch, and user-fault entries save one GPR/RIP/RSP/RFLAGS/CS and x87/SSE frame shape |
| Publication | `Scheduler::save_context` + `finish` | outgoing task is claimable only after context-save publication |
| Scheduler | `Scheduler` | eight generation-safe slots, atomic lifecycle/wake word, published address-space root per task, CAS `Runnable → Running`, no task-body lock |
| Task primitives | `spawn`, `wake`, `yield_current`, `block_current`, `reclaim_completed` | explicit runnable/blocked/completed states and cheap wake-pending race handling |
| Timed wait | `sleep_current_for`, `wake_due` | one fixed deadline per slot; explicit wake cancels the deadline and BSP timer scans remain bounded |
| Runtime operations | `runtime::Runtime` | one in-process fixed command/response mailbox over two generation-safe operation slots with explicit ready/waiting/complete/cancelled/timed-out states |
| Service restart contract | `service_lifecycle::ServiceLifecycle` | fixed owner-held operation slots become explicitly `Restarted`; late completions are rejected and retries remain owner policy |
| Health service | `health::HealthService` | one in-process fixed command/response mailbox for `Ping`; restart rejects the old completion and caller explicitly retries |
| Terminal ABI | `logos-abi` | fixed semantic input, session stream, cell-diff render, endpoint identity, service identities, capabilities, and bounded control-plane shapes |
| IPC mechanics | `ipc::BoundedQueue` | fixed ring capacity, explicit full/empty outcomes, and doorbell edge notification; no allocation or serialization framework |
| Input semantics | `input::InputDecoder` | PS/2 Set-2 bytes become semantic key events and committed text with bounded modifiers and layout state |
| Terminal emulator | `terminal::Terminal` | bounded 160×100 cell state, 2,048-line scroll count, parser parameters, alternate screen, SGR, cursor/edit/erase controls, and dirty-cell output |
| Display state | `display::Display` | validates cell diffs, dimensions, positions, and endpoint generations; pixel/font ownership remains outside this model |
| Session shell | `session::Session` | bounded line editing, history, environment, four-stage pipelines, eight child slots, and volatile redirection files |
| Process admission | `process::ProcessTable` | fixed 16-slot process model, bounded ELF64 load plans, one generation-safe address-space identity with 16 validated mappings per process, typed capability authorization, and exit/fault/reclaim outcomes |
| Service supervisor | `supervisor::ServiceSupervisor` | five-service lifecycle model, heartbeat timeouts, endpoint epochs, restart limits, and recovery transition |
| Ring-3 proof domain | `user_mode` + `arch` | one fixed ELF admitted through `ProcessTable`, bound root/code/stack mappings, explicit scheduler CR3 selection, DPL-3 vector 49, and contained #UD/#GP/#PF |
| Terminal proof graph | `terminal_stack::TerminalStack` | deterministic Input → Terminal → Session → Terminal → Display path with generation-safe terminal restart |
| Fatal path | `arch::fatal` | one debug marker, interrupts disabled, every CPU halts |
| Runtime handoff | `handoff_to_runtime` | registers one root `TaskEntry`; the scheduler starts it through the normal context path |
| Proof workload | `qemu-proof` feature | assembly CPU-bound canaries, timer/switch counters, cross-CPU block/wake, structured PASS |

AP startup is deliberately narrow: xAPIC IDs, low-memory trampoline, current CR3, fixed stacks, and
sequential INIT/SIPI/SIPI. x2APIC IDs, malformed topology, more than eight CPUs, allocators, APIC
IPIs for wakeups, affinity, priorities, and AVX/XSAVE are not part of this milestone.

The handoff registers one root task. That task owns the first fixed Runtime operation table; Core does
not inspect, schedule, or orchestrate Runtime state. Runtime operations use the scheduler's sleep and
wake primitives but retain their own deadlines, terminal states, and slot generations. The terminal
contracts and service graph remain host-tested bounded models. The QEMU proof now adds one static
ring-3 domain, a vector-49 entry, and a contained user exception. Process admission now owns a
generation-safe address-space identity, while root binding, mapping, capabilities, syscall payloads,
and ELF image loading remain follow-on Core work. The terminal/display
integration still runs before AP startup, so the large terminal state is not placed on a 16 KiB task
stack.

`v1_docs/` is historical and is not an active architecture contract.
