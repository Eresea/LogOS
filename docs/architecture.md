# LogOS vNext Core architecture

The package has two targets: the UEFI binary in `src/main.rs` calls `boot()` in `src/lib.rs`,
while independent `x86_64-unknown-none` service images run in isolated roots. ABI v5 owns one
production allocator, private service heaps, and generation-safe service/IPC handles. The ten
built-in images are the bootstrap set, while Core publishes runtime service, endpoint,
capability, event, and event-set records.
The live UEFI path uses the v5 registries and discovered capabilities; the old slot/graph/mask
model is retained only in host-test fixtures where it still models bounded scheduler behavior.
The capability and event directory syscalls expose v5 cursored records with stable typed contract
IDs, direction-specific event handles, and owner/source metadata. Service images resolve their
private IPC descriptors and hardware waits to opaque runtime handles; Core-owned queues and
allocation-free producer wake paths are authoritative.

```text
Terminal → Session → Flow → typed system API registry
                         ├── Storage
                         ├── Device
                         ├── Network
                         ├── Supervisor
                         └── Fetch
```

| Boundary | Owner | Invariant / proof |
| --- | --- | --- |
| UEFI handoff | `arch::boot` | discovers 1–8 healthy CPUs, stages fixed AP trampoline, exits boot services |
| Boot resources | `boot_resources` | copies bounded memory-map, GOP framebuffer, and PS/2 identities before UEFI handles are discarded |
| Physical frames | `frame_pool` + `memory` | copied UEFI descriptors normalize into sorted disjoint runs; ABI v5 reserves metadata pages before `ExitBootServices`, then sizes frame records from the discovered map with generation-safe leases, owner accounting, reclaim, and explicit exhaustion |
| Memory subsystem contracts | `memory` | one production allocator owns physical frames; Core binds a `KernelGlobalAllocator` after handoff, charged to `OwnerId::KERNEL`, with bounded reclaim and fatal unrecoverable OOM; service heaps use shared read-only allocator code, private metadata, kernel-mediated growth, quotas, and bounded reclaim; interrupt paths remain explicitly nonblocking |
| Control plane | `process` + `user_mode` | admission-time service mappings plus bounded event-set, IpcSend, IpcReceive, and ServiceManager calls with process-bound handle checks |
| ELF page admission | `loader` | maps validated segments and fixed user stacks to owned frames, then populates them through a page-local sink with bounded reclamation; Storage receives a fixed 128-page stack for bounded snapshot decoding and transaction state |
| User page tables | `page_table` | builds four-level user mappings with fixed root/intermediate-frame bounds, W^X/NX flags, conflict rejection, and grouped reclamation |
| Service address spaces | `service_runtime` | loads twelve retained ELFs into owned frames and retains one isolated root per service before scheduler admission |
| Service process admission | `service_runtime` + `process` | binds each service root, coalesced mappings, and validated user launch metadata without entering service RIPs prematurely |
| User launch transition | `arch` + `scheduler` | selects the task root before restore and provides the fixed-selector `iretq` path for service entry |
| Service startup barrier | `service_startup` | enforces image → address space → process → launch-ready states and Input/Display → Terminal → Session → Storage → User/Device → Flow → Shell → LockScreen dependencies; Network is independent and Fetch depends on it |
| Service IPC boundary | `runtime_ipc` + `service_runtime` | ABI v5 owns runtime endpoint records, generation-safe capability grants, private staging, and Core-only queue frames; typed payloads remain bounded and only Core-granted capabilities are discoverable |
| Network IPC extension | `service_ipc` + `service_runtime` | adds Flow/Fetch client routing with ABI v2 inline payloads capped at 192 bytes; Core-owned packet descriptors/pages remain private to Network |
| Storage boundary | Core VirtIO block adapter + `logos-storage` v5 COW root + storage IPC/object service | Core owns PCI discovery, feature negotiation, fixed DMA arena, queues, MSI-X interrupt delivery, reset, timeouts, and flush; Storage owns dual roots, immutable extents, disjoint system/user/package pools, persistent allocation metadata, flushed commit records, recovery, durable object publication, object IDs, namespace resolution, and bounded file operations; Flow reaches Storage through versioned messages over private staging pages; v4 media fails closed at the v5 opener and is never silently reformatted; one active writer remains bounded while generation-safe handles, bounded multi-extent streamed files, and map/unmap validation are active |
| User service | `services/images/src/user` + `services/user` | User owns canonical identities, Argon2id password verifiers, role capability templates, volatile session/capability handles, and lineage revocation; Flow reaches User through typed IPC, while User exchanges only chunked snapshots with Storage; no UID/GID mode bits or ambient path authority |
| Filesystem package boundary | `logos-package` + Storage v3 + `storage_ipc` + `service_runtime` | The legacy service envelope remains readable while the v2 service envelope carries a bounded manifest and streamed CRC validation; v3 keeps ordinary files capped at 8 KiB and stores up to sixteen generation-safe package records in a disk-derived extent arena; Storage reads v2 manifests from package extents on lookup and rejects non-newer updates or broken dependent ranges, while Core↔Storage package Lookup/Read retains one outstanding request and one reused 4 KiB frame with request, generation, offset, length, and service-epoch validation |
| Package memory budget | `loader` + `frame_pool` + `memory` | Package extents allocate only the blocks required by the package; prepared ELF images allocate exact code/data/BSS/stack/page-table frames under the selected service owner, and every failed prepare or restart path reclaims them before the old graph is discarded |
| Network boundary | Core VirtIO-net adapter + Network service + versioned Network IPC | Core owns optional PCI discovery, one fixed 64-entry RX/TX pair, 2 KiB DMA buffers, interrupts, reset, and deadlines; Network owns smoltcp Ethernet/ARP/IPv4/ICMP/UDP/DHCPv4/TCP state, eight sockets, two listeners, and private packet pages; Flow and Fetch receive routed inline payload responses; HTTP remains outside Network |
| Display device mapping | `service_runtime` + `process` | maps only the bounded retained GOP range into Display at `DISPLAY_FRAMEBUFFER_BASE`, one read-only `FramebufferConfig` page at `DISPLAY_CONFIG_BASE`, and one writable atomic present-sequence page at `DISPLAY_PRESENT_BASE`; boot rejects modes below the fixed 80×25/8×16 profile; no other service or kernel drawing path receives it |
| Keyboard byte mapping | `logos-abi` + `service_runtime` | allocates one zeroed fixed byte ring with an observable drop counter and maps it only into Input at `INPUT_KEYBOARD_RING_BASE`; PS/2 decoding remains outside the kernel |
| Pointer byte mapping | `logos-abi` + `service_runtime` | allocates one zeroed fixed byte ring with an observable drop counter and maps it only into Input at `INPUT_POINTER_RING_BASE`; the existing three-byte decoder remains outside Core |
| PS/2 interrupt adapter | `arch` | remaps the legacy PIC, unmasks IRQ1 and IRQ12 after the Input rings are published, copies port `0x60` bytes into their respective rings, and signals distinct keyboard/pointer events; no decoding occurs in Core |
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
| Terminal ABI | `logos-abi` | fixed semantic input, session stream, surface-scoped cell-diff render, bootstrap endpoint policy, and generation-safe service identities |
| IPC mechanics | `runtime_ipc` + `service_ipc` + `scheduler::Scheduler` | v5 runtime handles, discovered capabilities, exact typed payload validation, queue backpressure, and event-set operations own all ordinary service traffic; hardware IRQ adapters signal pre-existing event objects without allocation |
| Input service | `services/images/src/input` + `logos-input::InputDecoder` | consumes the Input-only PS/2 keyboard and pointer byte mappings, produces semantic key/text/pointer messages on the Input→Shell and Input→Atrium rings, waits on both hardware events, and owns decoder state |
| Shell service | `services/images/src/shell` + `logos-shell::Shell` | owns bounded User claim/login/logout brokering, credential clearing, restart invalidation, and typed session-context handoff to Flow; Atrium owns GUI routing and window policy |
| LockScreen service | `services/images/src/lockscreen` + `logos-lockscreen::LockScreen` | owns bounded credential editing/rendering, first-boot claim mode, retry/failure state, password clearing, fixed left-button field/submit hit targets, its visible native cursor surface, and typed auth requests to Shell; it receives input and visibility notifications from Atrium and no framebuffer or filesystem authority |
| Atrium service | `services/images/src/atrium` + `logos-atrium::Atrium` | owns Boot/Locked/Home routing, bounded surface request/admission and teardown, generation-safe client identities and surface references, authoritative focus order, pointer position/Home cursor-surface updates, pointer hit testing/capture and surface-local routing, keyboard focus/movement/close policy, and app surface orchestration; applications/processes only receive Atrium-issued references; it never receives the framebuffer |
| System service | `services/images/src/system` | inspect-only management process; requests one Atrium-owned surface and renders the bounded service-manager list, while Core/Atrium retain all lifecycle and surface authority; CPU/RAM/device telemetry requires a later bounded metrics ABI |
| Terminal service | `services/images/src/terminal` + `logos-terminal::TerminalState` | ring-3 owns bounded terminal state, consumes Session rings and Atrium-routed surface input, acquires its Atrium-issued reference, stamps it onto the compatibility dirty-cell stream, and sends that stream through Atrium |
| Display service | `services/images/src/display` + `logos-display` | sole framebuffer writer; receives validated terminal compatibility messages plus atomic retained scene operations, damages old/new stable-node bounds, composes bounded dirty tiles into a RAM backbuffer, skips occluded lower nodes, retains static glyph runs, and presents only completed regions to its mapped GOP framebuffer |
| Session service | `services/images/src/session` + `logos-session::SessionService` | ring-3 owns bounded line editing, history, prompt state, and completion requests; Flow evaluation state is not retained here and no state survives restart or reboot |
| Flow service | `services/images/src/flow` + `logos-flow::FlowService` | receives bounded Session source, lexes/parses/type-checks/evaluates fixed Flow programs, owns eight variables and four promise slots, routes typed registry operations to Storage/Network/Device/Supervisor/Fetch, provides stale-safe completion, and returns backpressured output over its reverse IPC ring |
| Device service | `services/images/src/device` + `logos-device` | owns a bounded physical inventory view, currently exposing the Core-owned block disk through `device.list()`; format and filesystem recreation remain a later capability |
| Fetch service | `services/images/src/fetch` + `logos-fetch` | owns one bounded HTTP operation, split-frame response parsing, progress/cancellation, and staged Storage publication; only numeric IPv4 `http://` 2xx downloads are accepted |
| User service | `services/images/src/user` + `logos-user` | owns persistent identities, verifiers, roles, and authorization lineage; loads/saves only the User catalog through typed Storage IPC, receives a Core-mapped fixed Argon2id workspace, and exposes volatile sessions/capabilities to Flow |
| Process admission | `process::ProcessTable` | fixed 16-slot process model, bounded ELF64 load plans, one generation-safe address-space identity with 64 validated mappings per process, and exit/fault/reclaim outcomes |
| User launch contract | `process::UserLaunch` + `Scheduler::spawn_user` | a running process with a bound root publishes entry RIP, aligned stack top, root, and process generation before its task becomes runnable |
| Ring-3 CPU migration | `scheduler::claim_next` + `arch::context` | published ring-3 tasks may migrate after a context boundary; the target loads CR3 and its TSS `RSP0` before restore, while live mappings remain immutable |
| Service image manifest | `service_images::SERVICE_IMAGES` | built-in ESP images are bootstrap policy; ABI v5 registers them through generation-safe runtime records and permits additional policy-approved records |
| Retained service images | `service_loader::ServiceImageBundle` | twelve validated ELF records with page-aligned retained addresses, loaded before `ExitBootServices`, and no filesystem lifetime after UEFI exit |
| Service ELF packaging | `services/images` + `scripts/build-services.ps1` | twelve independent `x86_64-unknown-none` ELF artifacts, each bounded to 512 KiB and staged under the fixed ESP paths |
| Service image handoff | `arch::boot` + `service_loader::load_from_esp` | all twelve staged ELF images are loaded and validated before `ExitBootServices`; only bounded metadata survives the firmware boundary |
| Service supervisor | `supervisor::LiveSupervisor` + `service_runtime` | heap-backed generation-safe service-handle records, live heartbeat polling, graph-wide quiesce, generation-bumped IPC rebuild, bounded process/page-table/frame reclamation, and restart limits |
| Service manager | `runtime_services` + `service_manager` + `service_runtime` | dynamic records, generation-safe handles, allocated dependencies, opaque list cursors, dynamic status/failure discovery, and service lifecycle admission are live; the fixed manager remains only for bounded program lifecycle and bootstrap image metadata; System receives only `ManagerRights::INSPECT` |
| Program lifecycle | `service_manager` + `service_runtime` + `process` + `scheduler` | eight fixed name-keyed program slots reuse the service manager ABI and Core resource owner; program ELF images receive private code/data/stack mappings plus a read-only surface bootstrap and five bounded Atrium channels, while stop waits for scheduler completion before reclaiming all program resources |
| Program client | `logos-program` + `programs/demo` | no-std client consumes the read-only bootstrap, requests one Atrium surface, polls its response/input channels, and submits only surface-scoped cell or GUI draw messages |
| Ring-3 proof domain | `user_mode` + `arch` | one fixed ELF admitted through `ProcessTable`, bound root/code/stack mappings, explicit scheduler CR3 selection, DPL-3 vector 49, and contained #UD/#GP/#PF |
| Fatal path | `arch::fatal` | one debug marker, interrupts disabled, every CPU halts |
| Runtime handoff | `handoff_to_runtime` | registers one root `TaskEntry`; the scheduler starts it through the normal context path |
| Proof workload | `qemu-proof` feature | assembly CPU-bound canaries, timer/switch counters, post-CR3 ring-3 migration, reschedule IPIs, event waits, hostile-peer IPC layout/rejection, IPC backpressure edges, independent keyboard/pointer wake, QMP relative pointer/button injection, LockScreen native-cursor raster proof, cross-CPU block/wake, structured PASS; semantic pointer routing remains host-tested |

