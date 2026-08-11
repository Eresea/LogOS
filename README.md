# LogOS vNext

LogOS vNext is a clean-slate `no_std` Rust UEFI kernel.

The first milestone is deliberately one thing: boot a UEFI binary and emit a
stable line on QEMU's debug console. Scheduling, memory, services, IPC, and
networking are deferred until a concrete proof requires each one.

## Build

```powershell
rustup target add x86_64-unknown-uefi
cargo fmt --check
cargo clippy --target x86_64-unknown-uefi -- -D warnings
cargo build --target x86_64-unknown-uefi
```

Use `./scripts/run.ps1` when QEMU and OVMF are installed.
