# ADR-0056: Bounded package manifests and dependency policy

Status: Accepted

## Context

Package storage currently has a service-only v1 envelope. The next package-manager
slice needs service and program packages, versioned replacement, and package trees
without introducing an allocator, repository resolver, or unbounded dependency data.

## Decision

logos-package defines the shared v2 metadata contract:

- names are lowercase ASCII identifiers capped at 32 bytes;
- versions are major.minor.patch semantic versions;
- ranges are bounded npm-style exact, wildcard, caret, tilde, and comparator forms;
- each package has at most four unique dependencies;
- the catalog holds at most 16 installed package manifests;
- a replacement must be strictly newer and is rejected atomically if it breaks an installed dependent;
- only service-package dependencies participate in topological activation; program dependencies
  remain version prerequisites and are never launched as services;
- service packages carry one fixed ServiceId target, while program packages carry no service target.

The v1 service envelope remains readable during migration, but it does not receive
synthetic v2 metadata. Existing v1 packages can be replaced by a v2 install; v2
replacements must be strictly newer and preserve installed dependent ranges.

## Consequences

The policy is deterministic and host-testable with fixed memory. Storage validates v2
service headers and enforces replacement/dependency policy while retaining v1 records;
Core activation accepts both envelopes and keeps the existing generation-safe package
IPC shape.
