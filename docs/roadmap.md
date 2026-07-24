# Roadmap

## Core v1

Core v1 is a dependable, event-driven kernel foundation. It is not a desktop OS, userspace, network stack, filesystem, or WASM runtime.

### Already demonstrated

- [x] UEFI boot, debug output, startup health gate, and automated headless QEMU verification.
- [x] Framebuffer console with PS/2 IRQ keyboard input.
- [x] IDT, exception halt path, PIT bootstrap clock, and ACPI-derived IOAPIC VirtIO completion IRQ.
- [x] Cooperative ready/blocked task scheduler, generation-tagged task handles, and event-driven idle.
- [x] Physical-page allocation across conventional-memory ranges, owned-page recycling, and a reversible bootstrap virtual mapping.
- [x] Capability checks, service registry, queued IPC requests/replies, PCI discovery, and legacy VirtIO balloon service.
- [x] ACPI RSDP/XSDT/MADT validation and APIC topology discovery.
- [x] Fixed trace ring for bootstrap, scheduler, and VirtIO lifecycle events.

### Required before Core v1

- [x] Parse ACPI PCI routing and remove reliance on firmware-programmed PCI interrupt lines.
- [ ] Extend owned-page tracking beyond the bounded recycle pool; add permissioned mappings and service-lifetime reclamation.
- [x] Replace fixed scheduler slots with event wait queues.
- [x] Make IPC safe for interrupt and concurrent producers, with bounded backpressure and request/reply correlation.
- [ ] Generalize VirtIO queue ownership, completion, errors, and device reset; keep the balloon driver as the hardware proof.
- [ ] Define driver lifecycle: discover, bind, interrupt, quiesce, and recover without kernel-wide assumptions.
- [ ] Add kernel panic/fault diagnostics, structured health reporting, and a defined recovery policy for failed drivers.
- [ ] Expand the trace ring into fault and driver lifecycle diagnostics with an export path.
- [x] Add ACPI power-off and reset for a real `exit` path.
- [ ] Expand QEMU integration checks to cover console input, IPC replies, blocked-task wake-up, and driver recovery.

### Core v1 exit criteria

- Boots on the supported QEMU profile without firmware-address assumptions.
- Sleeps when idle; hardware completion wakes only the waiting task.
- Reclaims pages and mappings after a service request completes or fails.
- A capability-gated client can issue, cancel, and receive a correlated reply from a persistent driver service.
- Device or driver failure is reported and contained without silent spinning or undefined memory ownership.
- Every required path is covered by the automated QEMU health gate.

## After Core v1

Identity, secrets, networking, remote console, filesystems, WASM applications, compositing, and desktop UX are replaceable services built on the Core v1 contracts.
