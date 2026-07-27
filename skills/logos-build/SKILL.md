---
name: logos-build
description: Build LogOS boot images with the canonical Rust test harness. Use for debug or release kernel builds, tool checks, and artifact location reporting.
---

# Build LogOS

1. Run `cargo run -p logos-test -- list` to verify the harness builds.
2. Run the smallest requested scenario or suite; the harness builds and stages the image.
3. Report the artifact directory printed by the harness.
4. Never install tools or change toolchains silently.
