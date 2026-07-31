# Development

## Prerequisites

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
```

Install QEMU and make `qemu-system-x86_64` available on `PATH`.

## Commands

```powershell
./scripts/check.ps1 -Stage host
./scripts/check.ps1 -Stage uefi
.\scripts\run.ps1
.\scripts\verify.ps1
```

The kernel prints a pass/fail self-check for every initialized subsystem, followed by `LogOS: startup self check passed`. A healthy boot hands normal input, presentation, and commands to the loaded Ring-3 terminal; Escape or `recovery` returns to the kernel recovery console. PS/2 IRQ input is handled through the IDT.

`verify.ps1` delegates to the structured `logos-test` runner. Suite fixtures reuse one QEMU boot when
reset is safe, while boot, privilege, address-space, malformed-image, and storage-recovery cases
receive fresh boots. Use `LOGOS_TEST_ARTIFACTS=all` to retain successful fixture files.

`check.ps1` runs formatting, linting, architecture/docs checks, ADR validation, and host/UEFI artifact checks. `verify.ps1` runs the QEMU suite.
