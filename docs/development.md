# Development

```powershell
rustup target add x86_64-unknown-uefi
./scripts/check.ps1
```

For a firmware proof, set `OVMF_CODE` and run `./scripts/run.ps1`.
Expected debug output: `LogOS vNext: booted`.
