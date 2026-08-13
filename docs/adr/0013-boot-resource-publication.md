# ADR-0013: Boot resource publication

## Status

Accepted

## Decision

UEFI protocol handles, borrowed memory-map entries, and GOP mode objects do
not cross `ExitBootServices`. The handoff copies only validated fixed-size
descriptors into `boot_resources`: a capped memory map, a framebuffer
descriptor, and the PS/2 keyboard resource identity.

The framebuffer is limited to `MAX_FRAMEBUFFER_BYTES`; the memory map is
limited to `MAX_MEMORY_DESCRIPTORS`. Unsupported GOP formats or malformed
ranges fail the boot handoff rather than producing partially initialized
display state.

## Consequences

- Later kernel and service code can operate without UEFI protocol lifetimes.
- The Display service receives a capability to the published framebuffer, not
  a UEFI object.
- The Input service receives keyboard bytes through a bounded kernel adapter;
  PS/2 decoding remains outside the kernel.
