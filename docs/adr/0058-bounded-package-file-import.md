# ADR-0058: Bounded package-file import

Status: Accepted

## Context

The package arena already owns durable validation, generation replacement, and dependency
policy, but Flow needs a bounded way to request an install/update from a file already owned by
Storage.

## Decision

Add `PackageInstall` to the versioned Flow↔Storage API. The request names an existing regular
file; Storage identifies its v1/v2 service target, copies it block-by-block into the package
arena, and commits it through the existing package validator and update policy. A regular
transaction or staged write makes the request return `Busy`. The file remains in the ordinary
namespace and no package bytes are accepted directly from Flow.

## Consequences

Flow exposes `pkg.install("/path")`. The operation supports service-package replacement while
preserving strict-newer and dependent-range checks. Repository resolution, signatures, larger
package staging, and program-package installation remain deferred.
