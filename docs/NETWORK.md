# Network

> **Status:** Network bootstrap v1 is the compatibility path. The current TCP slice provides bounded
> listener/connection tables, per-connection byte-stream storage, accepted/acknowledged watermarks,
> fair bounded TX scheduling, and a generation-bound `StreamPage`; the full Network suite is green.
>
> **Owner:** Core's Foundation network driver and the replaceable System Network service.

Current proof results are recorded in [testing status](../testing/STATUS.md). Superseded phase
checklists and historical results are in [reviewed Network history](reviewed/network-history.md).

## Goal

Provide bounded, capability-controlled packet and stream connectivity through a replaceable Network
service without making raw packets, POSIX sockets, or a general firewall part of the native contract.

## Ownership and fixed decisions

- Core owns VirtIO negotiation, RX/TX queues, DMA buffers, interrupts, deadlines, reset, physical
  address translation, capability enforcement, and page loans.
- The Ring-2 Network service owns Ethernet, ARP, IPv4, ICMP echo, DHCP, UDP, TCP foundation state,
  endpoint state, and protocol timers.
- Core passes complete Ethernet frames through two Core-owned DMA pages mapped to Network. The device
  never DMA-writes service or client memory.
- Clients use generation-tagged endpoint handles and separate `NetworkBind`, `NetworkSend`, and
  `NetworkReceive` capabilities. A denied request stops in Core before service wake-up, page loan,
  protocol mutation, or NIC access.
- The bootstrap path has one legacy transitional VirtIO device, one RX queue, one TX queue, sixteen
  RX buffers, one TX buffer, eight UDP endpoints, eight ARP entries, four queued datagrams, and one
  pending client operation globally. Fixed limits return `Busy`; they do not grow.
- The service remains allocation-free after startup and reports `Offline` or `Degraded` without
  blocking local boot, Terminal, Store, or recovery.
- Reset or service replacement invalidates endpoints, cancels pending work, returns loans, clears
  protocol state, increments the interface generation, and reacquires DHCP. Clients bind again.
- The TCP slice has one listener, eight connections, four 1 KiB TX chunks, and bounded readiness and
  loss detection. Chunks are byte storage, not message or segment boundaries.
- The hermetic QEMU peer requires no TAP device, administrator privilege, DNS, public internet, or
  host network configuration.

Changing these decisions requires updating this document and `docs/architecture.md`; changing the
cross-ring boundary requires superseding [ADR-0015](adr/0015-network-v1-boundary.md).

## Current implementation

- `logos-abi` and `logos-service-rt` define the typed client/server, device, event, and stream pages.
- `logos-net` contains allocation-free codecs and bounded Ethernet, ARP, IPv4, ICMP, DHCP, UDP, and
  TCP state transitions.
- Core binds the legacy VirtIO device when available, maps typed device/event pages only into the
  Network service, validates DMA identities and generations, and delivers one RX event at a time.
- `logos-network-service` handles device transport, DHCP, ARP, `Status`, datagram operations,
  `Listen`, `Accept`, `Read`, `Write`, `Close`, and `Cancel`. Core validates client authority and
  copies payloads through fixed pages.
- `NetworkClientPage` and `NetworkServerPage` are the sole bootstrap client/server boundary. The
  active association records exact endpoints, request, owner, and completion target; a competing
  client gets `Busy` without touching the active page.
- `SubmitWrite` accepts bytes into connection-owned storage. Sequence numbers are assigned only when
  a wire range is armed. One TCP frame is scheduled per service activation using bounded round-robin
  selection; queued plus in-flight bytes consume the fixed TX budget. `PollStream` is authoritative
  for `Readable`, `Writable`, `Closed`, and accepted/acknowledged byte watermarks. `AwaitWritable`
  defers one client request inside Network until that state changes, so clients without an event page
  do not poll or wait on device events. Legacy `Write` requests defer while the device slot is
  occupied instead of being rejected spuriously.
- The reactor processes device replies directly and preserves FIFO event heads when the next event is
  for another source; typed client/network events cannot strand a pending device completion.
- `StreamPage` is a coalesced notification cache, not a second transport or source of truth. On
  overflow, clients poll owned endpoints and clear the flag.
