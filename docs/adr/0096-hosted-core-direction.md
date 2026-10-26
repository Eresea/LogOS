# ADR-0096: LogOS v3 is a hosted Core; bare metal arrives as a Linux distro

- Status: Accepted
- Date: 2026-10-25

## Context

LogOS v2 is a clean-slate `no_std` UEFI kernel: a bounded preemptive SMP Core
that loads fourteen ring-3 service images (Input, Display, Terminal, Session,
Flow, Storage, Network, Fetch, Device, User, Shell, LockScreen, Atrium, System)
and proves them under QEMU with PS/2 input, GOP framebuffer, and VirtIO block,
net, and GPU devices.

The value of LogOS is above the machine layer: the typed service ABI
(`logos-abi`), the Flow language, the bounded Storage, User, Network and Fetch
services, the retained-scene UI stack, and the Atrium desktop. Those crates are
roughly eighty thousand lines, already compile on the host for tests, and only
reach the kernel through `services/images/src/common.rs`, where twelve
`int 49` wrappers and a handful of fixed-address windows form the whole seam.

The development machine (AMD Ryzen 7 9700X on an AM5 board, 32 GiB, USB-only
keyboard and mouse, NVMe system disks, Realtek 2.5 GbE and RTL8852CE Wi-Fi,
Radeon iGPU plus an RTX 3080) has none of the devices the v2 kernel drives.
Booting v2 on it would need an xHCI host controller stack with USB HID, an NVMe
driver, a Realtek RTL8125 driver, and a Wi-Fi stack that is not realistic
without vendor firmware. Each of those is a multi-week driver project that adds
nothing to Flow, Storage, or Atrium, and QEMU cannot stand in for the real
devices.

Two hosted shapes were considered. Running the services as threads of one
process is less code but breaks the ABI: every service dereferences the same
fixed addresses (`IPC_STAGING_BASE`, `SERVICE_BOOTSTRAP_BASE`, the Display and
Input windows), and v2's isolation and independent-restart guarantees disappear.
Running each service image as an OS process keeps the byte-for-byte ABI, the
isolation, and the restart model, at the cost of a shared-memory mapper and a
pipe transport.

## Decision

LogOS v3 is a **hosted Core**. `host/` (`logos-host`) replaces the v2 machine
layer with the host operating system:

- a service is an OS child process running the unchanged `logos-service-images`
  binary; its `_start` is entered after the fixed ABI windows are mapped;
- the fixed windows (`bootstrap`, `ipc`, `display`, `display_config`, `input`)
  are shared memory created by Core and mapped by the child at the bare-metal
  addresses (`host/shm`, `logos-host-shm`);
- `int 49` becomes a 24-byte request on the child's stdout and an 8-byte reply on
  its stdin; `common::trap` is the single target-specific line of the facade;
- Core presents the Display framebuffer in a native window and feeds the Input
  byte rings from native events;
- the Core policy modules that v2 host-tests (`runtime_ipc`, `runtime_events`,
  `runtime_services`, `service_manager`, `service_lifecycle`, `supervisor`) are
  reused by the hosted Core as the service graph lands; v2's machine modules
  (`arch`, `page_table`, `user_mode`, `virtio*`, the UEFI entry) are frozen and
  retired from `main` once the hosted graph owns every proof they covered.

Windows and Linux are both supported hosts. Bare metal is delivered as a
**Linux distro**: a Linux kernel and initramfs that boot straight into
`logos-host` as init on DRM/KMS and evdev, so the real hardware is driven by
Linux drivers and LogOS owns everything the user sees. No new from-scratch
driver work is started.

The v2 kernel is preserved unchanged on the `v2` branch and the `v2` tag.

## Alternatives rejected

- **Continue from scratch on this hardware.** Blocked on xHCI/HID, NVMe, RTL8125
  and Wi-Fi drivers before a single keystroke reaches Input; none of it advances
  the typed system that is LogOS.
- **Linux distro first.** A distro is packaging around a hosted Core; without the
  hosted Core there is nothing to package, and developing inside a VM or on a
  second machine is slower than running the services as processes on the
  desktop.
- **Threads in one process.** Smaller, but it forks the ABI and gives up
  isolation and restart.

## Consequences

- Service and shared crates stay `no_std`, allocation-bounded, and buildable for
  `x86_64-unknown-none`; nothing in them learns about the host.
- `common.rs` keeps its logic unconditional; only `trap` and the `host` module
  are target-gated. Host builds of the images run the real discovery, wait, and
  IPC code against the hosted Core instead of returning stub values.
- The hosted Core must reproduce the v2 service graph: directory records,
  capabilities, endpoints, queues, event sets, heartbeats, and supervisor
  restart. Until it does, services discover nothing and idle; that gap is the
  first milestone of v3 and is tracked in issues, not here.
- Input, Storage and Network get host backends (native events to the PS/2 byte
  rings, a disk-image file behind the Storage data page, a socket relay behind
  the Network packet pages). The service images do not change for this.
- QEMU proofs are replaced by host proofs: the same structured PASS markers,
  driven by `logos-host` instead of `run.ps1`.
- The first new dependencies since v1 are `minifb` (native window),
  `windows-sys`, and `libc`; they live only in `host/`.
- Local wall time, memory totals, and the Linux shared-memory path are
  unverified in the first slice and are called out in `testing/STATUS.md`.
