# Documentation map

Keep the default reading path small. Most documents are optional references, not project context.

## Read by default

1. [Roadmap](roadmap.md) and [TODO](TODO.md) for current work.
2. [Architecture](architecture.md) for active ownership and ABI boundaries.
3. [Boot sequence](boot-sequence.md), [Security](security.md), and [Development](development.md)
   when changing boot, authority, or build/proof behavior.
4. The affected subsystem page: [Core](CORE.md), [Platform](PLATFORM.md), [Console](CONSOLE.md),
   [Persistence](PERSISTENCE.md), [Network](NETWORK.md), [Remote](REMOTE.md), [Sessions](SESSIONS.md),
   [Applications](APPLICATIONS.md), [Experience](EXPERIENCE.md), or [Update](UPDATE.md).

## Optional references

- [Onion rings](ONION_RINGS.md): compact placement rationale; the [detailed reference](optional/ONION_RINGS.md)
  is optional.
- [Naming register](NAMING.md): consult only when introducing or renaming a subsystem; extended
  rationale is in [the optional reference](optional/NAMING.md).
- [Architecture detail](optional/architecture.md), [Persistence detail](optional/PERSISTENCE.md), and
  [archived design detail](optional/DESIGN.md): optional deep references.
- [Flow](FLOW.md): deferred language and automation charter; not an implementation contract. Its
  [full specification](optional/FLOW.md) is preserved for design work.
- [Filesystem ideas](suggestions_filesystem.md), [wish list](wish-list.md),
  [hardware ideas](hardware_ideas.md), [study notes](study_considerations.md), and
  [Design](../DESIGN.md): exploratory or historical material.

## Decisions and evidence

- Read [the ADR index](adr/README.md) and only the ADR relevant to the boundary being changed.
- Read [test status](../testing/STATUS.md) for current proof evidence.
- `review/` and `reviewed/` contain historical evidence; they are not active requirements.

## Maintenance rule

Active docs state ownership, contracts, invariants, exit proofs, and explicit deferrals. Completed
phase checklists, duplicated rationale, and speculative designs belong in ADRs or reviewed history.
