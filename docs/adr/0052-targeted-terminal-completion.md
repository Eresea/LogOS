# ADR-0052: Targeted terminal completion

Status: Accepted

## Decision

Completion is an optional Commands-owned sub-service exposed through the existing Session↔Commands
IPC queues. It is enabled by default by the Session constructor and adds no ring-3 service, endpoint,
or dependency. Requests and responses use fixed-size ABI records with bounded line contents,
replacement ranges, candidate counts, and candidate bytes.

The provider resolves only targeted Flow expression fragments: root expressions, live Core manager
service names, service members, network members, and the fixed `eth0` interface entry. Filesystem
paths and arbitrary method arguments remain deferred.

Completion is request-local and best-effort. A malformed response, provider error, queue failure, or
timeout emits one `completion unavailable` diagnostic, disables completion for that Session, and
leaves input editing and command execution operational. A crashed Commands process is not treated as
a completion-only failure and follows the existing graph-restart policy.

## Consequences

Session owns the bounded ghost/list interaction and stale-response checks. Terminal supports only the
vertical cursor movement needed to redraw that list. No scrolling or general popup framework is added.
