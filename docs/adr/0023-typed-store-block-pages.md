# ADR-0023: Complete typed Store and Block transport

- Status: Accepted
- Date: 2026-08-05

## Context

ABI-v4 had typed Input, Display, Session, and Effect pages, but Store and Block still used the
generic control payload and shared protocol configuration. That coupled replacement identity to a
service context and let Core and native services share protocol state accidentally.

## Decision

- Terminal and other real Store clients use independently owned `StoreClientPage` mappings.
- Storage receives one independently owned `StoreServerPage` and one `BlockClientPage`; a
  hardware-backed block declaration receives no fake native payload page.
- `ControlPage` carries only lifecycle, identity, endpoint addresses, and Store/Block notification
  operation codes. Store and Block state, transfer handles, and replies live in typed pages.
- Store states are `Ready -> Request -> Waiting -> terminal -> Ready` for clients and
  `Ready -> Waiting -> Request -> Processing -> terminal -> Ready` for Storage. Block states are
  `Ready -> Request -> Submitted -> terminal -> Ready`.
- Every transition validates scalar state, service and endpoint generations, request ID, bounded
  fields, and transfer-handle shape. Core owns page loans and returns them on every terminal path.
- `platform::storage::StorageRuntime` owns Store bindings, relay state, Storage rebinding, and the
  concrete `block::Dispatch`; it does not own allocators, page tables, scheduling, or device queues.
- Network and Remote transport remain unchanged.

## Consequences

Replaced client or Storage pages reject stale requests and replies even when physical pages are
reused. Store capability and namespace checks remain in Core. Bulk data remains in separately
owned transfer mappings, while protocol pages carry only validated handles. The existing object
format, block operations, timeout behavior, and recovery proofs remain unchanged.
