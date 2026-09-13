# ADR-0084: Bounded native Material Symbols

Status: Accepted

## Decision

The UI framework represents supported Google Material Symbols through a fixed
`UiIcon` value and the existing retained graphics ABI. Display rasterizes each
symbol as one bounded `MaterialSymbol` draw command; the first supported symbol
is `settings`. Buttons retain their text as the semantic label while an icon
replaces only their visual text paint.

The implementation is offline and native to the no-allocator display path. It
does not load a webfont, fetch symbol data, or add a runtime dependency.

## Fixed bounds and limits

Material symbol commands contain one validated symbol discriminator, a nonempty
rectangle, and no text payload. Unknown discriminators are rejected. UI scenes
retain the existing fixed node and operation limits, and the icon occupies one
button paint fragment.

Additional symbols and a larger icon catalog can be added as explicit bounded
enum values when concrete UI consumers require them. Dynamic font loading,
arbitrary SVG/path data, and network-backed symbol resolution remain deferred.
