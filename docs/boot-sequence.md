# Boot Sequence

The boot path defines dependency order; later milestones must not bypass it.

```text
UEFI firmware
  -> kernel entry
  -> memory manager
  -> interrupts and timer
  -> scheduler
  -> capability manager
  -> driver manager and device discovery
  -> identity service
  -> network service
  -> WASM runtime
  -> system services
  -> terminal service
  -> user session
  -> applications
```

## Current point

UEFI boot services have been exited. The kernel retains framebuffer metadata and initializes a physical-page allocator from the final memory map, then halts with debug-console logging. Next: kernel-owned virtual-memory mapping. Every stage added later must state its dependencies, failure mode, and recovery path.
