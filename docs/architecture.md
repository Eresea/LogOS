# Architecture

## Current state

LogOS is a Rust UEFI executable that enters the kernel and writes to QEMU's debug console.

## Direction

- Native Rust kernel and drivers.
- Capability-based IPC between kernel-managed services.
- WASM for applications, plugins, automation, and AI-generated modules.
- Replaceable services with typed APIs; no direct application-to-application control.

## Kernel boundary

Keep the kernel focused on hardware resources, scheduling, memory, IPC, and capabilities. Graphics, networking, package management, AI runtime, and desktop services sit above it.
