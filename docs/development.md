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

The terminal prints `LogOS: kernel entered`, `LogOS: leaving UEFI boot services`, `LogOS: boot services exited`, `LogOS: physical memory ready`, `LogOS: virtual memory ready`, and `LogOS: timer interrupt ready`. That final marker verifies the IDT and PIT IRQ0 in QEMU.
