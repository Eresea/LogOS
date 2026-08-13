# ADR-0022: Service image manifest

## Status

Accepted

## Decision

The kernel owns one fixed manifest describing the five ring-3 service images.
Each entry contains its shared ABI service ID, process kind, fixed ESP path,
and the process capabilities enforced at admission. Boot order is Input, Display, Terminal,
Session, then Commands.

The manifest validates image size and ELF admission before a future UEFI file
loader hands bytes to the process loader. It does not read files, allocate
memory, or start processes.

## Consequences

- Service packaging has one stable naming and capability source.
- Missing, oversized, or malformed images fail before process admission.
