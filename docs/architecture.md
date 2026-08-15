# LogOS vNext Core architecture

The package has two targets and no allocator: the UEFI binary in `src/main.rs` calls `boot()` in
`src/lib.rs`. Core owns mechanisms only; Runtime and the six fixed services execute through
fixed service boundaries.

| Boundary | Owner | Invariant / proof |
| --- | --- | --- |
| UEFI handoff | `arch::boot` | discovers 1–8 healthy CPUs, stages fixed AP trampoline, exits boot services |
| Boot resources | `boot_resources` | copies bounded memory-map, GOP framebuffer, and PS/2 identities before UEFI handles are discarded |
| Physical frames | `frame_pool` + `memory` | copied UEFI descriptors normalize into sorted disjoint runs; indexed bitmap words, generation-safe leases, bounded batches, reservations, zeroed/dirty state, per-CPU caches, sharded pools, and remote frees stay capped at 65,536 frames |
| Memory subsystem contracts | `memory` | fixed async wait nodes, cancellation/deadlines, address-space generations, 4 KiB VM map operations, batched TLB queues, page-table caches, slab/page heap handles, pressure/reclaim callbacks, ownership quotas, and atomic observability are present before architecture-specific expansion |
| Control plane | `process` + `user_mode` | admission-time fixed service mappings plus bounded Wait/Notify, IpcSend, and IpcReceive syscalls with process-bound capability checks |
| ELF page admission | `loader` | maps validated segments and fixed user stacks to owned frames, then populates them through a page-local sink with bounded reclamation; Storage receives a fixed 128-page stack for journal replay and transaction shadow state |
| User page tables | `page_table` | builds four-level user mappings with fixed root/intermediate-frame bounds, W^X/NX flags, conflict rejection, and grouped reclamation |
| Service address spaces | `service_runtime` | loads six retained ELFs into owned frames and retains one isolated root per service before scheduler admission |
| Service process admission | `service_runtime` + `process` | binds each service root, coalesced mappings, and validated user launch metadata without entering service RIPs prematurely |
| User launch transition | `arch` + `scheduler` | selects the task root before restore and provides the fixed-selector `iretq` path for service entry |
| Service startup barrier | `service_startup` | enforces image → address space → process → launch-ready states and Input/Display → Terminal → Session → Commands → Storage dependencies |
| Service IPC boundary | `service_ipc` + `service_runtime` | keeps the six existing terminal queues and adds dedicated process-bound StorageToCore/CoreToStorage capabilities through private staging pages; no queue, MMIO, or DMA frame is mapped into a service root |
| Storage boundary | Core VirtIO block adapter + `logos-storage` format + storage IPC/object service | Core owns PCI discovery, feature negotiation, fixed DMA arena, queues, MSI-X interrupt delivery, reset, timeouts, and flush; Storage owns fixed request lifecycles, superblocks, journal, replay, recovery, durability, object IDs, namespace resolution, and bounded file operations; Commands reaches Storage through versioned `CommandsToStorage`/`StorageToCommands` messages over private staging pages; one active transaction uses fixed shadow state and at most `MAX_RECORDS_PER_TRANSACTION` records; Core stores no paths or namespace state; split-ring generations reset the bounded queue before descriptor reuse |
| Display device mapping | `service_runtime` + `process` | maps only the bounded retained GOP range into Display at `DISPLAY_FRAMEBUFFER_BASE` plus one read-only `FramebufferConfig` page at `DISPLAY_CONFIG_BASE`; boot rejects modes below the fixed 80×25/8×16 profile; no other service or kernel drawing path receives it |
| Keyboard byte mapping | `logos-abi` + `service_runtime` | allocates one zeroed fixed byte ring with an observable drop counter and maps it only into Input at `INPUT_KEYBOARD_RING_BASE`; PS/2 decoding remains outside the kernel |
| PS/2 interrupt adapter | `arch` | remaps the legacy PIC, unmasks only IRQ1 after the Input ring is published, and copies port `0x60` bytes into that ring; no key decoding occurs in Core |
| Font rendering | `logos-display` | fixed 8×16 anti-aliased JetBrains Mono coverage atlas with deterministic replacement glyph |
| Per-CPU state | `arch::CpuLocal` via `GS_BASE` | private scheduler/idle stacks, TSS ring-transition fallback, cursor/current task, ticks, online state |
| Context boundary | `arch/context.rs` | timer, voluntary, reschedule IPI, Wait/Notify syscall dispatch, and user-fault entries save one GPR/RIP/RSP/RFLAGS/CS and x87/SSE frame shape |
| Publication | `Scheduler::save_context` + `finish` | outgoing task is claimable only after context-save publication |
| Scheduler | `Scheduler` | sixteen generation-safe slots, atomic lifecycle/wake word, published address-space root per task, CAS `Runnable → Running` on any online CPU, no task-body lock |
| Task primitives | `spawn`, `wake`, `yield_current`, `block_current`, `reclaim_completed` | explicit runnable/blocked/completed states, cheap wake-pending race handling, and bounded reschedule IPIs for hardware wakeups |
| Timed wait | `sleep_current_for`, `wake_due` | one fixed deadline per slot; explicit wake cancels the deadline and BSP timer scans remain bounded |
| Runtime operations | `runtime::Runtime` | one in-process fixed command/response mailbox over two generation-safe operation slots with explicit ready/waiting/complete/cancelled/timed-out states |
| Service restart contract | `service_lifecycle::ServiceLifecycle` | fixed owner-held operation slots become explicitly `Restarted`; late completions are rejected and retries remain owner policy |
| Health service | `health::HealthService` | one in-process fixed command/response mailbox for `Ping`; restart rejects the old completion and caller explicitly retries |
| Terminal ABI | `logos-abi` | fixed semantic input, session stream, cell-diff render, endpoint identity, and service identities |
| IPC mechanics | `service_ipc` + `logos-abi::SharedIpc` + `scheduler::Scheduler` | kernel-owned fixed SPSC rings preserve capacities 32/1/8, exact typed copies use private staging pages, and empty/full edges signal bounded event masks |
| Input service | `services/images/src/input` + `logos-input::InputDecoder` | consumes the Input-only PS/2 byte mapping, produces semantic key/text messages on the Input→Terminal ring, and owns modifier/layout state |
| Terminal service | `services/images/src/terminal` + `logos-terminal::TerminalState` | ring-3 owns a bounded fixed 80×25 live surface, consumes Input and Session rings, and emits compact Session input and dirty-cell Display messages |
| Display service | `services/images/src/display` + `logos-display` | ring-3 validates cell diffs and endpoint generations, then rasterizes dirty cells through embedded glyphs into its mapped GOP framebuffer |
| Session service | `services/images/src/session` + `logos-session::SessionService` | ring-3 owns bounded line editing and forwards completed commands/results; command execution remains in Commands and no state survives restart or reboot |
| Commands service | `services/images/src/commands` + `logos-commands::CommandService` | receives bounded Session requests, executes built-ins, and performs `ls`, `touch`, `cat`, `write`, `rm`, and `mv` through Storage transactions; returns backpressured output over its reverse IPC rings |
| Process admission | `process::ProcessTable` | fixed 16-slot process model, bounded ELF64 load plans, one generation-safe address-space identity with 16 validated mappings per process, and exit/fault/reclaim outcomes |
| User launch contract | `process::UserLaunch` + `Scheduler::spawn_user` | a running process with a bound root publishes entry RIP, aligned stack top, root, and process generation before its task becomes runnable |
| Ring-3 CPU migration | `scheduler::claim_next` + `arch::context` | published ring-3 tasks may migrate after a context boundary; the target loads CR3 and its TSS `RSP0` before restore, while live mappings remain immutable |
| Service image manifest | `service_images::SERVICE_IMAGES` | six fixed ESP paths and bounded ELF admission in dependency order |
| Retained service images | `service_loader::ServiceImageBundle` | six validated ELF records with page-aligned retained addresses, loaded before `ExitBootServices`, and no filesystem lifetime after UEFI exit |
| Service ELF packaging | `services/images` + `scripts/build-services.ps1` | six independent `x86_64-unknown-none` ELF artifacts, each bounded to 512 KiB and staged under the fixed ESP paths |
| Service image handoff | `arch::boot` + `service_loader::load_from_esp` | all six staged ELF images are loaded and validated before `ExitBootServices`; only bounded metadata survives the firmware boundary |
| Service supervisor | `supervisor::LiveSupervisor` + `service_runtime` | live heartbeat polling, graph-wide quiesce, generation-bumped IPC rebuild, bounded process/page-table/frame reclamation, and restart limits |
| Ring-3 proof domain | `user_mode` + `arch` | one fixed ELF admitted through `ProcessTable`, bound root/code/stack mappings, explicit scheduler CR3 selection, DPL-3 vector 49, and contained #UD/#GP/#PF |
| Fatal path | `arch::fatal` | one debug marker, interrupts disabled, every CPU halts |
| Runtime handoff | `handoff_to_runtime` | registers one root `TaskEntry`; the scheduler starts it through the normal context path |
| Proof workload | `qemu-proof` feature | assembly CPU-bound canaries, timer/switch counters, post-CR3 ring-3 migration, reschedule IPIs, event waits, hostile-peer IPC layout/rejection, IPC backpressure edges, keyboard wake, cross-CPU block/wake, structured PASS |

