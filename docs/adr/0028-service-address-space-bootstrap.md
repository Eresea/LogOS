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

Service images are linked at `0x0000_0080_0000_0000`, with stacks in a nearby
separate window. The service roots are not scheduled yet; this commit proves
resource ownership and address-space construction only.

## Consequences

- Service ELF bytes and page tables are real post-UEFI state, not metadata.
- A failed image, population, or mapping operation remains before scheduler
  admission and releases the frames acquired for that attempt.
- Switching into a service root is deferred until kernel-stack preservation and
  process/task launch are integrated and separately proven.
