# ADR-0025: Network readiness ownership and completion targets

- Status: Accepted
- Date: 2026-08-08

## Context

Core previously used the Terminal Network client page as a readiness probe. That made a
production Gateway start decision depend on a presentation client and made synthetic QEMU
transactions indistinguishable from live callers. Network replies also resumed Gateway but could
leave Terminal blocked for `Busy` or other immediate statuses.

## Decision

`platform::network::NetworkRuntime` owns cached Network readiness. After binding the Network
service, and only while no client transaction is active, it submits an internal `Status` request
directly to the Network server endpoint. The reply updates `NetworkRuntime::info()` and
`NetworkRuntime::configured()`. Gateway startup consumes `configured()`; QEMU address literals
remain assertions in test code only.

Every production client completion targets its caller task. Publishing a reply always wakes and
runs that task for success, denial, busy, invalid, timeout, cancellation, reset, and I/O results.
Test-only white-box transactions use an explicit `Probe` completion target and never alter
production scheduling semantics.

## Consequences

- Terminal is no longer a hidden Network readiness dependency.
- Gateway and Terminal follow the same blocked-call completion invariant.
- Synthetic probes can inspect typed replies without scheduling a live service caller.
- No ABI change or generic status framework is introduced.
- QEMU proof runs must be repeated after this boundary repair.
