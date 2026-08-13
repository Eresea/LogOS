# ADR-0028: Service address-space bootstrap

## Status

Accepted

## Decision

After `ExitBootServices`, the kernel initializes one fixed frame pool, loads
each retained service ELF into owned segment and stack frames, and creates one
four-level root per service. Roots inherit the kernel mappings except for the
reserved user PML4 branch; image bytes are populated through the bounded
identity-mapped bootstrap sink before the roots are retained for process
binding.

Service images are linked at `0x0000_0100_0000_0000`, with stacks in a nearby
separate window. The roots are retained by the process table and scheduled
through the bounded ring-3 launch path.

## Consequences

- Service ELF bytes and page tables are real post-UEFI state, not metadata.
- A failed image, population, or mapping operation remains before scheduler
  admission and releases the frames acquired for that attempt.
- Switching into a service root is live; replacement-time kernel-stack
  preservation and page-table teardown remain separately bounded work.
