# LogOS vNext

LogOS vNext is a clean-slate `no_std` Rust UEFI kernel with a bounded preemptive
SMP Core, fixed service graph, retained UI scene path, and QEMU regression proofs.

The current first milestone targets a trustworthy desktop baseline on supported
hardware. It proves UEFI handoff, per-CPU setup, scheduling, bounded services and
IPC, framebuffer input/rendering, and the LockScreen/System desktop surfaces.
See [the active architecture](docs/architecture.md), [development checks](docs/development.md),
and [current proof status](testing/STATUS.md) for the owned boundaries and deliberate limits.

## Build

```powershell
rustup target add x86_64-unknown-uefi
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --target x86_64-unknown-uefi
```

Use `./scripts/run.ps1` when QEMU and OVMF are installed.
