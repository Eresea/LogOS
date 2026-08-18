# ADR-0057: Read-only package inventory IPC

Status: Accepted

## Context

The package catalog is durable in Storage, but package activation remains a Core-owned
operation. Flow needs bounded visibility into installed versions and dependency edges without
receiving package extents or gaining install authority.

## Decision

Extend the versioned Flow↔Storage API with two read-only operations:

- `PackageList` returns one bounded, formatted package summary per cursor position;
- `PackageInfo` returns one formatted summary for a v2 package name.

The response is text-only and capped by the existing IPC response data bound. Legacy v1 packages
remain listable by service name and numeric package version, but have no synthetic manifest name.
Package payload transfer, install, update, signatures, and program packages remain outside this
surface.

## Consequences

Flow exposes `pkg.list()` and `pkg.info("name")` as promise-returning read operations. Storage
continues to own manifest decoding and CRC validation, while Core retains ownership of service
package activation and the existing package payload channel.
