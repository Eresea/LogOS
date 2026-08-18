# Network flow

Network is optional and is reached by Flow or Fetch through the versioned ABI.

```text
UEFI NETWORK.CFG → Core transport → Network private state
                                  ↘ Flow
                                  ↘ Fetch
```

Core owns VirtIO discovery, DMA, queues, and reset. Network owns protocol state and socket handles.
Flow never receives a Network-owned page; it uses typed `net.status`, `net.ping`, `net.tcp-probe`,
and `net.fetch` registry entries. Fetch remains the owner of HTTP parsing and response-body
transport.
