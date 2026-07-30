# Network

> **Status:** Network v1 planned
>
> **Owner:** Foundation network driver and System network service

## Goal

Prove bounded, capability-controlled packet connectivity through a replaceable network service.

## V1 scope

- [ ] Discover and drive one VirtIO network device.
- [ ] Support Ethernet, ARP, IPv4, ICMP, DHCP, and UDP.
- [ ] Expose bounded asynchronous datagram send/receive with cancellation, timeout, and backpressure.
- [ ] Require scoped capabilities to send, bind, or receive.
- [ ] Trace lifecycle and recover the driver/service after failure.

## Exit proof

In QEMU, LogOS obtains configuration, exchanges an ICMP echo and UDP datagram with the host, denies unauthorized send/bind/receive operations, and recovers from timeout, packet loss, and device reset without reboot or leaked resources.

## Deferred

- TCP, DNS, TLS, trust stores, certificate validation, and secure enrollment.
- Firewall policy beyond capability-scoped endpoints.
- IPv6, Unix-style socket compatibility, and SSH.

TCP plus authenticated transport become prerequisites of Remote v1; they may form Network v2 or the first Remote slice when that milestone begins. The outward datagram contract and driver/service ownership boundary require an [ADR](adr/README.md) before implementation. See [Architecture](ARCHITECTURE.md#12-networking-model) and [Security](security.md).
