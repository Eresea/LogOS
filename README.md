<p align="center">
  <img src="assets/logo.svg" width="112" alt="LogOS">
</p>

<h1 align="center">LogOS</h1>

<p align="center">A small, capability-based operating system written in Rust.</p>

LogOS keeps the kernel focused on the work only the kernel can do: hardware, memory, scheduling, IPC, and capability enforcement. Everything else is intended to be a replaceable service, with sandboxed WebAssembly applications as the long-term application model.

Console v1 is complete: a normal local terminal operates on typed commands and can hand off to an independent recovery console. Next: Platform v1.

## Start here

- [Architecture](docs/ARCHITECTURE.md) — system boundaries, layers, and service model.
- [Roadmap](docs/ROADMAP.md) — current progress and the path ahead.
- [Console](docs/CONSOLE.md) — normal terminal scope and versioned checklist.
- [Development](docs/development.md) — prerequisites, build, checks, and QEMU commands.

## Design constraints

- [Boot sequence](docs/boot-sequence.md) — initialization, dependencies, and recovery paths.
- [Security model](docs/security.md) — capability-first authority and isolation.

## Contributing

Read the [agent guide](AGENTS.md) before changing the kernel. Keep changes small, independently bootable, and observable through the QEMU debug console.
