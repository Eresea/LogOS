# Architecture Decision Records

ADRs preserve decisions that change a subsystem boundary, commitment, or long-term constraint.

Create one for an irreversible or cross-ring decision. Do not create one for routine, reversible implementation work. Use the next four-digit number and the [template](template.md), then update the affected roadmap and architecture documents.

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-terminal-service-boundary.md) | Accepted | Run the normal terminal as a separately loaded Sessions service. |
| [0002](0002-test-control-boundary.md) | Accepted | Gate deterministic test control outside production. |
| [0003](0003-native-service-payload-contract.md) | Accepted | Stage native services as versioned boot payloads. |
| [0004](0004-native-service-address-spaces.md) | Accepted | Isolate each native service in its own address space. |
| [0005](0005-native-service-suspension.md) | Accepted | Suspend native services at Core-owned gates. |
| [0006](0006-native-normal-console-handoff.md) | Accepted | Hand off the normal console to the loaded native terminal. |
| [0007](0007-typed-native-command-gate.md) | Accepted | Send typed terminal calls across the native command gate. |
| [0008](0008-input-contract-v1.md) | Accepted | Introduce the typed `foundation.input` v1 contract first. |
| [0009](0009-display-contract-v1.md) | Accepted | Introduce validated RGB display values before capability routing. |
| [0010](0010-session-contract-v1.md) | Accepted | Gate typed terminal syscalls with an explicit Session capability. |
| [0011](0011-session-dispatch-service.md) | Accepted | Run normal command dispatch in a Sessions service. |
| [0012](0012-service-runtime-boundary.md) | Accepted | Establish logos-service-rt as the runtime for Ring 1-3 services. |
| [0013](0013-persistence-v1-boundary.md) | Accepted | Use scoped shared pages and a two-arena object Store for Persistence v1. |
| [0014](0014-secret-root-key.md) | Accepted | Keep the bootstrap durable-secret key in a random UEFI variable. |
| [0015](0015-network-v1-boundary.md) | Accepted | Keep NIC DMA in Core and expose exact capability-scoped datagram endpoints. |
| [0016](0016-capability-slices.md) | Accepted | Compose system capabilities from independently versioned module contracts. |
| [0017](0017-native-service-fault-restart.md) | Accepted | Reload native services after contained Ring-3 faults. |
| [0018](0018-remote-foundation-boundary.md) | Accepted | Keep Remote Foundation behind bounded TCP, trust, and session gates. |
| [0019](0019-remote-v1-administration-attachment.md) | Accepted | Extend Remote Foundation into a persistent, credit-bounded administration attachment. |
| [0020](0020-typed-native-endpoint-pages.md) | Accepted | Use typed ABI-v4 endpoint pages and one canonical native-service specification. |

