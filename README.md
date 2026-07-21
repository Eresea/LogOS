# LogOS

Phase 0.1: a Rust UEFI kernel that logs its entry to QEMU's debug console.

Install the Rust target, QEMU, and OVMF once:

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
```

Boot it:

```powershell
.\scripts\run.ps1
```

Expected output: `LogOS: kernel entered`.
