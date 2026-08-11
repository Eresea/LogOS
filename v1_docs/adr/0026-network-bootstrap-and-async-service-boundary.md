# ADR-0026: Separate Network bootstrap from asynchronous service architecture

- Status: Accepted
- Date: 2026-08-08

## Context

The Network bootstrap proved typed device transport, safe DMA ownership, Ethernet, ARP, IPv4,
DHCP, UDP, and basic capability checks. Its first client implementation still serializes all
Terminal and Gateway work through one global transaction. TCP and Gateway experiments exposed that
this bootstrap path is not a scalable socket architecture: protocol processing must not decide
which higher-level service to wake or run.

## Decision

Keep the bootstrap ABI and fixed-resource device boundary while treating the current single
transaction as a compatibility path. NetworkRuntime may publish replies and record explicit service
or client wake notifications, but scheduler execution belongs to the Core composition layer.

The next Network architecture owns application and protocol state in layers:

- application writes enqueue into connection-owned TX buffers and return;
- bounded Network work turns queued data into TCP segments and device submissions;
- TX used-ring progress reclaims NIC buffers, and ACKs advance protocol/retransmission state;
- bounded RX draining places payload into per-connection RX buffers and publishes readability;
- one connection's timeout or reset cannot cancel unrelated connections.

Gateway and Remote are clients of this boundary, never scheduling dependencies inside Network.
Multiqueue device support, a scalable socket ABI, and robust production TCP remain deferred until
their proofs exist.

## Consequences

- Network bootstrap status is documented separately from Network architecture status.
- Existing typed datagram proofs remain useful evidence and are not rewritten as TCP proofs.
- A genuine TCP stream proof must run without Remote before Gateway and `logosctl` are used as the
  next vertical proof.
- No new ABI is required for the scheduler-boundary slice.
