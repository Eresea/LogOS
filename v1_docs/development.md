# Development

Install the target once:

```powershell
rustup target add x86_64-unknown-uefi
```

Focused proof:

```powershell
./scripts/check.ps1
```

Boot proof:

```powershell
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
./scripts/run.ps1
```

Expected debug output is `LogOS vNext: booted`.
