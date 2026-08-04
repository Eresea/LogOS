# ADR-0021: Complete the ABI-v4 Input and Display page migration

- Status: Accepted
- Date: 2026-08-04

## Context

ABI v4 defined typed endpoint page shapes, but the native Terminal still used the shared control
page for keyboard and display payload. That left replacement identity dependent on a context-page
address and allowed the old payload path to remain active.

## Decision

`ControlPage` is the lifecycle/notification header. Terminal receives independently mapped
`InputPage` and `DisplayPage` pages. Core writes the endpoint generation when it creates or replaces
the task, and every endpoint operation validates that generation before reading or writing data.
The address space records the endpoint pages as ordinary owned mappings, so `AddressSpace::release`
reclaims them with the task. No endpoint registry, adapter, serializer, or generic framework is
introduced.

### State transitions

| Endpoint | Valid service transition | Valid Core transition | Malformed or stale input | Reset/replacement |
| --- | --- | --- | --- | --- |
| Input | `Ready -> Waiting` (`InputPage::wait_at`), `Reply -> Ready` (`take_at`) | `Waiting -> Reply` (`deliver_at`) | reject with no write | reset to `Ready` with the new generation |
| Display | `Ready -> Request` (`request_*`), `Complete -> Ready` (`finish_at`) | `Request -> Complete` (`complete_at`) | reject with no write | reset to `Ready` with the new generation |

Unknown scalar states, out-of-range text, invalid colors, and generation mismatches are rejected.
An old handle therefore cannot deliver input, complete a display request, or observe a replacement
page even when physical page allocation reuses the same address.

## Consequences

- Terminal input and display payload no longer live in `ControlPage`.
- The native task handle carries the typed page address and generation; the control address remains
  available for lifecycle and non-migrated protocols.
- Session, Store, Block, Network, and Remote retain the bounded generic payload area until their
  own migration; they are outside this tranche.
- Later endpoint migrations should copy this concrete pattern: a fixed page, scalar validation,
  generation check, deterministic reset, and ownership through `AddressSpace::release`.
