# ADR-0023: Retained service images

## Status

Accepted

## Decision

Before `ExitBootServices`, each manifest image is validated as a bounded ELF
and recorded as a page-aligned physical location. `ServiceImageBundle` owns
five fixed records and retains only identity, byte length, and page allocation
length after the firmware reader is gone.

The UEFI filesystem adapter and page allocator will populate these records in a
later boot slice. The bundle does not own a filesystem handle, allocator, or
service process.

## Consequences

- Post-UEFI code has no dependency on file-system protocol lifetimes.
- Duplicate, malformed, oversized, unaligned, and overflowing images are
  rejected before process admission.
- Physical-page reclamation remains coupled to process/address-space teardown,
  not to the filesystem reader.
