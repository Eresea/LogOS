# ADR-0015: Service control plane

## Status

Accepted

## Decision

Service lifecycle, process, endpoint, and resource operations use fixed
`SyscallRequest`/`SyscallResponse` values. A request carries scalar arguments
and one typed capability; it never carries a Rust reference or implicit user
pointer. The kernel validates service identity, endpoint generation, resource
kind, and capability ownership before dispatch.

Terminal streams continue to use shared bounded IPC rings. Syscalls create,
map, signal, wait, start, reap, or map resources; they do not become a second
message transport.

## Consequences

- Resource failures are explicit and testable before hardware dispatch exists.
- A restarted service cannot reuse its predecessor's capabilities.
- The current syscall model is authorization-only; concrete page-table and
  scheduler effects remain in the loader milestone.
