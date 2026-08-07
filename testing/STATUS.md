# Test Status

Last verification: 2026-08-07. Starting SHA: `98c67bc5a791d0caef71ca3769933a1c38208634`.
Typed Network-client transport implementation and focused proofs are present; full serial Network
and Remote closure remains open. The current working-tree verification was intentionally paused
before ABI-v4 freeze.

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
- Catalog: 84 proofs; 53 ready; 31 intentionally skipped. No proof IDs were removed, renamed,
  weakened, or newly skipped.
- Fixed seed: `LOGOS_TEST_SEED=1`, one QEMU job.

Current task checkpoint: host checks passed before the final RX-buffer adjustment; individual
fixed-seed transport and packet-loss runs passed. The second full serial Network rerun was stopped
after intermittent RX-ring exhaustion remained under investigation. No Remote suite or clean-tree
verification is claimed here.

Per-suite totals:

| Suite | Passed | Failed | Skipped |
| --- | ---: | ---: | ---: |
| core | 1 | 0 | 9 |
| console | 12 | 0 | 3 |
| platform | 16 | 0 | 16 |
| persistence | 8 | 0 | 0 |
| network | 7 | 4 | 0 |
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

Remaining Network-client failures:

- `network/icmp-echo`: request completion timeout; kernel reports `network client response
  timeout`, harness times out waiting for the passed result.
- `network/udp-round-trip`: same Network-client response-state timeout.
- `network/backpressure-cancel`: same Network-client response-state timeout.
- `network/packet-loss`: same Network-client response-state timeout.
- `network/tcp-stream`: Gateway startup signal is absent; classified at the Network/Remote
  Gateway startup boundary.

Remaining Remote failures:

- `remote/enrollment-persistence`, `remote/auth-denied`, `remote/typed-invoke`,
  `remote/reconnect-replay`, `remote/pending-after-reset`, `remote/gateway-restart`, and
  `remote/protected-state-corrupt`: all time out waiting for `LogOS: Gateway started`.
  Classified as Gateway startup/Remote coordination behavior, not missing typed transport.

The remaining failures are implementation-boundary behavior in Network-client/Gateway/Remote
coordination. They are not missing typed pages or a request to expand the ABI. Typed Network-client,
Network-server, and Remote pages and their canonical mappings are present; ABI v4 is not yet frozen.
