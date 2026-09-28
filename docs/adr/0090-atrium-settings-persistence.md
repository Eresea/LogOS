# ADR-0090: Atrium settings persistence through User's snapshot

Status: Accepted

## Decision

Atrium persists one fixed-size, versioned settings record -- keyboard layout, mouse acceleration,
accent, the FPS overlay, reduced motion and light theme, with room left for a later field -- across
reboots (S4, #81). There are no per-user profiles, just one system record.

Atrium has no existing path to Storage (direct or indirect): its boot dependencies are Input,
Display, Shell, LockScreen and Terminal, and the only services with a Storage-facing IPC endpoint
are Flow, Fetch, Core and User. Rather than adding a new Atrium-to-Storage edge, Atrium reaches
Storage through User, reusing the canonical-snapshot path User already has (ADR-0064):

- A new bounded, point-to-point `AtriumToUser`/`UserToAtrium` endpoint pair (`IpcEndpointId` 66/67,
  contracts `IPC_CONTRACT_ATRIUM_SETTINGS_REQUEST`/`_RESPONSE`) carries a single `Load` or `Save`
  request and its response. One bounded message, no chunking: the encoded record is 16 bytes, far
  under the chunk size User already uses when talking to Storage. The pair is wired through the same
  generic, data-driven boot-capability loop every other endpoint uses (`ipc_message_size`,
  `ipc_contract_id`, `IpcEndpointId::producer`/`consumer`); no bespoke kernel wiring was added.
- User stores the encoded record as an opaque tail section of its own canonical snapshot
  (`UserCatalog`, ADR-0064): `encode_snapshot` appends it after the existing user/role slots, and
  `restore_snapshot` reads it back tolerantly -- a pre-S4 snapshot simply ends before that section
  and decodes as all-zero bytes, which Atrium's own decode already treats as "no valid record" and
  falls back to defaults. This is a tail extension, not a version bump: it does not touch Storage's
  own superblock/journal format (ADR-0048), which only ever sees User's snapshot as an opaque
  byte blob.
- User never interprets the record's bytes; `logos-atrium` (the host-tested crate) owns the layout,
  a 4-byte magic, a version byte, and an FNV-1a checksum over the header and fields. A short buffer,
  wrong magic, unknown version or checksum mismatch all decode to `AtriumSettingsRecord::DEFAULT` --
  corruption never becomes a crash or a silently wrong setting.
- Atrium loads the record once at boot (a bounded blocking exchange, mirroring User's own blocking
  boot-time load from Storage: User starts well before Atrium in `SERVICE_START_ORDER`) and applies
  it before its first render or Input push. It saves only when the encoded record actually changes
  (a Settings-page edit), not every frame, using the same non-blocking retry-on-`Full` pattern already
  used for the Atrium-to-Input settings push (ADR-0085).

This supersedes ADR-0085's "Settings are runtime-only" note: the same `InputSettings` push to Input
that ADR-0085 introduced still happens on every change, but the settings driving it now survive a
reboot.

## Consequences

- `ABI_VERSION` moves to 10 for the new endpoint pair and wire types (`AtriumSettingsRequest`/
  `AtriumSettingsResponse`, `AtriumSettingsOperation`, `AtriumSettingsStatus`).
- Atrium gains a boot dependency on User (`src/service_images.rs`); User gains a third IPC client
  alongside Flow and Shell, handled the same way (a non-blocking receive check per loop tick).
- A corrupt, short or unknown-version record, or a pre-S4 snapshot with no settings section, both
  fall back to `AtriumSettingsRecord::DEFAULT` -- the same defaults `Atrium::new()` already boots
  with -- rather than failing to boot or restoring a partially-decoded record.
- A future settings field (the ADR-0089 light theme already occupies one of the three reserved flag
  bits; three bytes of the 16-byte record remain reserved) is a record-layout change inside
  `logos-atrium`, not another ABI/endpoint change.
