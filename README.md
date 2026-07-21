# LogOS

An experimental Rust-native operating system: a capability-based kernel with a future WebAssembly application platform.

Current milestone: UEFI boot, physical-page allocation, and a kernel-owned virtual mapping.

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
.\scripts\run.ps1
```

Expected output: `LogOS: virtual memory ready`.

- [Architecture](docs/architecture.md)
- [Boot sequence](docs/boot-sequence.md)
- [Security](docs/security.md)
- [Roadmap](docs/roadmap.md)
- [Development](docs/development.md)
- [Agent guide](AGENTS.md)
