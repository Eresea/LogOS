# ADR-0024: Typed Network device and event transport

- Status: Accepted
- Date: 2026-08-05

## Context

ABI-v4 persistence paths already use typed endpoint pages, but Network device requests, replies,
events, deadlines, and DMA identities still occupied the generic `ControlPage` payload. That made
NIC reset and Network-service replacement difficult to validate without conflating lifecycle state
with protocol state.

## Decision

Use distinct `NetworkDevicePage` and `NetworkEventPage` mappings granted only to the Network service.
All shared state is scalar wire data converted and validated before use. `ControlPage` remains a
lifecycle and notification header; it carries no Network payload, deadline, event, or DMA handle.

`NetworkRuntime` owns the device-facing Network lifecycle: typed endpoint bindings, current device
generation, pending request and deadline, event waiting, reset/reconnect, resource identities, and
replacement rebinding. Core continues to own physical pages, page tables, VirtIO queues, interrupts,
reset mechanisms, and capability enforcement. No raw physical address crosses the typed ABI.

### Device request state

| Transition | Permitted actor and validation |
| --- | --- |
| `Ready -> Request` | Network service; service and endpoint generations match, device generation is current, operation and deadline are scalar-valid, and configured handles are present. |
| `Request -> Submitted` | Core; operation, request ID, service/endpoint/device generations, and DMA handles validate before driver submission. |
| `Submitted -> Reply` | Core; completion matches the active request ID and operation. Timeout/reset publishes only the matching request result. |
| `Submitted -> Ready` | Network service consumes the matching typed reply; stale IDs or generations fail without mutation. |
| any active state -> reset | Core; invalidates the device generation and rebinds resources before reuse. |

### Event state

| Transition | Permitted actor and validation |
| --- | --- |
| `Ready -> Waiting` | Network service; finite deadline and current service/endpoint/device generations are valid. |
| `Waiting -> Event` | Core/Foundation; event sequence, kind, device generation, RX handle, and bounded length validate. |
| `Event -> Consumed` | Network service; one event is read from the single slot. |
| `Consumed -> Ready` | Network service acknowledgement; the slot is cleared. |
| `Event -> Event` | Rejected; no unbounded queue or overwrite is permitted. |
| replacement/reset | Core; old service/device generations and event handles fail deterministically. |

RX and TX pages remain Core-owned. The service receives only configured virtual mappings where
needed, generation-bound `PageHandle` identities, and bounded lengths. Resource creation, driver
access, service access, acknowledgement, timeout, reset, service fault, replacement, and final
reclamation stay under Core/`NetworkRuntime` coordination.

Normal Network client request/reply transport remains on its existing bounded context path in this
tranche. Terminal and Gateway do not receive device or event endpoints. Migrating that client path
is a separate follow-up and must not add a second compatibility mechanism.

## Consequences

- Device information, DHCP setup, TX submission, RX delivery, timeout/reset, and replacement use
  typed pages and generation-safe validation.
- A malformed scalar, stale endpoint, stale event, or reused handle is rejected before unrelated
  endpoint state or driver state changes.
- The ABI migration is intentionally partial until the next Network client-transport tranche.
