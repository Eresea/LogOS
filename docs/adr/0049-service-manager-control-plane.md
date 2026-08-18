# ADR-0049: Core service manager control plane

- Status: Accepted
- Date: 2026-08-16

## Decision

Core owns a bounded `ServiceManager` for the six boot-declared services and two
reserved service slots. The manager exposes fixed list, status, start, stop,
and restart operations through `logos-abi` request/response values.

Commands receives one process-bound manager capability with inspect and
lifecycle rights. Requests use the caller's existing private staging page and
a dedicated control syscall; manager operations do not add another streaming
transport or expose IPC queue frames.

Service handles carry a slot and generation. Starts require running
dependencies, stops reject active dependents, and restarts include the running
dependent closure. Heartbeat failures continue to use the existing graph-wide
supervisor recovery path.

Filesystem-loaded service packages are not targeted by this boundary's image reset path. Their
manager Start/Restart operations return an explicit unsupported result until bounded durable
reloading exists; graph-wide supervisor recovery remains package-aware.

## Consequences

- Terminal commands are a text adapter over the structured manager API.
- A future GUI shell can use the same ABI and receive the same capability grant.
- Service IPC endpoints remain fixed; runtime endpoint creation and arbitrary
  capability delegation remain deferred.
- Lifecycle state is bounded and host-testable without adding an allocator or
  runtime dependency.
