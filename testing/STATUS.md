# Test Status

> **Last verification:** 2026-08-10 at repository `HEAD` `ee0df0693d64d359ea9d1e5f454a5ab02425d6ed`,
> with the uncommitted Gateway TX staging changes present in the resulting tree.

This file is the current evidence ledger. Historical totals, superseded failures, and earlier
milestone claims are in [the reviewed status record](reviewed/STATUS-history-2026-08.md).

## Current evidence

- Current local boundary-refactor validation passed `scripts/check.ps1 -Stage host` and
  `scripts/check.ps1 -Stage uefi`. A fresh `network/tcp-stream` run did not produce a structured
  boot report: two fresh runs stalled after Storage startup and timed out (latest seed
  `1786391407555509400`; retained artifact `target/logos-test/run-1786391407555257500`). It is not
  passing evidence for this dirty tree.
- `scripts/check.ps1 -Stage host` passed: formatting, clippy, host tests, architecture, documentation
  links, ADR index checks, and focused host tests. The changed crates also pass `logos-core` (12),
  `logos-net` (17), and `logos-network-service` (5) tests, including
  `staged_tcp_tx_transfers_ownership_once`.
- `scripts/check.ps1 -Stage uefi` and `-Stage uefi -Release` passed; all six images built.
- Headless boot reached `startup self check passed`, `check network typed endpoints passed`, and
  `native terminal active`.
- Fresh `cargo run -p logos-test -- run network/tcp-stream` passed; fresh
  `cargo run -p logos-test -- run network/simultaneous-client-busy` passed; and fresh
  `cargo run -p logos-test -- suite network` passed all 12 Network scenarios (seed
  `1786380932610535800`), including the two named proofs. `network/tcp-stream` verified accepted and
  acknowledged stream watermarks and exact peer bytes.
- The scalable TCP service now makes staged `TcpTx` ownership explicit: rejected device submission
  retains the frame, accepted submission clears it exactly once, and inbound/timer/legacy `Write`
  paths do not replace an already staged frame. The regression is
  `staged_tcp_tx_transfers_ownership_once`; it does not rely on peer byte-stream duplicate
  suppression.
- `remote/crypto-kat` passed in QEMU UEFI (seed `1786388210976619800`). It runs RFC 7748
  X25519 public-key/DH vectors, RFC 8439 ChaCha20-Poly1305 encrypt/decrypt vectors, SHA-256,
  and the fixed `Noise_IK_25519_ChaChaPoly_SHA256` transcript through `RemoteResponder`, including
  second-message and transport authentication. The same `logos-remote` KAT passes on the host.
  The target-scoped Fiat Curve25519 and software ChaCha20/Poly1305 cfgs are recognized by the
  locked dependency versions and do not produce a cryptographic divergence.
- The TCP foundation now has 17 `logos-net` tests covering handshake, data/ACK arithmetic, duplicate
  ACKs, bounded retransmission, FIN/CloseWait, RST, and bounded stream TX occupancy.
- Fresh `cargo run -p logos-test -- run remote/auth-denied` failed after the semantic authentication
  rejection with `timeout waiting for QEMU exit` (seed `1786388432816371600`). Fresh
  `cargo run -p logos-test -- run remote/typed-invoke` still fails before the post-enrollment
  `ADVANCE` reply (seed `1786389989859588400`); the Gateway writable-wait path is not reached.
  The isolated guest proof rules out the UEFI cryptographic implementation and target crypto cfgs
  as that failure's cause; these Remote proofs are not passing evidence for this tree. Five Remote
  proofs remain explicitly skipped/unimplemented: enrollment persistence, reconnect replay,
  pending-after-reset, Gateway restart, and protected-state corruption. Their permanent IDs remain
  registered.
- The direct `transport-dhcp` and `configuration` baselines assert typed configuration; raw DHCP
  Discover/Offer/Request/Ack orchestration is not current baseline evidence.

## Verification limits

The configured UEFI target checks pass. A generic `cargo check --workspace` is unsuitable for this
no-std UEFI workspace without its configured panic strategy. QEMU Network runs are deterministic and
hermetic; an environment-level `network_resources_unavailable` result is not a passing proof.

Completed proof IDs remain regression contracts. Proofs must assert structured state or exact bytes,
not diagnostic strings.
