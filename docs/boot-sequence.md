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

Core v1 is complete. The kernel exits UEFI boot services, initializes physical memory and reversible virtual mappings, receives PS/2 and ACPI-routed VirtIO interrupts through its IDT, runs cooperative ready/blocked tasks, enforces capability-gated IPC, and reclaims service-owned resources. The normal Console v1 terminal uses PIT ticks for its blinking caret; keyboard and VirtIO completion interrupts wake work. Recovery framebuffer output remains dormant unless normal-console startup fails or an authorized handoff requests it. Every stage added later must state its dependencies, failure mode, and recovery path.
