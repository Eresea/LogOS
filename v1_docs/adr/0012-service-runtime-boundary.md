# ADR-0012: Native Service Runtime Boundary

- Status: Accepted
- Date: 2026-07-29

## Context

Native services in Ring 1-3 (`logos-terminal-service`, `logos-sessions-service`, and `logos-storage-service`) are PE binaries loaded into separate task address spaces. They also need one bounded context/trap boundary. Leaving raw context access and syscall assembly in each payload duplicates unsafe ABI code.

## Decision

Use `crates/logos-service-rt` as the standard, lightweight `no_std` runtime library for native services in Ring 1-3. It owns the PE entry adapter, panic handler, raw context access, bounds validation, typed operation clients, and syscall trap. Service implementation files use only typed wrappers.

Native services MUST NOT depend on firmware crates (`uefi`).

## Consequences

- Native service manifests drop all dependencies on `uefi`.
- Service panic handling stays isolated within the task's address space without assuming UEFI boot services exist.
- New native services use `logos-service-rt` as their standard runtime contract.

## Alternatives considered

- Retaining `uefi` dependency in services - rejected because services execute in bare-metal user tasks after `exit_boot_services`.
- Duplicating entry points and panic handlers in every service crate - rejected to prevent duplication and inconsistent behavior.
