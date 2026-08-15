# ADR-0048: Fail-closed storage format compatibility

- Status: Accepted
- Date: 2026-08-16

## Context

The storage journal is persistent across kernel and service reboots. A newer or damaged on-disk
version must not be interpreted as the current format, silently rolled back to an older superblock,
or mistaken for an incomplete tail. Those behaviors can make committed files appear to disappear.

## Decision

- Format version 1 is the only version accepted by the current storage implementation.
- An unknown version in either superblock is reported as `UnsupportedVersion`; the other superblock
  is not selected as a silent fallback.
- An unknown journal-record version is reported as `UnsupportedVersion`, even when it is at the
  journal tail. The current kernel never reformats nonblank media.
- For version 1, a blank or incomplete journal gap abandons only its incomplete transaction. Later
  checksummed, sequence-valid committed transactions are replayed and preserved.
- Future format versions require an explicit migration or reader implementation and their own
  reboot/recovery proof before being accepted.

## Consequences

Known v1 files survive repeated reopen and torn-tail recovery without silent rollback. Unsupported
media fails visibly and preserves its bytes for an explicit migration tool or newer kernel. The
system does not promise to read arbitrary future formats automatically; compatibility is deliberate,
versioned, and proof-backed.
