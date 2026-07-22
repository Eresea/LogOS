# Architecture

## Current state

LogOS exits UEFI boot services into a Rust kernel. It retains the final UEFI memory map and boot framebuffer metadata, then initializes a physical-page allocator from the largest conventional-memory range.

## Physical memory

The bootstrap allocator returns 4 KiB physical pages from one conventional-memory range. It does not yet free pages or span ranges; both require a kernel-owned metadata allocator.

## Virtual memory

LogOS copies the active UEFI top-level page table, adds one kernel-owned high virtual mapping, then reloads CR3. This preserves the firmware mappings needed during bring-up while proving the kernel can own new mappings.

## Interrupts

The kernel owns the IDT and unmasks only PIT IRQ0 at 100 Hz. The timer handler increments a tick counter; all other hardware IRQs remain masked until their drivers exist.

## Scheduling

The bootstrap scheduler runs two fixed cooperative task slots in round-robin order. A task yields by returning `Ready`; it is removed when it returns `Complete`.

## Capabilities

The kernel grants opaque, generation-tagged capability handles from a fixed table. Checks require a matching kind and generation; revocation invalidates existing handles.

## Device discovery

The kernel scans PCI configuration space and retains a small list of discovered vendor/device identities. Drivers remain separate and bind only when explicitly added.

## Execution model

- The kernel, drivers, scheduler, memory manager, IPC, filesystem, networking, and compositor are native Rust.
- System services expose typed, capability-checked APIs.
- Applications, plugins, automation, and AI agents run as isolated WASM modules.
- WASM modules communicate through kernel-managed IPC, explicit shared memory, and delegated capabilities—not direct pointers.
- WASM hot-loading and hot-swapping are first-class goals; native kernel components are not a dynamic-plugin surface.

## Kernel boundary

Keep the kernel focused on hardware resources, scheduling, memory, IPC, and capabilities. Higher-level functionality is replaceable services. See [boot sequence](boot-sequence.md) and [security](security.md).
