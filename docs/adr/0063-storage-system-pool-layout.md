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

This milestone adds only the no-allocator layout validator and host proof. v4 roots,
allocation scanning, and on-disk format versioning remain unchanged until the complete
filesystem upgrade is implemented. v4 media therefore continues to open or reject under
the existing rules and is never reinterpreted as the new layout.

## Consequences

- Storage owns pool boundaries; User never receives path-based or raw block authority.
- A future COW root can persist the three ranges without changing capability semantics.
- The system pool requires explicit capacity accounting and exhaustion tests before User
  persistence is wired into the service.
