# ADR-0046: Storage service lifecycle

- Status: Accepted
- Date: 2026-08-15

## Decision

Storage is admitted as the sixth fixed ring-3 service. Its image is loaded,
mapped, supervised, restarted, and staged at `\\EFI\\LOGOS\\STORAGE.ELF`
through the same bounded lifecycle as the terminal services. The image emits
heartbeats and exercises the kernel-mediated storage request endpoint.

The storage format, journal, namespace, and IPC adapter remain owned by the
`logos-storage-service` package. The image does not receive PCI, MMIO,
interrupt, queue, or DMA access, and no path or namespace state is added to
Core. Its fixed user stack is larger than the general service stack so bounded
journal replay scratch remains outside Core and does not require an allocator.

## Consequences

- Service-count bounds are six everywhere in image admission and supervision.
- Storage lifecycle can be proved independently of hardware availability.
- The endpoint now drives the bounded format/journal namespace path; the
  remaining persistence proof requires QEMU and a real VirtIO disk.