The process-to-scheduler handoff is now explicit: a running process with a bound root produces one
validated `UserLaunch`, and the scheduler publishes its entry, stack, root, and process generation
before marking the task runnable. Hardware page-table construction, ring-3 entry, and safe live
replacement are part of the service path.

The ABI v5 dynamic-resource migration is live for the built-in service runtime. Core initializes
runtime endpoint, capability, service, and event registries; ordinary service traffic uses discovered
handles, route-specific Core adapters resolve live endpoint records, and the scheduler publishes waits
on event-set objects. Fixed bounds remain only where they describe typed payloads, hardware adapters,
process/task admission, or the separately scoped program lifecycle.

AP startup is deliberately narrow: xAPIC IDs, low-memory trampoline, current CR3, NXE, fixed
stacks, and sequential INIT/SIPI/SIPI. x2APIC IDs, malformed topology, more than eight CPUs,
allocators, affinity, priorities, and AVX/XSAVE are not part of this milestone.

The handoff registers one root task. That task owns the first fixed Runtime operation table; Core does
not inspect, schedule, or orchestrate Runtime state. Runtime operations use the scheduler's sleep and
wake primitives but retain their own deadlines, terminal states, and slot generations. The ten service
ELFs are loaded before `ExitBootServices`, receive isolated roots and explicit mappings, and enter
through the normal scheduler path. QEMU exercises the live service images and supervisor-driven restart.
Core enters on a 512 KiB per-CPU scheduler stack with a 256-byte canary; task stacks remain
separately bounded so interrupt and syscall depth cannot silently overwrite adjacent CPU metadata.

