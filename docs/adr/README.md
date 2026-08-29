# Architecture decisions

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-preemptive-smp-core.md) | Accepted | Fixed-stack preemptive SMP Core and canonical contexts |
| [0002](0002-service-restart-operation-outcomes.md) | Accepted | Service restarts explicitly terminate in-flight operations; retries remain owner policy |
| [0003](0003-terminal-service-contracts.md) | Accepted | Bounded terminal service graph, typed pages, and restart-safe endpoint identity |
| [0004](0004-ring3-proof-boundary.md) | Accepted | One fixed ring-3 proof domain with scheduler-owned fault containment |
| [0005](0005-process-address-space-ownership.md) | Accepted | Process admission owns one generation-safe address-space identity |
| [0006](0006-process-capability-authorization.md) | Superseded by ADR-0036 | Typed capability metadata was removed until an enforcing syscall boundary exists |
| [0007](0007-bounded-elf-load-plans.md) | Accepted | ELF admission produces a bounded executable load plan |
| [0008](0008-proof-process-uses-model.md) | Accepted | The ring-3 proof registers through the process model and mapping contract |
| [0009](0009-bounded-syscall-dispatch.md) | Accepted | Vector 49 dispatches one validated user syscall payload |
| [0010](0010-scheduler-cr3-selection.md) | Accepted | Scheduler handoff selects the kernel or proof CR3 explicitly |
| [0011](0011-scheduler-address-space-roots.md) | Accepted | Task slots publish one bounded address-space root for handoff |
| [0012](0012-service-abi-boundary.md) | Accepted | Kernel and terminal services share only fixed ABI values; service access is admission-time and data-plane IPC remains distinct |
| [0013](0013-boot-resource-publication.md) | Accepted | UEFI handles become bounded copied resource descriptors before `ExitBootServices` |
| [0014](0014-bounded-frame-supply.md) | Accepted | User address spaces consume only a fixed conventional-frame pool with explicit exhaustion and release |
| [0015](0015-service-control-plane.md) | Superseded | Generic service syscall control plane deferred; proof-only Yield/Heartbeat remain |
| [0016](0016-elf-page-admission.md) | Accepted | Validated ELF plans acquire bounded segment and stack frames with explicit rollback |
| [0017](0017-fixed-glyph-cache.md) | Accepted | Display resolves scalars through a fixed 8×16 glyph cache with deterministic fallback |
| [0018](0018-display-raster-boundary.md) | Accepted | Display owns dirty-cell rasterization and pixel format conversion; terminal remains cell-only |
| [0019](0019-service-model-ownership.md) | Accepted | Input, Terminal, and Session state machines live in independent no-std service packages |
| [0020](0020-service-entry-facades.md) | Accepted | Terminal, Session, and Commands expose one-message bounded service façades over their owned state |
| [0021](0021-user-launch-contract.md) | Accepted | Loaded process launch metadata is published atomically with its scheduler task |
| [0022](0022-service-image-manifest.md) | Accepted | Five fixed service image paths are validated before loading; capability grants are deferred |
| [0023](0023-retained-service-images.md) | Accepted | Validated service ELF files become five fixed post-UEFI physical image records |
| [0024](0024-service-elf-packaging.md) | Accepted | Five no_std service ELF artifacts are built and size/magic checked into the ESP staging layout |
| [0025](0025-service-image-boot-handoff.md) | Accepted | UEFI loads and retains all five validated service images before ExitBootServices |
| [0026](0026-elf-page-population.md) | Accepted | Loaded images populate owned frames through a bounded page-local sink |
| [0027](0027-bounded-page-table-builder.md) | Accepted | Four-level user page tables are built through a bounded architecture memory seam |
| [0028](0028-service-address-space-bootstrap.md) | Accepted | Five retained service ELFs are populated into owned frames and isolated roots after UEFI exit |
| [0029](0029-service-process-admission.md) | Accepted | Services receive generation-safe process handles, mappings, and launch records before scheduling |
| [0030](0030-user-launch-transition.md) | Accepted | Scheduler CR3 selection and validated ring-3 iret launch are centralized before service scheduling |
| [0031](0031-service-startup-barrier.md) | Accepted | Five services advance through a fixed dependency-ordered launch barrier before execution |
| [0032](0032-service-ipc-pages.md) | Accepted | Replaced by ADR-0040 kernel-owned queues and private service IPC pages |
| [0033](0033-ps2-interrupt-boundary.md) | Accepted | Legacy PIC IRQ1 supplies bounded raw PS/2 bytes to Input; decoding remains in the service |
| [0034](0034-ring3-bsp-affinity.md) | Superseded by ADR-0039 | Historical BSP-only guard replaced by bounded ring-3 SMP migration in ADR-0039 |
| [0035](0035-live-supervisor-restart.md) | Accepted | Supervisor-owned graph restart quiesces tasks, reclaims address spaces safely, and rejects stale IPC generations |
| [0036](0036-deferred-capability-metadata.md) | Accepted | Replaced by ADR-0040's enforced process-bound IPC capabilities |
| [0037](0037-bounded-memory-subsystem.md) | Accepted | Layered bounded physical, virtual, async, SMP, heap, pressure, and observability contracts behind stable frame interfaces |
| [0038](0038-event-driven-blocking-ipc.md) | Accepted | Event masks remain, but transport moved to kernel-owned queues in ADR-0040 |
| [0039](0039-ring3-smp-migration.md) | Accepted | Ring-3 tasks migrate across online CPUs with CR3-first handoff and bounded reschedule IPIs |
| [0040](0040-hostile-peer-ipc.md) | Accepted | Kernel-owned bounded queues enforce process-bound directional capabilities through private staging pages |
| [0041](0041-storage-boundary.md) | Accepted | Core owns block mechanics while a bounded storage format owns journal, replay, recovery, and durability |
| [0042](0042-virtio-block-transport.md) | Accepted | Fixed VirtIO block request chains translate 4096-byte LogOS blocks to 512-byte sectors before hardware integration |
| [0043](0043-virtio-device-ownership.md) | Accepted | Core owns VirtIO PCI, DMA, queues, interrupts, reset, and flush while storage owns logical durability |
| [0044](0044-storage-service-ipc.md) | Accepted | Storage uses fixed kernel-mediated IPC and private staging pages without device access |
| [0045](0045-bounded-object-namespace.md) | Accepted | A fixed storage-service object table provides generation-safe durable namespace operations |
| [0046](0046-storage-service-lifecycle.md) | Accepted | Storage is admitted as the sixth fixed service while its kernel-mediated request endpoint remains separately gated |
| [0047](0047-storage-command-api.md) | Accepted | Commands uses a bounded versioned Storage API; Storage owns one active transaction, shadow state, and durable publication |
| [0048](0048-storage-format-compatibility.md) | Accepted | Storage versions fail closed; v1 recovers valid commits after torn gaps and never silently downgrades or reformats unknown media |
| [0049](0049-service-manager-control-plane.md) | Accepted | Core-owned bounded service lifecycle API uses a process-bound manager capability and fixed control syscall |
| [0050](0050-network-service-boundary.md) | Accepted | Optional Network service owns protocol state while Core owns VirtIO-net transport and targeted recovery |
| [0051](0051-terminal-render-batching.md) | Accepted | Terminal render batches coalesce before display while full redraws clear the complete framebuffer |
| [0052](0052-targeted-terminal-completion.md) | Accepted | Commands-owned bounded completion is optional, best-effort, and isolated from terminal execution failures |
| [0054](0054-flow-interpreter-service-migration.md) | Accepted | Flow replaces Commands in the existing slot with bounded typed evaluation, promise lifecycle, response-body transport, and cancellation |
| [0055](0055-filesystem-service-packages.md) | Accepted | Storage v3 stores variable-sized service packages in a bounded extent arena and Core activates validated ELF packages through a fixed channel |
| [0056](0056-bounded-package-manifests-and-dependency-policy.md) | Accepted | Package metadata uses bounded names, semantic versions, npm-style ranges, four dependencies, strict newer replacement, and topological service activation |
| [0057](0057-read-only-package-inventory-ipc.md) | Accepted | Flow reads bounded package inventory and manifest summaries through Storage without receiving package payload write authority |
| [0058](0058-bounded-package-file-import.md) | Accepted | Flow requests Storage-owned package-file import; Storage streams the existing file into the package arena and reuses validation/update policy |
| [0059](0059-v4-copy-on-write-storage-boundary.md) | Accepted | v4 uses immutable extents, persistent allocation metadata, flushed commit records, atomic root publication, and fail-closed legacy media handling |
| [0060](0060-bounded-program-lifecycle.md) | Accepted | Persistent capability-free program packages reuse the service ABI, fixed Storage catalog, Core resource owner, process loader, and scheduler-safe reclamation |
| [0061](0061-device-manager-service.md) | Accepted | A bounded Device service exposes Core-owned physical inventory through Flow while destructive format/recreate authority remains deferred |
| [0062](0062-user-identity-and-capability-core.md) | Accepted | User identity policy uses Argon2id verifiers, volatile sessions, role templates, typed namespace capabilities, and lineage revocation |
| [0063](0063-storage-system-pool-layout.md) | Accepted | The post-v4 storage format separates reserved system metadata, user content, and package allocation ranges |
| [0064](0064-user-ipc-flow-storage.md) | Accepted | User reaches Flow and v5 Storage through bounded typed IPC and chunked catalog transport |
| [0065](0065-runtime-allocation-ownership.md) | Accepted | UEFI-reserved runtime allocation metadata, Core GlobalAlloc, and private quota-controlled service heaps |
| [0066](0066-dynamic-ipc-topology.md) | Accepted | Versioned generation-safe runtime service, endpoint, capability, and event handles |
| [0067](0067-event-driven-graphical-shell.md) | Accepted | Display-owned retained surfaces with typed invalidation hooks and event-driven graphical shell services |
| [0068](0068-runtime-owned-lazy-service-stacks.md) | Accepted | Services begin with a small stack window and borrow additional stack pages from Core only on validated page faults |
| [0069](0069-atrium-shell-orchestration.md) | Accepted | Atrium owns GUI shell orchestration while Shell brokers authentication, LockScreen owns credentials, and Display owns pixels |
| [0070](0070-program-atrium-surface-contract.md) | Accepted | Running programs receive only bounded Atrium surface channels and a read-only program bootstrap page |
| [0071](0071-atrium-system-management-surface.md) | Accepted | System is an inspect-only service-manager client rendered through one Atrium-owned surface |
| [0072](0072-bounded-gui-raster-primitives.md) | Accepted | Display owns bounded rounded, thick-line, alpha, and fixed-kernel shadow rasterization |
| [0073](0073-bounded-pointer-input-path.md) | Accepted | Bounded IRQ12 → Input → Atrium pointer delivery, wake ownership, capture, and local routing |
| [0074](0074-bounded-native-cursor.md) | Accepted | Atrium-owned bounded native cursor surface and LockScreen left-button interaction |
| [0075](0075-retained-graphics-v2.md) | Accepted | Atomic retained scene commits, stable node damage, dirty-tile RAM composition, caching, and occlusion |
| [0076](0076-bounded-gpu-present-boundary.md) | Accepted | Core-owned bounded GPU resources and queues with Display-owned scene policy, dirty-region present, and software fallback |
| [0077](0077-bounded-display-present-sequence.md) | Accepted | One Core-owned atomic sequence lets Display publish completed framebuffer presents without repeated idle GPU transfers |
| [0078](0078-bounded-gpu-cursor.md) | Accepted | Core-owned bounded VirtIO-GPU hardware cursor with Display-owned image publication and software fallback |
