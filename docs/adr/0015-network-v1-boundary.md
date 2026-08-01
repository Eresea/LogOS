# ADR-0015: Network v1 device and datagram boundary

- Status: Accepted
- Date: 2026-08-02

## Context

Network v1 needs bounded packet connectivity without giving a restartable service authority to
program DMA or exposing raw packets as the application contract. The current native-service ABI
has one context page per service, so a long-lived receive request would also block client traffic.
Endpoint authority must be revocable and exact without introducing a firewall policy registry
before one is needed.

## Decision

Core owns VirtIO network negotiation, DMA queues and bounce buffers, interrupts, timeout, reset,
and resource reclamation. It copies complete Ethernet frames through fixed Network-owned RX/TX
pages and delivers receive frames as bounded events through the Network service's existing context
gate. The service supplies a next deadline while waiting, allowing Core to wake it for a frame,
client request, cancellation, reset, or timer without a second IPC channel.

The Ring-2 Network service owns Ethernet through UDP protocol state and exposes generation-tagged,
owner-bound datagram endpoints. Core gates bind, send, and receive separately. A 64-bit capability
resource encodes an exact protocol/local-port or protocol/remote-IPv4-and-port scope. Clients do not
receive raw packet access.

## Consequences

- A compromised Network service cannot program NIC descriptors or physical addresses.
- Device DMA reaches only Core-owned, zeroed bounce buffers; client and service pages are copied.
- Endpoint access has no ambient authority, wildcard rule, or policy lookup in v1.
- One context gate can serve device events, protocol timers, and client operations with explicit
  bounded buffering and fairness.
- Device or service generation changes invalidate endpoints and pending work.
- Core gains a finite-deadline Network wait/event operation and a 64-bit internal capability
  resource while existing 32-bit scoped APIs remain available.
- Raw sockets, a general firewall, TCP, and DMA isolation from a malicious physical device remain
  out of scope.

## Alternatives considered

- Let the Network service own DMA — rejected until Ring-1 isolation and IOMMU enforcement exist.
- DMA directly into shared service pages — rejected because device lengths and stale bytes would
  cross the trust boundary without a Core-owned copy.
- Add a second service context page for RX — rejected because bounded pushed events and deadline
  wakeups fit the existing gate.
- Expose POSIX sockets — rejected because v1 needs only typed datagram endpoints and cancellation.
- Add a firewall scope registry — rejected because exact IPv4/port scopes fit in the existing
  capability model once its internal resource widens to 64 bits.
