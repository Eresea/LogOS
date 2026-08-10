# Test Status

> **Last verification:** 2026-08-10 from starting SHA `0a5a02c`; working-tree changes are listed
> below.

This file is the current evidence ledger. Historical totals, superseded failures, and earlier
milestone claims are in [the reviewed status record](reviewed/STATUS-history-2026-08.md).

## Current evidence

- `scripts/check.ps1 -Stage host` passed: formatting, clippy, host tests, architecture, documentation
  links, ADR index checks, and focused host tests. The changed crates also pass `logos-core` (12),
  `logos-net` (17), and `logos-network-service` (4) tests.
- `scripts/check.ps1 -Stage uefi` and `-Stage uefi -Release` passed; all six images built.
- Headless boot reached `startup self check passed`, `check network typed endpoints passed`, and
  `native terminal active`.
- Fresh `cargo run -p logos-test -- suite network` passed all 12 Network scenarios, including
  `network/simultaneous-client-busy` and `network/tcp-stream`; the latter verified accepted and
  acknowledged stream watermarks and exact peer bytes.
- The TCP foundation now has 17 `logos-net` tests covering handshake, data/ACK arithmetic, duplicate
  ACKs, bounded retransmission, FIN/CloseWait, RST, and bounded stream TX occupancy.
- Fresh Remote auth and typed-invoke runs reach enrollment and Gateway startup, then fail before a
  host reply: auth-denied reaches the gate-denial/transport-close path, while typed-invoke produces
  no Remote gate request after enrollment. Five Remote proofs remain explicitly skipped/unimplemented:
  enrollment persistence, reconnect replay, pending-after-reset, Gateway restart, and protected-state
  corruption. Their permanent IDs remain registered.
- The direct `transport-dhcp` and `configuration` baselines assert typed configuration; raw DHCP
  Discover/Offer/Request/Ack orchestration is not current baseline evidence.

## Verification limits

The configured UEFI target checks pass. A generic `cargo check --workspace` is unsuitable for this
no-std UEFI workspace without its configured panic strategy. QEMU Network runs are deterministic and
hermetic; an environment-level `network_resources_unavailable` result is not a passing proof.

Completed proof IDs remain regression contracts. Proofs must assert structured state or exact bytes,
not diagnostic strings.
