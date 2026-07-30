# Network

> **Status:** Network v1 planned

## Goal

Provide asynchronous, capability-controlled networking as a service.

## V1 scope

- [ ] VirtIO network driver; Ethernet, ARP, IPv4, ICMP, DHCP, UDP, TCP, and DNS.
- [ ] Cancellation, timeout, backpressure, limits, lifecycle tracing, and driver recovery.
- [ ] Capability-controlled connect, bind, and listen plus firewall policy and audit events.
- [ ] TLS client/server, trust store, certificate validation, secure enrollment, and explicit TOFU or pinned-key workflows where appropriate.

## Exit criteria

- LogOS obtains configuration, resolves DNS, and completes a validated TLS connection.
- Unauthorized connect/listen operations are denied and driver failure is recoverable.
- Application-facing APIs contain no IPv4-specific assumptions.
- QEMU covers packet loss, timeout, reset, reconnect, and denial.

## Deferred scope

- IPv6 without redesigning outward APIs.
- Unix-style socket compatibility.
- SSH as an optional compatibility service.

See [Architecture](ARCHITECTURE.md#12-networking-model) and [Security](security.md).
