# Architecture decisions

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-preemptive-smp-core.md) | Accepted | Fixed-stack preemptive SMP Core and canonical contexts |
| [0002](0002-service-restart-operation-outcomes.md) | Accepted | Service restarts explicitly terminate in-flight operations; retries remain owner policy |
| [0003](0003-terminal-service-contracts.md) | Accepted | Bounded terminal service graph, typed pages, and restart-safe endpoint identity |
| [0004](0004-ring3-proof-boundary.md) | Accepted | One fixed ring-3 proof domain with scheduler-owned fault containment |
| [0005](0005-process-address-space-ownership.md) | Accepted | Process admission owns one generation-safe address-space identity |
| [0006](0006-process-capability-authorization.md) | Superseded | Typed capability metadata was removed until an enforcing syscall boundary exists |
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
| [0022](0022-service-image-manifest.md) | Superseded | Five fixed service image paths are validated before loading; capability grants are deferred |
| [0023](0023-retained-service-images.md) | Accepted | Validated service ELF files become five fixed post-UEFI physical image records |
| [0024](0024-service-elf-packaging.md) | Accepted | Five no_std service ELF artifacts are built and size/magic checked into the ESP staging layout |
| [0025](0025-service-image-boot-handoff.md) | Accepted | UEFI loads and retains all five validated service images before ExitBootServices |
| [0026](0026-elf-page-population.md) | Accepted | Loaded images populate owned frames through a bounded page-local sink |
| [0027](0027-bounded-page-table-builder.md) | Accepted | Four-level user page tables are built through a bounded architecture memory seam |
| [0028](0028-service-address-space-bootstrap.md) | Accepted | Five retained service ELFs are populated into owned frames and isolated roots after UEFI exit |
| [0029](0029-service-process-admission.md) | Superseded | Services receive generation-safe process handles, mappings, and launch records before scheduling |
| [0030](0030-user-launch-transition.md) | Accepted | Scheduler CR3 selection and validated ring-3 iret launch are centralized before service scheduling |
| [0031](0031-service-startup-barrier.md) | Accepted | Five services advance through a fixed dependency-ordered launch barrier before execution |
| [0032](0032-service-ipc-pages.md) | Superseded | Replaced by ADR-0040 kernel-owned queues and private service IPC pages |
| [0034](0034-ring3-bsp-affinity.md) | Superseded | Historical BSP-only guard replaced by bounded ring-3 SMP migration in ADR-0039 |
| [0033](0033-ps2-interrupt-boundary.md) | Accepted | Legacy PIC IRQ1 supplies bounded raw PS/2 bytes to Input; decoding remains in the service |
| [0035](0035-live-supervisor-restart.md) | Accepted | Supervisor-owned graph restart quiesces tasks, reclaims address spaces safely, and rejects stale IPC generations |
| [0036](0036-deferred-capability-metadata.md) | Superseded | Replaced by ADR-0040's enforced process-bound IPC capabilities |
| [0037](0037-bounded-memory-subsystem.md) | Accepted | Layered bounded physical, virtual, async, SMP, heap, pressure, and observability contracts behind stable frame interfaces |
| [0038](0038-event-driven-blocking-ipc.md) | Superseded | Event masks remain, but transport moved to kernel-owned queues in ADR-0040 |
| [0039](0039-ring3-smp-migration.md) | Accepted | Ring-3 tasks migrate across online CPUs with CR3-first handoff and bounded reschedule IPIs |
| [0040](0040-hostile-peer-ipc.md) | Accepted | Kernel-owned bounded queues enforce process-bound directional capabilities through private staging pages |
