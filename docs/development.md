# Development

## Prerequisites

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
```

Install QEMU and make `qemu-system-x86_64` available on `PATH`.

## Commands

```powershell
cargo fmt --check
cargo clippy -- -D warnings
.\scripts\run.ps1
```

The terminal prints `LogOS: kernel entered`, `LogOS: framebuffer online`, and `LogOS: shell ready`. QEMU shows the `LOGOS` banner, then a basic shell with `help`, `clear`, `version`, and `exit`.
