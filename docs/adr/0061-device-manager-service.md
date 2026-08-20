# ADR-0061: Bounded device manager service

## Status

Accepted

## Decision

Add a ninth fixed `Device` service with one bounded Flow operation, `device.list()`. The service
owns the returned inventory records and exposes only typed, generation-checked IPC. Core remains
the owner of PCI and VirtIO mechanics and publishes the currently supported block device as a
bounded disk record.

Formatting and filesystem recreation are deliberately deferred. They require a separate destructive
operation contract, authorization, media identity checks, and Storage coordination; `device.list()`
must not imply or trigger any mutation.

## Consequences

- The service manifest and IPC graph gain one fixed service and two Flow-facing endpoints.
- Device records are fixed-size and capped at eight entries, with bounded names and block geometry.
- Flow receives a bounded human-readable inventory result and no direct hardware access.
- Future format/recreate work must add an explicit ABI operation and proof for authorization,
  cancellation, stale media rejection, and durable Storage reinitialization.
