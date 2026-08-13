# ADR-0012: Shared service ABI boundary

## Status

Accepted

## Decision

The kernel and terminal services exchange only fixed `repr(C)` values from the
`logos-abi` crate. Service-to-service terminal traffic uses bounded,
generation-stamped shared IPC rings. Kernel/resource access is established by
process admission and fixed mappings; the proof-only syscall gate does not
expose a general service control-plane ABI.

The kernel imports the `logos-abi` crate directly; the former kernel
`terminal_abi` compatibility re-export was removed once the service split was
complete.

## Consequences

- ABI values can be tested without the UEFI kernel dependency.
- Message size, service count, image size, framebuffer size, and resource
  bounds are visible in one no-allocator crate.
- Adding a service operation requires an explicit wire shape and review of its
  ownership and capability requirements.
- The ABI does not provide serialization, allocation, dynamic discovery, or
  general-purpose syscall pointers.
