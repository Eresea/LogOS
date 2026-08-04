<p align="center">
  <img src="assets/logo.svg" width="112" alt="LogOS">
</p>

<h1 align="center">LogOS</h1>

<p align="center">An OS-building thought experiment in Rust.</p>

LogOS is an experimental, capability-based operating system. It keeps the kernel focused on the work only the kernel can do: hardware, memory, scheduling, IPC, and capability enforcement. Everything else is intended to be a replaceable service, with sandboxed WebAssembly applications as the long-term application model.

The normal terminal and Sessions service run as separate, restartable Ring-3 payloads behind capability-gated Core effects; the recovery console remains kernel-owned. Console v1, Platform v1, Persistence v1, and Network v1 are complete; Remote Foundation v1 is implemented and awaiting its QEMU verification gate.

## Start here

- [Architecture](docs/architecture.md) — system boundaries, layers, and service model.
- [Roadmap](docs/roadmap.md) — current progress and the path ahead.
- [Console](docs/CONSOLE.md) — normal terminal scope and versioned checklist.
- [Development](docs/development.md) — prerequisites, build, checks, and QEMU commands.

## Design constraints

- [Boot sequence](docs/boot-sequence.md) — initialization, dependencies, and recovery paths.
- [Security model](docs/security.md) — capability-first authority and isolation.

## Contributing

Read the [agent guide](AGENTS.md) before changing the kernel. Keep changes small, independently bootable, and observable through the QEMU debug console.

## License

LogOS is open source under the [MIT License](LICENSE).
