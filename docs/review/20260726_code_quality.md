# Code Quality Follow-ups

The 2026-07-26 review is archived in `docs/reviewed/20260726_code_quality.md`.

Keep these constraints for future work:

- Before non-identity physical mappings, define one explicit physical-memory access model. Do not add a higher-half direct map until a milestone needs it.
- Before Persistence v1 or Network v1, define the bounded DMA buffer/allocation contract those drivers require; do not introduce a buddy allocator speculatively.
- Keep host-testable code in workspace crates and use QEMU for hardware-facing behavior.
