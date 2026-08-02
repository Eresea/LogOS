# Sessions

> **Status:** Sessions v1 complete as part of Console v1
> **Owner:** Sessions

## Goal

Expose typed, capability-scoped operations independently of local, remote, graphical, human, or agent presentation.

## V1 — Typed local sessions

The implemented command, result, cancellation, timeout, backpressure, pipeline, and capability contracts are recorded in [Console](CONSOLE.md).

## V2 — Attach and resume

- Persistent jobs and typed streaming results.
- Authenticated remote attach, bounded reconnect, and resume.
- Flow execution, resource references, and event subscriptions.

Remote Foundation may consume only the attachment, reconnect, and stream-transport slice.

## V3 — Agent and rich-client sessions

- Typed tool descriptors and effect metadata.
- Agent-session provenance and approval continuity.
- Multiple concurrent presentation clients without presentation-specific command contracts.

The recovery console remains outside this module.
