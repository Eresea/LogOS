# Test Status

Last run: `cargo run -p logos-test -- suite main`  
Seed: `1785531673265228300`  
Result: **4 failed, 1 passed, 39 skipped**

| Test | Status | Detail |
| --- | --- | --- |
| console/structured-command | failed | initial reset failed: timeout waiting for `LOGOS/1 RESULT reset=accepted` |
| core/boot-normal | failed | initial reset failed: timeout waiting for `LOGOS/1 RESULT reset=accepted` |
| persistence/block-read-flush | failed | timeout waiting for `LogOS: storage formatted` |
| persistence/terminal-history | failed | timeout waiting for `LogOS: storage formatted` |
| persistence/capability-denied | passed | |
| core/boot-recovery | skipped | semantic proof unavailable |
| core/ipc-request-reply | skipped | semantic proof unavailable |
| core/ipc-cancellation | skipped | semantic proof unavailable |
| core/task-block-wake | skipped | semantic proof unavailable |
| core/capability-denied | skipped | semantic proof unavailable |
| core/capability-revoked | skipped | semantic proof unavailable |
| core/driver-reset-recovery | skipped | semantic proof unavailable |
| core/resource-reclamation | skipped | semantic proof unavailable |
| core/panic-diagnostics | skipped | semantic proof unavailable |
| console/input-qwerty | skipped | semantic proof unavailable |
| console/input-azerty | skipped | semantic proof unavailable |
| console/editing-utf8 | skipped | semantic proof unavailable |
| console/history | skipped | semantic proof unavailable |
| console/display-restart | skipped | semantic proof unavailable |
| console/recovery-handoff | skipped | semantic proof unavailable |
| platform/manifest-valid | skipped | semantic proof unavailable |
| platform/manifest-invalid | skipped | semantic proof unavailable |
| platform/dependency-order | skipped | semantic proof unavailable |
| platform/dependency-cycle-rejected | skipped | semantic proof unavailable |
| platform/startup-failure | skipped | semantic proof unavailable |
| platform/dependency-loss | skipped | semantic proof unavailable |
| platform/resource-reclamation | skipped | semantic proof unavailable |
| platform/protocol-compatible | skipped | semantic proof unavailable |
| platform/protocol-incompatible | skipped | semantic proof unavailable |
| platform/unauthorized-capability | skipped | semantic proof unavailable |
| platform/diagnostics | skipped | semantic proof unavailable |
| platform/native-payload-staged | skipped | semantic proof unavailable |
| platform/service-address-space | skipped | semantic proof unavailable |
| platform/native-image-mapped | skipped | semantic proof unavailable |
| platform/service-privilege-setup | skipped | semantic proof unavailable |
| platform/service-ring3-transition | skipped | semantic proof unavailable |
| persistence/write-interruption | skipped | semantic proof unavailable |
| persistence/recovery | skipped | semantic proof unavailable |
| persistence/capability-denied | skipped | semantic proof unavailable |
| persistence/corruption-detected | skipped | semantic proof unavailable |
| network/packet-loss | skipped | semantic proof unavailable |
| network/timeout | skipped | semantic proof unavailable |
| network/reset-reconnect | skipped | semantic proof unavailable |
| network/unauthorized-operation | skipped | semantic proof unavailable |

Artifacts: `target/logos-test/main-1785531673264947000`.
