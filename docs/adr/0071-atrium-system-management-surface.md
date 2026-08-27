# ADR-0071: Atrium System Management Surface

- Status: Accepted
- Date: 2026-08-27

## Decision

System is a normal built-in service with `ManagerRights::INSPECT`. It requests one
`AtriumApp::System` surface through four static IPC endpoints; Atrium admits, stores,
focuses, routes input to, and revokes that surface. System only reads bounded service
manager `List` snapshots and emits surface-scoped draw batches.

The first view is limited to service names and lifecycle states. CPU, RAM, device, and
failure telemetry wait for a bounded read-only metrics ABI. Start, stop, restart, and
surface construction remain Core/Atrium authority and are not exposed by System.

## Consequences

- A System process cannot create or retain an untracked framebuffer surface.
- Service status is visible without granting lifecycle capability.
- The static endpoint topology grows by four append-only endpoint IDs.
- Session logout and focused close revoke the System surface through its response channel.
