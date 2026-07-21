# Architecture

## Current state

LogOS is a Rust UEFI executable that enters the kernel, writes to QEMU's debug console, and draws a framebuffer banner through UEFI GOP.

## Execution model

- The kernel, drivers, scheduler, memory manager, IPC, filesystem, networking, and compositor are native Rust.
- System services expose typed, capability-checked APIs.
- Applications, plugins, automation, and AI agents run as isolated WASM modules.
- WASM modules communicate through kernel-managed IPC, explicit shared memory, and delegated capabilities—not direct pointers.
- WASM hot-loading and hot-swapping are first-class goals; native kernel components are not a dynamic-plugin surface.

## Kernel boundary

Keep the kernel focused on hardware resources, scheduling, memory, IPC, and capabilities. Higher-level functionality is replaceable services. See [boot sequence](boot-sequence.md) and [security](security.md).
