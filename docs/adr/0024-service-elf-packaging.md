# ADR-0024: Service ELF packaging

## Status

Accepted

## Decision

The five service binaries are built as `no_std` ELF executables for the
official `x86_64-unknown-none` target and linked as fixed-address `ET_EXEC`
images at `0x400000`. `scripts/build-services.ps1` emits the fixed ESP layout
under `build/esp/EFI/LOGOS/` and rejects missing, non-ELF, or oversized
artifacts.

The binaries currently contain bounded service entry stubs that idle. IPC
loops, capability mapping, and process startup are separate implementation
slices; the artifacts are not yet invoked by the kernel boot path.

## Consequences

- Service artifacts are independent ELF files with no kernel-module dependency.
- The target is an explicit build prerequisite rather than an implicit host
  compiler assumption.
- The current QEMU proof remains unchanged until real service entry loops are
  ready.
