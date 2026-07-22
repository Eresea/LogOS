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
.\scripts\verify.ps1
.\scripts\check.ps1
```

The terminal prints a pass/fail self-check for every initialized subsystem, followed by `LogOS: startup self check passed`. QEMU then shows a kernel framebuffer console with `help`, `clear`, `version`, and `exit`; input is polled through PS/2.

`verify.ps1` runs QEMU headlessly and fails if that marker is not reached within 15 seconds.

`check.ps1` runs formatting, linting, and the headless boot verifier.
