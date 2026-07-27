---
name: logos-debug-qemu-failure
description: Diagnose LogOS QEMU proof failures from harness artifacts. Use for build, boot, timeout, protocol, assertion, or emulator failures.
---

# Debug a QEMU Failure

1. Read `result.json`, `command.txt`, `control.log`, `debug.log`, and `qmp.log` in the artifact directory.
2. Re-run the exact scenario with its `LOGOS_TEST_SEED`.
3. Classify build, boot, control, timeout, assertion, or OS failure.
4. Minimize the scenario; never add sleeps or delete assertions.
