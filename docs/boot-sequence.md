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

UEFI boot services have been exited. The kernel initializes physical memory, copies the active top-level page table, and verifies one kernel-owned virtual mapping before halting with debug-console logging. Next: interrupt descriptor table and timer. Every stage added later must state its dependencies, failure mode, and recovery path.