The process-to-scheduler handoff is now explicit: a running process with a bound root produces one
validated `UserLaunch`, and the scheduler publishes its entry, stack, root, and process generation
before marking the task runnable. Hardware page-table construction, ring-3 entry, and safe live
replacement are part of the service path.

AP startup is deliberately narrow: xAPIC IDs, low-memory trampoline, current CR3, NXE, fixed
stacks, and sequential INIT/SIPI/SIPI. x2APIC IDs, malformed topology, more than eight CPUs,
allocators, affinity, priorities, and AVX/XSAVE are not part of this milestone.

The handoff registers one root task. That task owns the first fixed Runtime operation table; Core does
not inspect, schedule, or orchestrate Runtime state. Runtime operations use the scheduler's sleep and
wake primitives but retain their own deadlines, terminal states, and slot generations. The six service
ELFs are loaded before `ExitBootServices`, receive isolated roots and explicit mappings, and enter
through the normal scheduler path. QEMU exercises the live service images and supervisor-driven restart.
The fixed scheduler task stack is 64 KiB with a 256-byte canary so bounded interrupt
and syscall depth cannot silently overwrite adjacent CPU metadata.

`v1_docs/` is historical and is not an active architecture contract.

## Future persistence boundary

Live supervisor restart rebuilds volatile state and abandons in-flight work. Durable state is introduced
through the bounded storage boundary in ADR-0041, with explicit ownership, journal, replay, durability,
and idempotency proofs. The host-tested `logos-storage` and `logos-storage-service` packages provide
the format, journal, namespace, file API, and IPC adapter. The boot image is admitted independently;
the kernel-mediated storage endpoint is identity-checked; requests reach the bounded VirtIO adapter,
and the fresh-disk QEMU proof covers format, flush, reopen, and torn-journal recovery.

## Deferred next-step improvements

The bounded storage milestone now proves durable file commands, reboot reopen, and torn-journal
recovery. A generalized service topology remains deferred until the fixed six-service graph has
stable lifecycle evidence.
