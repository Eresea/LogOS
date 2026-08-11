# Network history

This reviewed document preserves the phase-oriented Network implementation record removed from the
active Network contract. ADRs remain authoritative for architectural decisions.

## Completed implementation phases

The v1 work established: typed ABI contracts and rejection tests; allocation-free Ethernet, ARP,
IPv4, ICMP, DHCP, UDP, and bounded protocol state; Core-owned VirtIO DMA and reset/reclamation;
the independently restartable Network service; capability-scoped datagrams; deterministic fault,
timeout, cancellation, malformed-frame, reset, replacement, and restart tests; and the first typed
TCP stream proof.

The former phase checklist and its named proof commands are retained in repository history. The
current proof matrix and open work are maintained in [Network](../NETWORK.md) and [Test Status](../../testing/STATUS.md).

## Earlier transport record

QEMU's built-in DHCP server was an earlier transport proof source, while an independent peer covered
configuration and client paths. The current `transport-dhcp` and `configuration` IDs assert typed
configuration through the direct-client profile; raw Discover/Offer/Request/Ack orchestration is a
separate deferred concern.
