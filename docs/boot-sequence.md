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

UEFI boot services have been exited. The kernel initializes physical memory, verifies one kernel-owned virtual mapping, receives PIT IRQ0 and PS/2 IRQ1 through its IDT, runs cooperative tasks in round-robin order, verifies capability revocation, discovers PCI devices, routes capability-gated requests through a persistent VirtIO task, then starts a framebuffer console. The PIT is masked before steady state; keyboard and VirtIO completion interrupts wake work. Next: ACPI-derived routing and kernel-owned resource lifecycles. Every stage added later must state its dependencies, failure mode, and recovery path.
