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

The terminal prints a pass/fail self-check for every initialized subsystem, followed by `LogOS: startup self check passed`. QEMU then shows the kernel recovery console with `help`, `clear`, `version`, and `exit`; PS/2 IRQ input is handled through the IDT.

`verify.ps1` runs QEMU headlessly and requires scheduler wake-up, IPC replies, persistent-service, VirtIO recovery, keyboard input, and final startup health markers within 15 seconds.

`check.ps1` runs formatting, linting, and the headless boot verifier.
