# ADR-0046: Storage service lifecycle

- Status: Accepted
- Date: 2026-08-15

## Decision

Storage is admitted as the sixth fixed ring-3 service. Its image is loaded,
mapped, supervised, restarted, and staged at `\\EFI\\LOGOS\\STORAGE.ELF`
through the same bounded lifecycle as the terminal services. The image emits
heartbeats and waits on the scheduler until the kernel-mediated storage
request endpoint is enabled.

The storage format, journal, namespace, and IPC adapter remain owned by the
`logos-storage-service` package. The image does not receive PCI, MMIO,
interrupt, queue, or DMA access, and no path or namespace state is added to
Core.

## Consequences

- Service-count bounds are six everywhere in image admission and supervision.
- Storage lifecycle can be proved independently of hardware availability.
- Functional storage requests remain gated on a later endpoint proof; the
  current boot image is deliberately not a fake persistence path.
