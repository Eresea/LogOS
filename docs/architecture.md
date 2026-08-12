# LogOS vNext Core architecture

The package has two targets and no allocator: the UEFI binary in `src/main.rs` calls `boot()` in
`src/lib.rs`. Core owns mechanisms only; Runtime and all services are deferred.

| Boundary | Owner | Invariant / proof |
| --- | --- | --- |
| UEFI handoff | `arch::boot` | discovers 1–8 healthy CPUs, stages fixed AP trampoline, exits boot services |
| Boot resources | `boot_resources` | copies bounded memory-map, GOP framebuffer, and PS/2 identities before UEFI handles are discarded |
| Physical frames | `frame_pool` | fixed frame addresses from copied conventional memory, capped at 65,536 frames with explicit reuse/exhaustion |
| Control plane | `syscall` + `logos-abi` | fixed scalar syscall requests, typed service-generation capabilities, and explicit status results |
| ELF page admission | `loader` | maps validated segments and fixed user stacks to owned frames, then populates them through a page-local sink with bounded reclamation |
| User page tables | `page_table` | builds four-level user mappings with fixed root/intermediate-frame bounds, W^X/NX flags, conflict rejection, and grouped reclamation |
| Service address spaces | `service_runtime` | loads five retained ELFs into owned frames and retains one isolated root per service before scheduler admission |
| Service process admission | `service_runtime` + `process` | binds each service root, capabilities, coalesced mappings, and validated user launch metadata without entering service RIPs prematurely |
| User launch transition | `arch` + `scheduler` | selects the task root before restore and provides one fixed-selector `iretq` seam for future service entry |
| Service startup barrier | `service_startup` | enforces image → address space → process → launch-ready states and Input/Display → Terminal → Session → Commands dependencies |
| Service IPC pages | `service_ipc` + `page_table` | allocates five fixed generation-stamped endpoint pages, initializes their concrete ABI rings, and maps each only into its producer/consumer processes |
| Display device mapping | `service_runtime` + `process` | maps only the bounded retained GOP range into Display at `DISPLAY_FRAMEBUFFER_BASE` plus one read-only `FramebufferConfig` page at `DISPLAY_CONFIG_BASE`; no other service or kernel drawing path receives it |
| Keyboard byte mapping | `logos-abi` + `service_runtime` | allocates one zeroed fixed byte ring and maps it only into Input at `INPUT_KEYBOARD_RING_BASE`; PS/2 decoding remains outside the kernel |
| PS/2 interrupt adapter | `arch` | remaps the legacy PIC, unmasks only IRQ1 after the Input ring is published, and copies port `0x60` bytes into that ring; no key decoding occurs in Core |
| Font cache | `font` | fixed 8×16 scalar lookup, 1,024-entry cache, and deterministic replacement glyph |
| Per-CPU state | `arch::CpuLocal` via `GS_BASE` | private scheduler/idle stacks, TSS ring-transition fallback, cursor/current task, ticks, online state |
| Context boundary | `arch/context.rs` | timer, voluntary, ring-3 syscall dispatch, and user-fault entries save one GPR/RIP/RSP/RFLAGS/CS and x87/SSE frame shape |
| Publication | `Scheduler::save_context` + `finish` | outgoing task is claimable only after context-save publication |
| Scheduler | `Scheduler` | sixteen generation-safe slots, atomic lifecycle/wake word, published address-space root per task, CAS `Runnable → Running`, no task-body lock |
| Task primitives | `spawn`, `wake`, `yield_current`, `block_current`, `reclaim_completed` | explicit runnable/blocked/completed states and cheap wake-pending race handling |
| Timed wait | `sleep_current_for`, `wake_due` | one fixed deadline per slot; explicit wake cancels the deadline and BSP timer scans remain bounded |
| Runtime operations | `runtime::Runtime` | one in-process fixed command/response mailbox over two generation-safe operation slots with explicit ready/waiting/complete/cancelled/timed-out states |
| Service restart contract | `service_lifecycle::ServiceLifecycle` | fixed owner-held operation slots become explicitly `Restarted`; late completions are rejected and retries remain owner policy |
| Health service | `health::HealthService` | one in-process fixed command/response mailbox for `Ping`; restart rejects the old completion and caller explicitly retries |
| Terminal ABI | `logos-abi` | fixed semantic input, session stream, cell-diff render, endpoint identity, service identities, capabilities, and bounded control-plane shapes |
| IPC mechanics | `logos-abi::SharedIpc` + `ipc::BoundedQueue` | fixed SPSC rings are usable by kernel and services, fit bounded endpoint pages, and report full/empty/stale/disconnected outcomes without allocation or serialization |
| Input service | `services/images/src/input` + `logos-input::InputDecoder` | consumes the Input-only PS/2 byte mapping, produces semantic key/text messages on the Input→Terminal ring, and owns modifier/layout state |
| Terminal service | `services/images/src/terminal` + `logos-terminal::TerminalState` | ring-3 owns a bounded 80×25 live surface, consumes Input and Session rings, emits compact Session input and dirty-cell Display messages; the reusable host model remains bounded to 160×100 |
| Display service | `services/images/src/display` + `logos-display` | ring-3 validates cell diffs and endpoint generations, then rasterizes dirty cells through the fixed glyph cache into its mapped GOP framebuffer |
| Session service | `services/images/src/session` + `logos-session::SessionService` | ring-3 owns bounded line editing and shell state, emits prompt/output in backpressured 256-byte chunks, and uses a compact volatile-file budget; the host `Session` model retains the larger proof bounds |
| Commands service model | `logos-commands::CommandService` | bounded built-ins, output, status, and clear-screen command effects |
| Process admission | `process::ProcessTable` | fixed 16-slot process model, bounded ELF64 load plans, one generation-safe address-space identity with 16 validated mappings per process, typed capability authorization, and exit/fault/reclaim outcomes |
| User launch contract | `process::UserLaunch` + `Scheduler::spawn_user` | a running process with a bound root publishes entry RIP, aligned stack top, root, and process generation before its task becomes runnable |
| Service image manifest | `service_images::SERVICE_IMAGES` | five fixed ESP paths, process kinds, capability slots, and bounded ELF admission in dependency order |
| Retained service images | `service_loader::ServiceImageBundle` | five validated ELF records with page-aligned retained addresses, loaded before `ExitBootServices`, and no filesystem lifetime after UEFI exit |
| Service ELF packaging | `services/images` + `scripts/build-services.ps1` | five independent `x86_64-unknown-none` ELF artifacts, each bounded to 512 KiB and staged under the fixed ESP paths |
| Service image handoff | `arch::boot` + `service_loader::load_from_esp` | all five staged ELF images are loaded and validated before `ExitBootServices`; only bounded metadata survives the firmware boundary |
| Service supervisor | `supervisor::ServiceSupervisor` | five-service lifecycle model, heartbeat timeouts, endpoint epochs, restart limits, and recovery transition |
| Ring-3 proof domain | `user_mode` + `arch` | one fixed ELF admitted through `ProcessTable`, bound root/code/stack mappings, explicit scheduler CR3 selection, DPL-3 vector 49, and contained #UD/#GP/#PF |
| Terminal proof graph | `terminal_stack::TerminalStack` | deterministic Input → Terminal → Session → Terminal → Display path with generation-safe terminal restart |
| Fatal path | `arch::fatal` | one debug marker, interrupts disabled, every CPU halts |
| Runtime handoff | `handoff_to_runtime` | registers one root `TaskEntry`; the scheduler starts it through the normal context path |
| Proof workload | `qemu-proof` feature | assembly CPU-bound canaries, timer/switch counters, cross-CPU block/wake, structured PASS |

The process-to-scheduler handoff is now explicit: a running process with a bound root produces one
validated `UserLaunch`, and the scheduler publishes its entry, stack, root, and process generation
before marking the task runnable. Hardware page-table construction and ring-3 entry still consume
the existing follow-on boundary.

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
