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

Core v1 and the Network device-facing and client ABI tranches are implemented. The kernel exits UEFI boot services, initializes physical memory and reversible virtual mappings, receives PS/2 and ACPI-routed VirtIO interrupts through its IDT, runs cooperative ready/blocked tasks, enforces capability-gated IPC, and reclaims service-owned resources. It independently stages, validates, relocates, maps, and starts Ring-3 Terminal, Sessions, Store, and Network payloads. NetworkRuntime owns Network readiness through an internal server `Status` transaction; Terminal is not used as a Network probe, and production Network replies always wake and run their blocked caller. Network DHCP and packet work are non-blocking: offline or restarting networking never prevents Terminal, Store, or recovery startup. Recovery framebuffer output remains dormant unless Terminal startup fails or an authorized handoff requests it. Missing optional services select a degraded local mode rather than failing Core. Every stage added later must state its dependencies, failure mode, and recovery path.

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
Terminal task gets one clean address-space replacement with a new generation-tagged handle; a
failed replacement enters the direct recovery console. Sessions, Store, and Network restart
independently with bounded backoff; their exhaustion leaves the normal terminal usable in degraded
mode. A replacement never reuses the failed service's mappings or context.
The payload header transport ABI must match exactly; protocol major must match and the payload minor
must support the manifest requirement. Ring-3 exception stubs normalize CPU frames and return fault
metadata to Core. Service panics use the same typed failure gate. Cleanup and replacement begin only
after Core has restored its own address space and stack.

## Persistence ordering and recovery

After device discovery and Block-driver binding, Core starts the Ring-2 Store and completes its
`Info`/format-or-recover handshake before Terminal accepts normal input. Store owns the raw
`target/logos-store.raw` device through the Core Block gate and its transfer page; Terminal history
is loaded only after Store reports ready. A blank device is formatted, while corruption or I/O
failure is reported as a degraded Store status and never silently reformatted. The recovery
console remains available for those statuses, and an incomplete committed tail is recovered using
the last valid superblock. Store restart cancels in-flight Block work, reclaims its pages, and
repeats this startup handshake before service traffic resumes.

## Remote Foundation ordering and recovery

Before leaving the bootstrap path, the trust owner derives separate device and storage keys from
the UEFI root, seeds bounded ephemeral-key generation from firmware entropy, and wipes the root.
After Store recovery, the trust owner opens the protected enrollment record. Session-record loading,
the optional Gateway, and durable remote replay are the remaining attachment work; they must not
delay local boot. Missing entropy, root key, enrollment, Store, Sessions, Network, or Gateway
leaves remote explicitly unavailable while Terminal and recovery remain usable. The completed
Network transport slice preserves trusted caller ownership across the Network-service boundary.
