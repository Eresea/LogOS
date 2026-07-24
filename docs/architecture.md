# Architecture

## Current state

LogOS exits UEFI boot services into a Rust kernel. It retains the final UEFI memory map and boot framebuffer metadata, then initializes a physical-page allocator from its conventional-memory ranges.

## Physical memory

The bootstrap allocator returns 4 KiB physical pages from up to eight conventional-memory ranges. Reclaimable callers use owned page tokens; released physical pages form an intrusive free list, so reclamation is not capped by allocator metadata. Long-lived driver pages remain reserved until driver teardown exists.

## Virtual memory

LogOS copies the active UEFI top-level page table, adds one kernel-owned high virtual mapping with an explicit read-only or read-write leaf permission, then reloads CR3. The bootstrap self-check verifies, unmaps, and releases a read-write temporary mapping before continuing.

## Interrupts

The kernel owns the IDT and uses PIT IRQ0 only during startup, then masks it before entering the console so `hlt` wakes only for keyboard or device events. CPU exceptions halt safely; the keyboard handler queues scancodes. ACPI MADT supplies the local-APIC and IOAPIC addresses used for PCI interrupt routing; the VirtIO completion IRQ acknowledges the device and wakes its blocked task.

## ACPI

The UEFI entry path finds and validates the ACPI 2 RSDP and XSDT, retaining MADT APIC addresses after boot services end. It selects QEMU's APIC `_PRT` package through the FADT and routes root-bus PCI interrupts by device and pin, rather than the firmware-programmed PCI interrupt line.

## Scheduling

The bootstrap scheduler runs bounded cooperative tasks in round-robin order. A task yields by returning `Ready`, waits by returning `Blocked(event)`, and is removed when it returns `Complete`. Event wakes release every matching waiter; generation-tagged task handles remain available for direct stale-wake protection. The VirtIO service waits for the VirtIO completion event.

## Capabilities

The kernel grants opaque, generation-tagged capability handles from a fixed table. Checks require a matching kind and generation; revocation invalidates existing handles.

## Device discovery

The kernel scans PCI configuration space and retains a small list of discovered vendor/device identities. Drivers remain separate and bind only when explicitly added.

## VirtIO

The VirtIO balloon service binds a legacy PCI function through its I/O BAR, allocates and programs queue 0 from owned physical pages, releases them if binding fails, and, as a persistent task, answers routed `Ping` messages with `Pong` or submits an `Inflate` request with a completion reply.

## IPC

The kernel provides capability-gated typed request and response channels. Each bounded enqueue receives a request ID; service replies preserve that ID for correlation. A local interrupt-safe spin lock protects queue producers and consumers; a full queue applies backpressure by rejecting the enqueue.

## Service registry

The kernel registers typed services behind opaque handles. Registration requires a service capability, and IPC envelopes use the resolved handle rather than a direct pointer.

## Startup health

Each initialized kernel subsystem must report a startup self-check to the debug console and framebuffer. A failed check emits its module name, displays `FAIL`, and halts; the boot verifier accepts only the final `startup self check passed` marker.

## Tracing

The kernel keeps a fixed in-memory trace ring for low-overhead event diagnostics. It records bootstrap, scheduler block/wake, and VirtIO request/completion events without allocation; the console exports its oldest-to-newest snapshot with `trace`. Filtering and multi-core writers are deferred until a real consumer requires them.

## Kernel console

The kernel renders its console directly to the boot framebuffer and consumes PS/2 IRQ1 scancodes after firmware services end. It provides `help`, `clear`, `version`, `ping`, `inflate`, and `exit`; `ping` and `inflate` send capability-gated IPC requests to VirtIO and display their replies. A full terminal service remains a future userspace concern.

## Execution model

- The kernel, drivers, scheduler, memory manager, IPC, filesystem, networking, and compositor are native Rust.
- System services expose typed, capability-checked APIs.
- Applications, plugins, automation, and AI agents run as isolated WASM modules.
- WASM modules communicate through kernel-managed IPC, explicit shared memory, and delegated capabilities—not direct pointers.
- WASM hot-loading and hot-swapping are first-class goals; native kernel components are not a dynamic-plugin surface.

## Kernel boundary

Keep the kernel focused on hardware resources, scheduling, memory, IPC, and capabilities. Higher-level functionality is replaceable services. See [boot sequence](boot-sequence.md) and [security](security.md).
