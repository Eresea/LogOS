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
