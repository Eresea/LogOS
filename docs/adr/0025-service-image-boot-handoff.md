# ADR-0025: Service image boot handoff

## Status

Accepted

## Decision

The UEFI boot path loads all five manifest images before
`ExitBootServices`. It stages them from the image ESP, validates them as
fixed-address ELF executables, retains their page allocations in
`ServiceImageBundle`, and publishes a proof marker before leaving firmware.

The bundle is metadata-only after the handoff. The static ring-3 proof remains
the scheduler workload until page-table construction and service startup
consume these retained images.

## Consequences

- Missing or invalid service artifacts fail early through the normal fatal path.
- QEMU stages the same artifacts used by the firmware reader, preventing a
  proof-only shortcut.
- Existing kernel boot and scheduler proofs remain available while real
  service execution is implemented.
