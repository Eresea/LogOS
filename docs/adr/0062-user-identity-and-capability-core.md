# ADR-0062: User identity and capability policy core

Status: Accepted

## Context

LogOS needs identities and sessions without reproducing Unix ownership and mode bits. The current
Storage service is still a bounded v4 namespace and the current service graph is fixed, so the policy
must be independently testable before it is admitted into ring-3 IPC or persistent Storage.

## Decision

- User names are canonical lower-case bounded names; `UserId` and `RoleId` are generation-safe stable
  identifiers.
- Passwords are never persisted. The policy stores a versioned Argon2id verifier using 64 MiB,
  three passes, one lane, a 16-byte salt, and a 32-byte output.
- Roles persist capability templates, not live handles. Login materializes fresh capabilities into a
  volatile session; reboot and logout destroy sessions and capabilities.
- Namespace capabilities carry `Read`, `Write`, and `DelegableDerive` rights. Derivation can only
  attenuate rights and is tracked by a revocation lineage.
- Revocation invalidates a selected lineage and all descendants; unrelated sessions remain valid.
- Snapshot restore rejects malformed records and always starts with an empty session table.

## Consequences

The policy is capability-based and has no UID/GID mode checks or ambient path authority. The current
implementation is a host-tested `logos-user` core. `UserCatalogStore` is the explicit persistence
seam: Storage supplies the system-pool-backed load/save implementation and User only exchanges a
bounded snapshot. A follow-up cross-ring change must add the User IPC endpoint, Storage
system-reserve/catalog transaction, first-boot bootstrap grant, and service admission before claiming
a bootable User service.

## Verification

`cargo test -p logos-user` covers one-shot claim, Argon2id verification, canonical names, capability
attenuation, descendant revocation, and snapshot restore with volatile-session loss.