- `NetworkRuntime` owns readiness through an internal server `Status` request and exposes
  `configured()`/`info()`. Terminal is not a readiness dependency.
- The QEMU harness uses structured `QUERY network/configured` and typed replies; debug output remains
  diagnostic only.

## Contracts and invariants

### Core device boundary

`Info` returns MAC, MTU, link state, interface generation, counters, and fixed limits. `Transmit`
consumes one complete Ethernet frame from the Network TX page; `Receive` publishes one validated frame
through the RX page. Requests and events require matching service, endpoint, device, request, and page
generations. `NetworkDevicePage` uses `Ready -> Request -> Submitted -> Reply`; `NetworkEventPage`
uses `Ready -> Waiting -> Event -> Consumed -> Ready`.

Physical addresses never cross the ABI. Device reset rebuilds queues and resources, reclaims partial
allocations, and publishes a new generation.

### Client boundary

Requests are scalar-validated before wake-up: operation shape, ID, endpoint, owner, exact capability
scope, page direction, length, deadline, and generation all match. Completion accepts only the active
request and exact generation. Timeout, cancellation, reset, replacement, and close each release the
associated page and endpoint state exactly once.

The bootstrap client path is globally serialized for compatibility. Every production status,
including `Busy`, `Denied`, `Invalid`, timeout, cancellation, reset, and failure, wakes the requesting
task. This serialization is deliberately not the target architecture.

### Protocol behavior

The service is fixed-size and allocation-free after startup. Malformed frames are rejected before
state mutation. DHCP retries and lease transitions use bounded deadlines; ARP, datagrams, pending
operations, retransmission, and stream buffers have explicit capacities. Network failure leaves local
services and recovery usable.

## Current work

- Prove Gateway and `logosctl` through the real TCP foundation.
- Extend bounded asynchronous work to RX and concurrent stream operations without moving scheduler
  ownership into Network or adding a generic async framework.
- Implement the five skipped Remote proofs only after their documented multi-boot and persistence
  postconditions are available.

## Automated proof matrix

| Proof ID | Required evidence |
| --- | --- |
| `network/transport-dhcp` | Typed client observes exact configured address, mask, and router |
| `network/device-bind` | Exact NIC class/MAC, posted RX buffers, and host frame |
| `network/configuration` | Typed client observes exact address/mask/router |
| `network/icmp-echo` | Valid echo in both directions with matching ID/sequence/checksum |
| `network/udp-round-trip` | Exact payload and source/destination endpoints both ways |
| `network/unauthorized-operation` | Denial before service wake-up or NIC/page effects |
| `network/simultaneous-client-busy` | One client receives `Busy`; cancellation restores another client |
| `network/backpressure-cancel` | Second request is `Busy`; cancellation releases resources |
| `network/packet-loss` | Deterministic loss and malformed/duplicate traffic make bounded progress |
| `network/timeout` | One timeout reply and no held resources |
| `network/reset-reconnect` | Reset leaves DHCP-bound Network usable without reboot |
| `network/tcp-stream` | Independent host TCP peer completes typed stream operations and close |

Host tests prove codecs, state transitions, bounds, and malformed input. QEMU proves PCI, interrupt,
DMA, service-gate, capability, and recovery paths. The host peer is an independent wire oracle.

## Deferred

- Scalable TCP: per-connection ownership, asynchronous completion, bounded budgets, readiness
  notifications, concurrent operations, and broader capability-scoped socket operations.
- Remote Foundation's exact-port TCP slice, authenticated transport, identity, trust, and key
  ownership in their System services.
- IPv6, fragmentation/reassembly, multicast, VLANs, jumbo frames, raw sockets, packet capture,
  multiple NICs, modern VirtIO, offloads, multiqueue, hotplug, IOMMU protection, DNS, and public
  network tests.
- Ephemeral ports, port reuse, connected UDP, broadcast/multicast datagrams, more than one pending
  bootstrap operation, DHCP persistence, static configuration, and DHCPv6.

Do not split codecs or fixed protocol state into more crates unless a real dependency boundary
requires it.

## References

- [Architecture subsystem boundaries](architecture.md#current-subsystem-boundaries)
- [Security constraints](security.md)
- [Boot constraints](boot-sequence.md)
- [Network boundary ADR](adr/0015-network-v1-boundary.md)
