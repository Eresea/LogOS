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

The terminal prints `LogOS: kernel entered`, `LogOS: leaving UEFI boot services`, and `LogOS: boot services exited`. QEMU shows the `LOGOS` boot screen, then the kernel halts after the firmware handoff.
