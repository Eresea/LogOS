# Development checks

Host scheduler checks:

```text
cargo fmt --check
cargo test --lib
cargo clippy --lib -- -D warnings
```

UEFI checks use `scripts/check.ps1`; the target is `x86_64-unknown-uefi` and the package has no
allocator. The bounded proof runner accepts `-Cpus 1`, `-Cpus 2`, or `-Cpus 8`:

```text
.\scripts\run.ps1 -Proof -Cpus 1 -TimeoutSeconds 60
```

Proof mode captures debugcon output, rejects fatal markers, requires the structured PASS marker,
and terminates QEMU after the bounded timeout.
