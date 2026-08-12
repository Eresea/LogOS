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
