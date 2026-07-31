# Test Status

Last verification: `cargo run -p logos-test -- run console/structured-command`  
Seed: `1785535272568917600`  
Result: **1 passed**

| Test | Status | Detail |
| --- | --- | --- |
| console/structured-command | passed | |
| console/capability-denied | failed | timeout waiting for `LogOS: storage formatted` |
| console/input-capability-denied | failed | timeout waiting for `LogOS: storage formatted` |
| console/display-capability-denied | failed | timeout waiting for `LogOS: storage formatted` |
| console/session-capability-denied | failed | timeout waiting for `LogOS: storage formatted` |
| console/cancellation | failed | timeout waiting for `LogOS: storage formatted` |
| console/input-service-restart | failed | timeout waiting for `LogOS: storage formatted` |
| console/terminal-service-restart | failed | timeout waiting for `LogOS: storage formatted` |
| console/sessions-service-restart | failed | timeout waiting for `LogOS: storage formatted` |
| core/boot-normal | passed | |
| persistence/block-read-flush | passed | |
| persistence/terminal-history | passed | |
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
| platform/runtime-crash-restart | failed | timeout waiting for `LogOS: storage formatted` |
| platform/restart-backoff | failed | timeout waiting for `LogOS: storage formatted` |
| platform/native-service-ready | failed | timeout waiting for `LogOS: storage formatted` |
| persistence/write-interruption | skipped | semantic proof unavailable |
| persistence/recovery | skipped | semantic proof unavailable |
| persistence/capability-denied | skipped | semantic proof unavailable |
| persistence/corruption-detected | skipped | semantic proof unavailable |
| persistence/storage-service-restart | failed | timeout waiting for `LogOS: storage formatted` |
| network/packet-loss | skipped | semantic proof unavailable |
| network/timeout | skipped | semantic proof unavailable |
| network/reset-reconnect | skipped | semantic proof unavailable |
| network/unauthorized-operation | skipped | semantic proof unavailable |

Artifacts: `target/logos-test/main-1785534282132602000`.
