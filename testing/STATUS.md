# Test Status

Last verification: 2026-08-06. Stabilization passed for the completed typed layers; ABI v4
restructuring remains closed while the bounded Network-client and Remote migrations remain.

- Catalog: 83 proofs; 52 ready; 31 intentionally skipped. IDs and readiness states are unchanged.
- Host checks: `scripts/check.ps1 -Stage host` passed; 14 ABI, 6 Core, 12 Remote, 3 Store,
  5 Terminal, and 5 Test-host tests passed.
- UEFI static checks: target-scoped kernel clippy and the Gateway UEFI check passed.
- QEMU/OVMF main suite, fixed seed `1`: 43 passed, 12 migration-deferred failures, and 28
  intentionally skipped. The repaired shared-fixture harness now renews its existing timeout per
  operation; all completed Core, Console, Platform, Persistence, and Network transport proofs pass.

Required host pass set: **PASS** for the ABI-v4 boundary:
`Input/Display → Sessions/Effects → Store/Block → Network device/event`.

Deferred Network-client IDs: `network/icmp-echo`, `network/udp-round-trip`,
`network/backpressure-cancel`, `network/packet-loss`, and `network/tcp-stream`. Deferred Remote
IDs: all seven registered `remote/*` proofs. They are registered regression contracts and remain
open for their bounded migration cycles; no IDs were removed or marked passed.

Skipped proofs remain permanent IDs and were not removed.
