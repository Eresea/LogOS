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

UEFI boot services have been exited. The kernel initializes physical memory, verifies one kernel-owned virtual mapping, receives PIT IRQ0 through its IDT, runs two cooperative tasks in round-robin order, verifies capability revocation, discovers PCI devices, and routes a capability-gated `Ping` through the service registry to a VirtIO service that completes one balloon request before halting with debug-console logging. Next: driver interrupts or a persistent service task. Every stage added later must state its dependencies, failure mode, and recovery path.
