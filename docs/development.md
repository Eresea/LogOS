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

The terminal prints the boot, memory, timer, scheduler, capability, PCI, VirtIO, IPC, and registry markers, followed by `LogOS: VirtIO service ready`. That final marker verifies queue-0 allocation, PFN programming, driver-ready status, and a routed `Ping → Pong`.
