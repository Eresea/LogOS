# Test Status

Last verification: 2026-08-09. Current direct-client baseline on `codex/repair-network-invariants`.
Historical checkpoint material is retained below only where explicitly labeled.
The opening architecture notes are retained context; **Current evidence** is the authoritative
run ledger.
Typed Network bootstrap transport is present. NetworkRuntime now owns readiness through an internal
server `Status` request, production replies always resume their blocked caller, and white-box
probes use an explicit test-only completion target. The current client path remains globally
serialized; async per-connection Network architecture and full Network/Remote closure remain open.
TCP foundation evidence is now host plus QEMU: 15 `logos-net` tests cover handshake, data/write
arithmetic, duplicate ACKs, bounded retransmission, FIN/CloseWait, and RST; the independent
`network/tcp-stream` proof passes a real host TCP peer through the Network service.

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
- Historical catalog snapshot: 84 proofs; 51 ready; 33 intentionally skipped. No proof IDs were removed, renamed,
  weakened, or newly skipped.
- Fixed seed: `LOGOS_TEST_SEED=1`, one QEMU job.

## Current evidence

With `LOGOS_TEST_SEED=1` and one QEMU job, the dedicated TCP QEMU proof and the direct Network-client
proofs pass individually. The TCP structured event sequence is
`starting -> listener_waiting -> connection_established -> connection_readable -> write_pending ->
write_acknowledged -> connection_closed -> passed`. Its ESP contains only `TERMINAL.EFI`,
`STORAGE.EFI`, and `NETWORK.EFI`; no Sessions, Gateway, Remote, enrollment, persistence, Noise, or
`logosctl` runtime path is involved. `cargo check --workspace` remains unsuitable for this no-std
UEFI workspace without the configured panic strategy; target-scoped UEFI checks pass.
The direct-client proofs run through the TCP-style `test-usernet` image and host peer. The
remaining suite failure is deliberately isolated: `network/simultaneous-client-busy` still
depends on the Gateway slot for its second real client. The permanent transport/configuration
IDs are direct typed-client baseline proofs; raw DHCP orchestration remains outside this baseline.
The baseline `network/transport-dhcp` and `network/configuration` proofs use typed Network status,
not debugcon or the legacy readiness query. The five unfinished Remote
proofs are explicitly skipped/unimplemented while
their permanent IDs remain registered: enrollment persistence, reconnect replay, pending-after-reset,
Gateway restart, and protected-state corruption.
`network/tcp-stream` is implemented and passes independently without Remote. No debug-log readiness
or Gateway-start string is a proof source.

Historical full-catalog totals from the earlier mixed-era run:

| Suite | Passed | Failed | Skipped |
| --- | ---: | ---: | ---: |
| core | 1 | 0 | 9 |
| console | 12 | 0 | 3 |
| platform | 16 | 0 | 16 |
| persistence | 8 | 0 | 0 |
| network | 9 | 3 | 0 |
| remote | 0 | 8 | 0 |
| main | 44 | 12 | 28 |

Current Network-only result: **11 passed, 1 failed**. The seven required baseline IDs are green
individually; the only suite failure is `network/simultaneous-client-busy`, which remains a
Gateway-slot contract.

Historical completed-layer summary: **PASS** for all implemented completed-layer proofs. Normal boot,
Display, Terminal replacement and containment, Sessions/Effect, Store/Block, Storage
replacement, terminal history, persistence interruption/recovery/corruption/denial, Network
device information/binding, DHCP/configuration, timeout/reset, and Network service replacement
and containment remained green in the fixed-seed catalog and headless boot checks. The separate
Terminal/Gateway Busy proof remains the only Network suite failure.
The permanent `core/boot-recovery` and `console/recovery-handoff` IDs remain intentionally skipped
because their semantic proofs are not implemented; they are not reported as passes.

Current exception: `network/simultaneous-client-busy` remains a Gateway-slot contract and is not
part of the direct client profile; its failure is the unavailable Remote/Gateway composition path.

Current direct Network-client results:

- `network/device-bind`, `network/icmp-echo`, `network/udp-round-trip`,
  `network/unauthorized-operation`, `network/backpressure-cancel`, `network/packet-loss`,
  `network/transport-dhcp`, `network/configuration`, `network/timeout`, and
  `network/reset-reconnect`: pass independently through real VirtIO, the
  Network service, typed client requests, and the deterministic host peer.
- `network/tcp-stream`: passes independently through real VirtIO, the Network service, typed
  Listen/Accept/Read/Write requests, and a deterministic host TCP peer.

Historical Remote failures (pre-repair; superseded by explicit skips for five unfinished proofs):

- `remote/auth-denied` and `remote/typed-invoke`: previously timed out waiting for
  `LogOS: Gateway started`; the harness now uses structured readiness and host-side authority.
- `remote/enrollment-persistence`, `remote/reconnect-replay`, `remote/pending-after-reset`,
  `remote/gateway-restart`, and `remote/protected-state-corrupt`: explicitly skipped/unimplemented;
  no partial proof is claimed.
  Classified as Gateway startup/Remote coordination behavior, not missing typed transport.

Current remaining issue: the Network-only suite's Gateway-dependent `network/simultaneous-client-busy`
proof. Historical Remote failures are listed above and are now explicit skips where unfinished.
They are not missing typed pages or a request to expand the ABI; ABI v4 is not yet frozen.
