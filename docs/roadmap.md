# Roadmap

1. Developer loop: one-command build and QEMU boot, serial logs, debugger support.
2. Framebuffer terminal, keyboard input, and basic shell.
3. Memory management, interrupts, timer, and cooperative scheduling.
4. PCI, VirtIO, and basic networking.
5. Remote console and binary upload.
6. Capability manager, then userspace identity and secrets services.
7. WASM runtime with capability-mediated messaging and hot reload.

This is sequencing, not a promise of interfaces. Build vertical, bootable slices; do not implement future services early.
