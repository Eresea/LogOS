---
name: logos-test-change
description: Select and run the minimum safe LogOS tests for changed files. Use when verifying a patch, branch, commit, or working-tree change.
---

# Test a Change

1. Inspect `git diff --name-only` and `git status --short`.
2. Run formatting, workspace clippy, and workspace host tests.
3. Run Console for terminal changes, Platform for service changes, and Core for kernel changes.
4. Run `suite main` for shared Core, protocol, image, harness, or test-hook changes.
5. Summarize structured failures and artifact paths.
