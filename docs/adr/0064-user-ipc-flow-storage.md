# ADR-0064: User service IPC and Storage catalog transport

Status: Accepted

## Context

The User policy core and the v5 Storage catalog boundary exist independently, but the boot graph
needs a live User service without exposing Storage paths or raw blocks to it. Flow also needs a typed
shell-facing route for identity operations.

## Decision

- Add bounded stream edges Flow↔User and User↔Storage to the fixed service graph.
- Flow sends `UserRequest` values wrapped in the existing fixed `IpcBytes` envelope; User returns
  `UserResponse` values with the original request identity.
- User loads and saves the canonical snapshot through fixed-size chunk messages. Storage owns the
  system-pool transaction, snapshot buffer, flush, and commit; User never receives a path or block
  capability.
- User sessions and capabilities remain in the User image and are recreated after restart. Flow
  clears its cached handles on logout or targeted capability revocation.
- Flow exposes only bounded `user.*` operations; namespace file access remains a separate
  capability-backed boundary.

## Consequences

The graph grows by four stream queues and two capability slots on Flow and Storage. Endpoint 31 has
no separate write-edge event bit because the syscall event mask is a u64; its bounded producer path
retries while all read edges and the keyboard event remain distinct. The User image carries one
fixed Argon2id workspace and no kernel-wide allocator is introduced.
Core maps that workspace as a bounded per-page service region; the standard QEMU proofs use
256 MiB so the fixed 64 MiB KDF requirement coexists with the ten-service graph and firmware
reservations.

## Verification

Host ABI, Flow, User, Core, service-image, and QEMU service/storage proofs validate the fixed graph,
typed command parsing, snapshot chunk bounds, v5 persistence, and restart-safe volatile handles.
