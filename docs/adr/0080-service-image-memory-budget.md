# ADR-0080: Service image memory budget

## Status

Accepted

## Decision

Raise the shared service ELF and aggregate load-plan budget from 512 KiB to
2048 KiB. The value remains an absolute runtime safety ceiling for UEFI
retention and process admission; service images should remain materially below
it where practical.

## Consequences

- The retained UEFI allocation and bounded loader page tables admit the larger
  Display image and future GPU-facing service code.
- Malformed or unexpectedly large images remain rejected before process launch.
- The larger ceiling increases the maximum possible per-service resource
  reservation, so image-size reduction remains the preferred optimization.
