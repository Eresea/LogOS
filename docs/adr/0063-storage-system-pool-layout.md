# ADR-0063: Storage system-pool layout

- Status: Accepted
- Date: 2026-08-20

## Context

User identity records need durable storage that cannot be consumed by ordinary user
content or package payloads. The current v4 volume has one data arena and a package
prefix, but changing that format in place would violate its fail-closed compatibility
boundary.

## Decision

The next filesystem format will persist three disjoint block ranges:

1. a system pool for User, role, and recovery metadata;
2. a user pool for namespace content;
3. a package pool for signed service payloads.

The validated layout uses the data arena's prefix for the system pool, the following
range for user content, and the format-selected package boundary for package payloads.
Zero-sized system or user pools are rejected. Allocation exhaustion in the user or
package pool must not make the system pool unavailable for recovery metadata.

The v5 system-catalog root persists the three ranges, uses dual-superblock publication
and commit records, and stores namespace metadata and the User snapshot as bounded extents
inside the system pool. Namespace file content is allocated only from the user pool, while
package payloads remain in the package pool.
The live v5 namespace implements `UserCatalogStore`, so namespace metadata and User catalog
updates share one root publication boundary; User does not receive a path or raw block handle.
The catalog and v5 namespace openers reject v4 roots. The v4 namespace opener
remains available for legacy host proofs until the service boot path is switched to the v5 alias.

## Consequences

- Storage owns pool boundaries; User never receives path-based or raw block authority.
- COW publication and torn-superblock recovery protect the catalog root.
- System-pool exhaustion is explicit and cannot fall through into user or package blocks.
- The v5 namespace backend is present; service-image boot selection and signed-package trust remain
  separate follow-up slices.
