# LogOS

LogOS is a bounded, typed desktop system: a fixed graph of fourteen isolated
services (Input, Display, Terminal, Session, Flow, Storage, Network, Fetch,
Device, User, Shell, LockScreen, Atrium, System), the Flow typed language, and
the Atrium retained-scene desktop, all speaking one small syscall ABI
(`logos-abi`).

**v3 runs that system as a hosted Core** on Windows and Linux
([ADR-0096](docs/adr/0096-hosted-core-direction.md)): every service image is an
OS process, its fixed ABI windows are shared memory, and `int 49` is a pipe. Bare
metal arrives later as a Linux distro that boots straight into the hosted Core.
The v2 clean-slate UEFI kernel is frozen on the `v2` branch and tag.

See [the architecture](docs/architecture.md), [development checks](docs/development.md),
and [proof status](testing/STATUS.md).

## Run

```powershell
.\scripts\host.ps1
```

That builds the service images and `logos-host`, then opens the LogOS window.
Equivalent commands:

```powershell
cargo build -p logos-service-images --bins -p logos-host
cargo run -p logos-host
```

## Check

```powershell
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --target x86_64-unknown-none -p logos-service-images --bins
```

The last line keeps the service images `no_std`-clean; the same binaries boot
on the frozen v2 kernel and on the hosted Core.

## Layout

| Path | Owns |
| --- | --- |
| `host/` | the hosted Core: process spawn, shared windows, syscall transport, native window |
| `host/shm/` | shared-memory windows at the fixed ABI addresses |
| `services/images/src/common.rs` | the service facade; `trap` is its one target-specific line |
| `services/*`, `abi/`, `storage/`, `network/`, `package/`, `program/`, `ui*` | the `no_std` system |
| `src/` | v2 Core policy modules reused on the host; the machine layer is frozen |
