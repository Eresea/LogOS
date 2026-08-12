# Architecture decisions

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-preemptive-smp-core.md) | Accepted | Fixed-stack preemptive SMP Core and canonical contexts |
| [0002](0002-service-restart-operation-outcomes.md) | Accepted | Service restarts explicitly terminate in-flight operations; retries remain owner policy |
| [0003](0003-terminal-service-contracts.md) | Accepted | Bounded terminal service graph, typed pages, and restart-safe endpoint identity |
| [0004](0004-ring3-proof-boundary.md) | Accepted | One fixed ring-3 proof domain with scheduler-owned fault containment |
| [0005](0005-process-address-space-ownership.md) | Accepted | Process admission owns one generation-safe address-space identity |
| [0006](0006-process-capability-authorization.md) | Accepted | Process handles authorize typed capability checks before service access |
| [0007](0007-bounded-elf-load-plans.md) | Accepted | ELF admission produces a bounded executable load plan |
| [0008](0008-proof-process-uses-model.md) | Accepted | The ring-3 proof registers through the process model and mapping contract |
| [0009](0009-bounded-syscall-dispatch.md) | Accepted | Vector 49 dispatches one validated user syscall payload |
| [0010](0010-scheduler-cr3-selection.md) | Accepted | Scheduler handoff selects the kernel or proof CR3 explicitly |
| [0011](0011-scheduler-address-space-roots.md) | Accepted | Task slots publish one bounded address-space root for handoff |
| [0012](0012-service-abi-boundary.md) | Accepted | Kernel and terminal services share only fixed ABI values; control-plane syscalls and data-plane IPC remain distinct |
| [0013](0013-boot-resource-publication.md) | Accepted | UEFI handles become bounded copied resource descriptors before `ExitBootServices` |
| [0014](0014-bounded-frame-supply.md) | Accepted | User address spaces consume only a fixed conventional-frame pool with explicit exhaustion and release |
| [0015](0015-service-control-plane.md) | Accepted | Control operations use typed capability-gated syscalls; terminal data remains shared IPC |
| [0016](0016-elf-page-admission.md) | Accepted | Validated ELF plans acquire bounded segment and stack frames with explicit rollback |
| [0017](0017-fixed-glyph-cache.md) | Accepted | Display resolves scalars through a fixed 8×16 glyph cache with deterministic fallback |
| [0018](0018-display-raster-boundary.md) | Accepted | Display owns dirty-cell rasterization and pixel format conversion; terminal remains cell-only |
| [0019](0019-service-model-ownership.md) | Accepted | Input, Terminal, and Session state machines live in independent no-std service packages |
| [0020](0020-service-entry-facades.md) | Accepted | Terminal, Session, and Commands expose one-message bounded service façades over their owned state |
| [0021](0021-user-launch-contract.md) | Accepted | Loaded process launch metadata is published atomically with its scheduler task |
| [0022](0022-service-image-manifest.md) | Accepted | Five fixed service image paths and capability grants are validated before loading |
| [0023](0023-retained-service-images.md) | Accepted | Validated service ELF files become five fixed post-UEFI physical image records |
| [0024](0024-service-elf-packaging.md) | Accepted | Five no_std service ELF artifacts are built and size/magic checked into the ESP staging layout |
| [0025](0025-service-image-boot-handoff.md) | Accepted | UEFI loads and retains all five validated service images before ExitBootServices |
| [0026](0026-elf-page-population.md) | Accepted | Loaded images populate owned frames through a bounded page-local sink |
| [0027](0027-bounded-page-table-builder.md) | Accepted | Four-level user page tables are built through a bounded architecture memory seam |
| [0028](0028-service-address-space-bootstrap.md) | Accepted | Five retained service ELFs are populated into owned frames and isolated roots after UEFI exit |
| [0029](0029-service-process-admission.md) | Accepted | Services receive generation-safe process handles, mappings, capabilities, and launch records before scheduling |
