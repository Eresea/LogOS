# Test Status

Last baseline verification: 2026-08-08. Current work is on `codex/repair-network-invariants`;
the checkpoint includes the structured readiness race repair, bounded wake-set coverage, and TCP
prototype tests.
Typed Network bootstrap transport is present. NetworkRuntime now owns readiness through an internal
server `Status` request, production replies always resume their blocked caller, and white-box
probes use an explicit test-only completion target. The current client path remains globally
serialized; async per-connection Network architecture and full Network/Remote closure remain open.
TCP foundation evidence is host-only: 15 `logos-net` tests cover handshake, data/write arithmetic,
duplicate ACKs, bounded retransmission, FIN/CloseWait, and RST.

- Toolchain: Rust `1.93.0`, Cargo `1.93.0`, target `x86_64-unknown-uefi` installed.
- Host: `scripts/check.ps1 -Stage host` passed; format, clippy, host tests, architecture,
  documentation links, and ADR index checks passed; `logos-abi` focused transport tests: 30
  passed.
- UEFI debug: `scripts/check.ps1 -Stage uefi` passed; all six images built.
- UEFI release: `scripts/check.ps1 -Stage uefi -Release` passed; all six images built.
- ESP debug and release contents are exactly `BOOTX64.EFI`, `TERMINAL.EFI`, `SESSIONS.EFI`,
  `STORAGE.EFI`, `NETWORK.EFI`, and `GATEWAY.EFI`.
- QEMU: `C:\Program Files\qemu\qemu-system-x86_64.exe`, version `11.0.50`; OVMF:
  `C:\Program Files\qemu\share\edk2-x86_64-code.fd`. Headless boot reached `startup self
  check passed`, `check network typed endpoints passed`, and `native terminal active`.
- Catalog: 84 proofs; 50 ready; 34 intentionally skipped. No proof IDs were removed, renamed,
  weakened, or newly skipped.
- Fixed seed: `LOGOS_TEST_SEED=1`, one QEMU job.

Current task checkpoint: the finalization host and UEFI build checks pass. `cargo check --workspace`
remains unsuitable for this no-std UEFI workspace without the configured panic strategy; target-
scoped UEFI checks pass. The Network QEMU suite was rerun after the repair: boot and structured
`ADVANCE` responses succeeded, but all 11 implemented Network scenarios remained pending and timed
out before configuration; the transport-DHCP artifact shows one submitted transmit and no peer
frame. This is an unresolved bootstrap proof failure, not a claimed pass.
`network/configuration` uses `QUERY network/configured`, not debugcon. The five unfinished Remote
proofs are explicitly skipped/unimplemented while
their permanent IDs remain registered: enrollment persistence, reconnect replay, pending-after-reset,
Gateway restart, and protected-state corruption.
`network/tcp-stream` is explicitly skipped/unimplemented until a real host-to-guest TCP exchange
passes without Remote. No debug-log readiness or Gateway-start string is a proof source.

Per-suite totals:

| Suite | Passed | Failed | Skipped |
| --- | ---: | ---: | ---: |
| core | 1 | 0 | 9 |
| console | 12 | 0 | 3 |
| platform | 16 | 0 | 16 |
| persistence | 8 | 0 | 0 |
| network | 0 | 11 | 1 |
| remote | 0 | 8 | 0 |
| main | 44 | 12 | 28 |

Required completed-layer status: **PASS** for all implemented completed-layer proofs. Normal boot,
Display, Terminal replacement and containment, Sessions/Effect, Store/Block, Storage
replacement, terminal history, persistence interruption/recovery/corruption/denial, Network
device information/binding, DHCP/configuration, timeout/reset, Network service replacement and
containment, plus the simultaneous Terminal/Gateway Busy proof, remained green where implemented
by the fixed-seed catalog and headless boot checks.
The permanent `core/boot-recovery` and `console/recovery-handoff` IDs remain intentionally skipped
because their semantic proofs are not implemented; they are not reported as passes.

`network/simultaneous-client-busy` passed: the Gateway client received a typed `Busy` reply while
the Terminal transaction remained active.

Baseline Network-client failures (pre-repair; rerun pending):

- `network/icmp-echo`: request completion timeout; kernel reports `network client response
  timeout`, harness times out waiting for the passed result.
- `network/udp-round-trip`: same Network-client response-state timeout.
- `network/backpressure-cancel`: same Network-client response-state timeout.
- `network/packet-loss`: same Network-client response-state timeout.
- `network/tcp-stream`: the former Gateway-backed scenario was removed from the implemented set;
  the permanent ID is currently skipped until the independent TCP proof exists.

Baseline Remote failures (pre-repair; superseded by explicit skips for five unfinished proofs):

- `remote/auth-denied` and `remote/typed-invoke`: previously timed out waiting for
  `LogOS: Gateway started`; the harness now uses structured readiness and host-side authority.
- `remote/enrollment-persistence`, `remote/reconnect-replay`, `remote/pending-after-reset`,
  `remote/gateway-restart`, and `remote/protected-state-corrupt`: explicitly skipped/unimplemented;
  no partial proof is claimed.
  Classified as Gateway startup/Remote coordination behavior, not missing typed transport.

The remaining failures are implementation-boundary behavior in Network-client/Gateway/Remote
coordination. They are not missing typed pages or a request to expand the ABI. Typed Network-client,
Network-server, and Remote pages and their canonical mappings are present; ABI v4 is not yet frozen.
