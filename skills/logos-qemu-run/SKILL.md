---
name: logos-qemu-run
description: Run LogOS QEMU scenarios and suites through the canonical harness. Use for boot proofs, module suites, recovery tests, and reproducible emulator runs.
---

# Run QEMU Proofs

1. List IDs with `cargo run -p logos-test -- list` when the proof is unclear.
2. Run `cargo run -p logos-test -- run <id>` or `suite <name>`.
3. Preserve the printed seed and artifact path on failure.
4. Do not add custom QEMU flags outside harness debugging.
