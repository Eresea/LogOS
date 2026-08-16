# ADR-0050: Optional Network service boundary

Status: Accepted

## Decision

Network is a seventh fixed service with an explicit boot profile. Core owns
VirtIO-net discovery, feature negotiation, DMA buffers, one bounded RX/TX
queue pair, interrupt delivery, reset, and deadlines. Network owns Ethernet,
ARP, IPv4, ICMP, UDP, DHCPv4, and TCP state through the pinned smoltcp 0.12.0
dependency. Commands reaches it only through the versioned Network IPC ABI.

The profile is read from \EFI\LOGOS\NETWORK.CFG before ExitBootServices.
Missing or malformed files select Disabled and do not fail boot. Disabled
Network remains visible to the service manager, but no Network task or
hardware initialization is attempted; Commands remains independent and
reports disabled, unavailable, or configuring.

## Fixed bounds and isolation

Network uses eight socket slots, two listener slots, 64 RX descriptors, 64
TX descriptors, 1536-byte Ethernet frames, fixed 2 KiB Core DMA buffers, and
fixed private RX/TX packet pages. Core copies packet bytes between DMA and
Network-owned pages; Network never receives a DMA address. Control messages
remain within the existing 256-byte payload bound.

All requests, responses, packet descriptors, socket handles, endpoint
generations, request IDs, and service epochs are validated. Full queues return
Full or WouldBlock; no queue overwrites or unbounded buffering are permitted.

## Recovery and limits

A Network failure resets only Network sockets, packet pages, queues, and
driver state. Existing socket handles become stale through the Network service
epoch; Storage, Commands, Terminal, and the rest of the graph remain running.
IPv6, DNS, TLS, Wi-Fi, firewalling, NAT, jumbo frames, multiqueue/RSS,
zero-copy service DMA, and arbitrary IPC topology are deferred.

smoltcp 0.12.0 is selected for its no-allocator, IPv4/Ethernet/ICMP/UDP/TCP
and DHCPv4 support under the repository Rust baseline. Network v1 makes no
massive-traffic or high-throughput claim; congestion-control and throughput
work is the first Network v2 milestone.
