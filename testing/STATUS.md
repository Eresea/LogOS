# Test Status

Last verification: `cargo run -p logos-test -- suite main` (2026-08-04).

- Catalog: 83 proofs; 52 ready; 31 intentionally skipped. The pre-refactor and current
  `cargo run -p logos-test -- list` outputs have the same IDs and readiness states.
- Main: 33 passed, 19 failed, 31 skipped. All startup and boot-health checks passed. The failures
  are the registered network/remote semantic paths that currently time out or report unavailable,
  plus `persistence/terminal-history` (`read terminal history: NotFound`) and
  `persistence/block-timeout-reset` (timeout waiting for `RESULT input=accepted`).
- Persistence: 6 passed, 2 failed (the two failures above).
- Network: 3 passed, 7 failed; DHCP, configuration, and unauthorized-operation passed. Device-bind
  and packet/transport proofs timed out waiting for QEMU exit.
- Remote: 0 passed, 8 failed; all registered remote proofs timed out waiting for QEMU/peer completion.
- Boot script: `scripts/run.ps1 -Headless` reached kernel entry, all startup self-checks, storage
  recovery, and DHCP; the intentionally bounded 90-second command timed out while QEMU remained in
  its interactive run loop.
- Post-refactor smoke proofs: `core/boot-normal` and `network/configuration` passed.

Portable tests and target checks are recorded in the change report. The skipped proofs remain
permanent IDs and are not removed by this refactor.
