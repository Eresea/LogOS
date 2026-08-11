# ADR-0004: Isolate each native service in its own address space

- Status: Accepted
- Date: 2026-07-28

## Context

Platform v1 must run `logos-terminal` outside the kernel while preserving capability enforcement.
An in-process or shared-address-space native service could read or modify kernel memory directly,
which bypasses that enforcement.

## Decision

Each native service runs in its own address space. Core owns service page tables, kernel mappings,
entry/return transitions, and teardown. A service receives only explicitly mapped memory for its
code, stack, IPC buffers, and granted capability endpoints.

## Consequences

- The native loader needs PE section mapping, relocation, a per-service stack, and a privilege
  transition before `logos-terminal` can execute from its staged payload.
- Input, display, and session contracts cross IPC; raw framebuffer, PS/2, and kernel pointers do
  not cross the boundary.
- QEMU must prove that an invalid access faults the service and leaves the recovery console alive.

## Alternatives considered

- Shared kernel address space -- rejected because it makes native-service capabilities advisory.
- Language-only isolation -- rejected because it does not protect against memory-unsafe code or a
  compromised service.
