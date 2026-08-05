# Test Status

Last verification: 2026-08-05. Architecture is frozen for this verification pass.

- Catalog: 83 proofs; 52 ready; 31 intentionally skipped. IDs and readiness states are unchanged.
- Host checks: `scripts/check.ps1 -Stage host` passed; 14 ABI, 6 Core, 12 Remote, 3 Store,
  5 Terminal, and 5 Test-host tests passed.
- UEFI checks: `scripts/check.ps1 -Stage uefi` passed for the kernel and all six service images.
- Platform suite: 16 passed, 16 intentionally skipped, 0 failed. Terminal, Sessions, Store, and
  Network panic/fault containment passed.
- Persistence proofs: storage replacement, block read/flush, terminal history, and block
  timeout/reset passed. The suite later failed at `persistence/capability-denied` and timed out
  before completing the remaining scenarios.
- Console suite: 11 passed, 1 failed (`console/input-capability-denied`), 3 skipped. The failure
  is a reset/restart regression and is a bug until the suite is green.
- Network suite: DHCP and configuration passed; `network/device-bind`, `network/timeout`, and
  `network/reset-reconnect` failed or timed out. These are required-set bugs. Remaining network
  transport proofs also did not complete before the bounded suite run timed out.
- Headless boot: `scripts/run.ps1 -Headless` reached kernel entry, all startup self-checks,
  storage recovery, DHCP, and `native terminal active`; the intentionally bounded 20-second run
  ended with QEMU still in its interactive loop.

Required pass set: **FAIL**. The failing required network device/bind and device timeout/reset
proofs block the architecture freeze from being declared green.

Skipped proofs remain permanent IDs and were not removed.
