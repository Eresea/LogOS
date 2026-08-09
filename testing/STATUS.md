# Test Status

> **Last verification:** 2026-08-09 from starting SHA `248dcc4`; fresh proof results below include
> the capability-table correction.

This file is the current evidence ledger. Historical totals, superseded failures, and earlier
milestone claims are in [the reviewed status record](reviewed/STATUS-history-2026-08.md).

## Current evidence

- `scripts/check.ps1 -Stage host` passed: formatting, clippy, host tests, architecture, documentation
  links, ADR index checks, and focused `logos-abi` transport tests (30 passed).
- `scripts/check.ps1 -Stage uefi` and `-Stage uefi -Release` passed; all six images built.
- Headless boot reached `startup self check passed`, `check network typed endpoints passed`, and
  `native terminal active`.
- Fresh direct Network-client runs passed through real VirtIO, the Network service, typed client
  requests, and the deterministic host peer for transport/configuration, ICMP, UDP, and
  reset/reconnect. Fresh `network/tcp-stream` stopped at `write_pending`; the result timed out
  waiting for its structured completion.
- `logos-net` has 15 TCP foundation tests covering handshake, data/ACK arithmetic, duplicate ACKs,
  bounded retransmission, FIN/CloseWait, and RST.
- The historical Network-only ledger is stale against this fresh run: in addition to the known
  `network/simultaneous-client-busy` gap, `network/tcp-stream` currently stops at `write_pending`.
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
