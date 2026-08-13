# ADR-0024: Service ELF packaging

## Status

Accepted

## Decision

The five service binaries are built as `no_std` ELF executables for the
official `x86_64-unknown-none` target and linked as fixed-address `ET_EXEC`
images at `0x0000_0100_0000_0000`, outside the inherited low kernel mapping.
`scripts/build-services.ps1` emits the fixed ESP layout
under `build/esp/EFI/LOGOS/` and rejects missing, non-ELF, or oversized
artifacts.

The binaries contain bounded service entry loops. IPC loops, capability mapping,
and process startup are separate layers, all admitted by the kernel boot path
after the retained ESP images are validated.

## Consequences

- Service artifacts are independent ELF files with no kernel-module dependency.
- The target is an explicit build prerequisite rather than an implicit host
  compiler assumption.
- The QEMU proof exercises the fixed service images through ring-3 entry and
  the bounded terminal path.
