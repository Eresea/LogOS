# LogOS Onion Rings

> **Status:** Optional architecture reference

The [compact architecture contract](architecture.md) is canonical. This page records the placement
heuristic without repeating subsystem checklists.

## Ring summary

| Ring | Purpose | Typical contents | Recovery expectation |
| --- | --- | --- | --- |
| 0 Core | privileged mechanisms and global invariants | CPU, memory, scheduler, IPC, capabilities, DMA | always available |
| 1 Foundation | hardware-facing abstractions | drivers, display/input, block and network device paths | restart/rebind where possible |
| 2 System | machine policy and durable state | supervisor, identity, Store, Network, trust, audit, update | independently restartable |
| 3 Sessions | human and remote interaction | sessions, terminal, command dispatch, Gateway | replaceable; recovery is outside it |
| 4 Runtime | application execution | WASM host, packages, workspaces, tools | sandboxed and replaceable |
| 5 Experience | presentation | compositor, desktop, graphical clients | optional |

Rings are dependency, authority, and failure boundaries. They are not x86 privilege rings, crate
boundaries, or a requirement that every component be a separate process.

## Global rules

- Put an invariant in the innermost ring that must enforce it, and no deeper.
- Keep policy outside Core; keep raw hardware outside sessions and applications.
- Grant exact capabilities and typed contracts; do not expose ambient authority.
- Give long-lived state to the subsystem that owns its resources and can advance it.
- Define bounded capacity, cancellation, timeout, generation, replacement, and recovery behavior.
- Prefer native Rust only for trusted mechanisms or interfaces that cannot yet be sandboxed; prefer
  WASM for replaceable application policy.

## Placement procedure

1. State the invariant and owned resources.
2. Identify the minimum authority and nearest ring that can enforce it.
3. Define the outward typed contract and fixed limits.
4. Define failure, restart, replacement, and recovery behavior.
5. Select host tests for portable state and QEMU proofs for isolation or hardware seams.
6. Record an ADR when the choice changes a cross-ring or irreversible boundary.

Use the [naming register](NAMING.md) only for new or renamed subsystem vocabulary. Historical ring
checklists are intentionally not part of the active documentation path.
