# Test Status

Last run: `cargo run -p logos-test -- suite main`
Seed: `424242`
Result: **21 passed, 36 skipped**

Persistence suite: `cargo run -p logos-test -- suite persistence` passed, seed `424242`.

| Test | Status | Detail |
| --- | --- | --- |
| console/structured-command | passed | |
| console/capability-denied | passed | |
| console/input-capability-denied | passed | |
| console/display-capability-denied | passed | |
| console/session-capability-denied | passed | |
| console/cancellation | passed | |
| console/input-service-restart | passed | |
| console/terminal-service-restart | passed | |
| console/sessions-service-restart | passed | |
| core/boot-normal | passed | |
| persistence/block-read-flush | passed | |
| persistence/terminal-history | passed | |
| persistence/block-timeout-reset | passed | timeout reset followed by a real read without reboot |
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
| platform/runtime-crash-restart | passed | |
| platform/restart-backoff | passed | |
| platform/native-service-ready | passed | |
| persistence/write-interruption | passed | replacement and compaction interruption matrix |
| persistence/recovery | passed | |
| persistence/capability-denied | passed | |
| persistence/corruption-detected | passed | corruption reported without reformat |
| persistence/storage-service-restart | passed | |
| network/packet-loss | skipped | semantic proof unavailable |
| network/timeout | skipped | semantic proof unavailable |
| network/reset-reconnect | skipped | semantic proof unavailable |
| network/unauthorized-operation | skipped | semantic proof unavailable |

Artifacts: `target/logos-test/main-424242.run2` (same-seed reruns are isolated with suffixed directories).
