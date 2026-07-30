# Applications

> **Status:** Applications v1 planned

## Goal

Run portable applications as capability-isolated WASM components without making the runtime part of kernel correctness.

## Runtime and packages

- [ ] Validate/load WASM components through versioned native LogOS WIT interfaces; WASI remains compatibility-only.
- [ ] Capability-scoped imports; memory, CPU, storage, and network quotas; cancellation and asynchronous host operations.
- [ ] Health, lifecycle, crash isolation, restart policy, restricted execution where useful, and runtime rollback.
- [ ] Signed packages with application identity, declared interfaces/capabilities, state migration, atomic update/rollback, and a Rust SDK first.

## Workspaces

- [ ] Workspace identity, metadata, resources, applications, services, sessions, and scoped capability grants.
- [ ] Export, import, snapshot, and recovery without granting machine-wide authority.

## AI-addressable operation

- [ ] Typed tool descriptors expose schemas, capabilities, side effects, and approval policy.
- [ ] Human approval for sensitive effects; no implicit agent privilege.
- [ ] Auditable agent sessions and bounded decision records without private model reasoning.
- [ ] Re-approval when untrusted content would trigger privileged effects.
- [ ] Sensitive-value labels with explicit audited declassification before remote-model access.
- [ ] Optional compensating actions backed by Store versions where reversal exists.

## Exit criteria

A WASM application can be remotely installed, run, communicate, persist, upgrade, roll back, and be removed using only granted interfaces. Failures remain isolated; workspaces remain portable; human and authorized agent clients use the same operations.

See [Architecture](ARCHITECTURE.md#ring-4--runtime), [AI addressability](ARCHITECTURE.md#14-ai-addressability), and [Security](security.md).
