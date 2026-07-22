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

The terminal prints the boot, memory, timer, and scheduler markers, followed by `LogOS: capability manager ready`. That final marker verifies capability grant, revocation, and stale-handle rejection in QEMU.
