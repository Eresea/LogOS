# Network flow

Network is optional and has no dependency edge from Commands or any other service.

```text
UEFI NETWORK.CFG
        |
        v
Core validates profile ---- Disabled ----> no VirtIO discovery, DMA, or Network task
        |
        v
VirtIO-net RX/TX queues <-> Core private DMA buffers
        |
        | bounded packet descriptors; no DMA mapping into ring-3
        v
Network private packet pages <-> smoltcp Ethernet/ARP/IP/socket state
        |
        v
Commands <-> versioned Network ABI
```

For `StaticThenDhcp`, Network applies the static IPv4 address first and waits for the fixed
gateway-ARP deadline. If ARP does not resolve, it clears the static path and starts bounded DHCPv4
configuration. `net status` remains usable throughout and reports `configuring` until the interface
is ready.

Network recovery is targeted: Core resets the Network queues and device state, Network resets its
socket/listener tables and private packet pages, and the service epoch invalidates every old socket
handle. Other service tasks and their IPC generations continue running.
