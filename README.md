# LogOS

An experimental Rust-native operating system: a capability-based kernel with a future WebAssembly application platform.

Current milestone: UEFI boot, kernel-owned memory, and a PIT timer interrupt.

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
.\scripts\run.ps1
```

Expected output: `LogOS: timer interrupt ready`.

- [Architecture](docs/architecture.md)
- [Boot sequence](docs/boot-sequence.md)
- [Security](docs/security.md)
- [Roadmap](docs/roadmap.md)
- [Development](docs/development.md)
- [Agent guide](AGENTS.md)
