# LogOS

An experimental Rust-native operating system: a capability-based kernel with a future WebAssembly application platform.

Core v1 is complete: a dependable, event-driven kernel foundation with UEFI boot, capability-gated IPC, recoverable VirtIO, and a recovery console. Next: Console v1.

```powershell
rustup target add x86_64-unknown-uefi
$env:OVMF_CODE = 'C:\path\to\OVMF_CODE.fd'
.\scripts\run.ps1
.\scripts\verify.ps1
.\scripts\check.ps1
```

Expected output: `LogOS: startup self check passed`.

- [Architecture](docs/ARCHITECTURE.md)
- [Boot sequence](docs/boot-sequence.md)
- [Security](docs/security.md)
- [Roadmap](docs/ROADMAP.md)
- [Development](docs/development.md)
- [Agent guide](AGENTS.md)
