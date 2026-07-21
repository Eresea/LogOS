# Architecture

## Current state

LogOS exits UEFI boot services into a Rust kernel. It retains the final UEFI memory map and boot framebuffer metadata, then initializes a physical-page allocator from the largest conventional-memory range.

## Physical memory

The bootstrap allocator returns 4 KiB physical pages from one conventional-memory range. It does not yet free pages or span ranges; both require a kernel-owned metadata allocator.

## Virtual memory

LogOS copies the active UEFI top-level page table, adds one kernel-owned high virtual mapping, then reloads CR3. This preserves the firmware mappings needed during bring-up while proving the kernel can own new mappings.

## Execution model

- The kernel, drivers, scheduler, memory manager, IPC, filesystem, networking, and compositor are native Rust.
- System services expose typed, capability-checked APIs.
- Applications, plugins, automation, and AI agents run as isolated WASM modules.
- WASM modules communicate through kernel-managed IPC, explicit shared memory, and delegated capabilities—not direct pointers.
- WASM hot-loading and hot-swapping are first-class goals; native kernel components are not a dynamic-plugin surface.

## Kernel boundary

Keep the kernel focused on hardware resources, scheduling, memory, IPC, and capabilities. Higher-level functionality is replaceable services. See [boot sequence](boot-sequence.md) and [security](security.md).
