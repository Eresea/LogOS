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

Core v1 is complete. The kernel exits UEFI boot services, initializes physical memory and reversible virtual mappings, receives PS/2 and ACPI-routed VirtIO interrupts through its IDT, runs cooperative ready/blocked tasks, enforces capability-gated IPC, and reclaims service-owned resources. It then stages, relocates, maps, and starts the normal terminal as a separate Ring-3 payload; keyboard, presentation, and commands cross the bounded bootstrap gate. Recovery framebuffer output remains dormant unless normal-terminal startup fails or an authorized handoff requests it. Every stage added later must state its dependencies, failure mode, and recovery path.

The first Platform v1 loader stage creates a separate service PML4 after physical memory is ready
and before service startup. It depends on the existing kernel map and allocator; an allocation or
mapping failure rejects normal-service startup and leaves the recovery console path intact. It does
execute the terminal service only after the later privilege-transition stage. The second stage
validates the staged PE32+ payload, applies base relocations, copies its sections into Core-owned
frames, and applies user/write/execute page permissions; failure follows the same recovery path.

The third stage installs the privilege-transition GDT and TSS after memory initialization and
before interrupt setup. Its ring-0 stack is Core-owned; a failure prevents service entry and keeps
recovery available.

The fourth stage starts the staged Ring-3 terminal through the service gate after the IDT is
installed. Core routes normal input, presentation, and bounded command requests through that gate;
the terminal handles local redraw while Core delegates system operations to ACPI or platform IPC.
Escape or the authorized `recovery` command returns to the direct recovery console. A failed
transition refuses normal-terminal startup and leaves recovery available.
