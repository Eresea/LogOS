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

The terminal prints the boot, memory, timer, scheduler, capability, PCI, VirtIO, and IPC markers, followed by `LogOS: service registry ready`. That final marker verifies capability-gated service registration, lookup, and addressed IPC.