`v1_docs/` is historical and is not an active architecture contract.

## Persistence boundary

Live supervisor restart rebuilds volatile state and abandons in-flight work. Durable state is introduced
through the bounded storage boundary in ADR-0059, with explicit ownership, immutable roots, recovery,
durability, and idempotency proofs. The host-tested `logos-storage` and `logos-storage-service`
packages provide the v4 compatibility format, the v5 system-catalog root, the v5 namespace backend,
the file API, and the User catalog boundary. The boot image is admitted independently;
the kernel-mediated storage endpoint is identity-checked; requests reach the bounded VirtIO adapter,
and the fresh-disk QEMU proof covers format, flush, reopen, and torn-root recovery.

Storage compatibility is fail-closed. The v5 namespace and User system-catalog openers return
`UnsupportedVersion` for v4 roots and never silently reformat them. The v4 opener remains only for
legacy host proofs. Both roots write allocation metadata, a commit
record, and an alternate root in order, flushing each publication boundary. Package payloads use an
arena outside the metadata allocation prefix and are catalogued only after validation.

The User policy core is host-tested against the v5 system-catalog boundary, and the live User image
loads/saves that snapshot through the typed User↔Storage chunk protocol. Its snapshot format
persists identities, verifiers, roles, and namespace-root descriptors but deliberately omits sessions
and live capability handles. The live `DurableNamespaceV5` implements `UserCatalogStore`, so User
catalog updates and namespace commits publish through the same v5 root and reserved system
allocation class; Flow exposes bounded `user.*` commands through User. The post-v4 storage layout
validator and arena-scoped COW allocator define disjoint system, user-content, and package ranges,
while the v5 namespace routes file content through the user pool. Ordinary path operations are not treated as a user
authorization boundary.

Package-backed service records use a name-bound package target for lookup and streamed ELF loading.
Register, Start, Stop, and Restart operate on the dynamic service handle; a failed package lookup or
image admission leaves the record failed and reclaims any partially allocated process, page-table,
heap, IPC, and event resources. Built-in image metadata remains bootstrap policy, not lifecycle identity.

## Deferred next-step improvements

The bounded storage milestone now proves durable Flow filesystem workflows, reboot reopen, torn-root
recovery, generation-safe file handles, bounded multi-extent COW-backed streamed files beyond the inline cache,
single-extent read-only MapRead/UnmapRead pins, page-table unmap/TLB invalidation hooks, bounded Storage cache-window grants for Flow/Fetch, variable-sized
service packages, reader-based ELF streaming, graph-wide package activation, read-only Flow package
inventory/info queries, and bounded package-file import/update. Repository
resolution, signatures, boot preference, richer program UI APIs,
capabilities, and automatic boot remain deferred.
Persistent program packages now use the same manifest, catalog, and manager transport.
The ABI v5 migration is bounded by physical memory, owner quotas, typed payload sizes, and explicit
backpressure. Arbitrary capability delegation and unvalidated service-to-service connection policy
remain outside the boundary.
