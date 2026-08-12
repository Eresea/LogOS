# ADR-0017: Fixed glyph cache

## Status

Accepted

## Decision

Display rasterization resolves terminal cell scalars through a fixed 8×16
glyph cache with at most 1,024 entries. The embedded baseline covers the
ASCII letters, digits, and terminal punctuation needed by the initial shell;
invalid or unsupported scalars use one deterministic U+FFFD-style replacement
glyph.

Shaping, bidi, combining-mark layout, dynamic font loading, and external font
dependencies remain out of scope for the first graphical terminal.

## Consequences

- Glyph lookup has bounded memory and deterministic eviction.
- The cell model remains Unicode-scalar based without putting font state in
  Terminal or the kernel.
- Additional glyph coverage can be added by replacing the embedded table
  without changing the service protocol.
