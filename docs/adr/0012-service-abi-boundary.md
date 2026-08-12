# ADR-0012: Shared service ABI boundary

## Status

Accepted

## Decision

The kernel and terminal services exchange only fixed `repr(C)` values from the
`logos-abi` crate. Service-to-service terminal traffic uses bounded,
generation-stamped shared IPC rings. Kernel/resource operations use explicit
capability-gated syscall kinds and never carry Rust references or unvalidated
user pointers.

The existing `terminal_abi` module remains a kernel compatibility re-export
while the service crates are introduced. It is not a second ABI.

## Consequences

- ABI values can be tested without the UEFI kernel dependency.
- Message size, service count, image size, framebuffer size, and resource
  bounds are visible in one no-allocator crate.
- Adding a service operation requires an explicit wire shape and review of its
  ownership and capability requirements.
- The ABI does not provide serialization, allocation, dynamic discovery, or
  general-purpose syscall pointers.
