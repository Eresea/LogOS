# ADR-0051: Terminal render batching

Status: Accepted

## Decision

Terminal render messages use the existing bounded IPC flag space to mark when
more cell chunks follow. Display coalesces those chunks and paints the final
cell state once, preventing multiline output and screen replacement from
appearing line by line. A `FullRedraw` batch clears the complete framebuffer
before its cells are painted.

The terminal remains a fixed 80×25 cell surface with bounded render messages.
Interactive scrollback and user-facing scrolling remain deferred.
