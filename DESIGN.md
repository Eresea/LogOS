# LogOS Design Notes

> **Status:** Historical, optional reference
> **Original target:** pre-Console-v1 architecture improvements

The former design document was a large proposal for allocator growth, service manifests, a HAL,
structured errors, async IPC, testing, CI, and other future work. It is not an active requirements
list; several proposals are implemented, deferred, or superseded by the current ABI-v4 architecture.

Use these documents instead:

- [Documentation map](docs/README.md) for the smallest reading path;
- [Roadmap](docs/roadmap.md) and [TODO](docs/TODO.md) for active work;
- [Architecture](docs/architecture.md), [Milestone policy](docs/MILESTONE-POLICY.md), and the relevant
  [ADR](docs/adr/README.md) for boundaries and irreversible decisions;
- [Development](docs/development.md) and [testing status](testing/STATUS.md) for verification.

Do not revive a proposal from the retired notes without an explicit roadmap entry, bounded scope, and
an ADR when it changes a cross-ring or runtime boundary.
