# Core

> **Status:** Core v1 complete
> **Owner:**

Core v1 is a dependable, event-driven kernel foundation. It is not a desktop OS, userspace, network stack, filesystem, or WASM runtime.

## v1 — Complete

### Demonstrated

- [x] UEFI boot, debug output, startup health gate, and automated headless QEMU verification.
- [x] Framebuffer recovery console with PS/2 IRQ keyboard input.
- [x] IDT, exception halt path, PIT bootstrap clock, and ACPI-derived IOAPIC VirtIO completion IRQ.
- [x] Cooperative ready/blocked task scheduler, generation-tagged task handles, and event-driven idle.
- [x] Physical-page allocation across conventional-memory ranges, owned-page recycling, and reversible bootstrap virtual mappings.
- [x] Generation-tagged capability grants and revocation, service registry, queued IPC requests/replies, PCI discovery, and legacy VirtIO balloon service.
- [x] ACPI RSDP/XSDT/MADT validation and APIC topology discovery.
- [x] ACPI PCI routing without reliance on firmware-programmed interrupt lines.
- [x] Permissioned mappings and service-lifetime memory reclamation.
- [x] Event wait queues.
- [x] Interrupt-safe IPC producers, bounded backpressure, cancellation, and request/reply correlation.
- [x] Generalized VirtIO queue ownership, completion, error handling, and reset.
- [x] Driver lifecycle: discover, bind, interrupt, quiesce, recover.
- [x] Panic diagnostics, structured health reporting, driver recovery policy, and trace export.
- [x] ACPI power-off and reset.
- [x] QEMU integration checks for console input, IPC replies, task wake-up, and driver recovery.

### Preserved role

The current framebuffer interface becomes the **recovery console**.

It remains:

- independent of the normal terminal stack;
- deliberately small and auditable;
- usable when services, storage, fonts, or the WASM runtime are unavailable;
- limited to health, diagnostics, service recovery, trace export, reset, and power-off.

It should not grow into the normal user environment.
