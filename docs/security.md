# Security

## Model

LogOS is capability-first, not ACL-first. Identity answers who; capabilities answer what; secrets answer how something authenticates. Protected resources are accessed through unforgeable kernel capabilities that may be delegated, temporary, revoked, and audited.

Primary identities are users, services, applications, and AI agents. All receive immutable identifiers.

## Identity and secrets services

The Identity Service owns user and device identity, sessions, passkeys, certificates, OAuth/OpenID Connect, SSH identities, and hardware-backed identity where available.

The Secrets Service is the credential broker, not a browsable password database. It stores passwords, passkeys, API keys, OAuth tokens, SSH keys, certificates, and encryption keys in encrypted vaults. Applications request operations such as `SignChallenge`, `GetOAuthToken`, or `AuthenticateToWebsite`; the service should authenticate without exposing the underlying secret whenever possible.

AI agents receive narrowly scoped capabilities and never receive a secret by default. Browsers integrate with the service instead of owning separate credential stores.

## Persistence namespaces

The Store API is capability-scoped. Terminal receives separate read and write authority limited to
`TERMINAL_NAMESPACE`; requests for text, audit, secrets, or any other namespace are denied before
Store wake-up or disk access. Read-only, revoked, stale, malformed, and wrong-owner capabilities
are rejected at the Core relay. Client transfer pages are generation-tagged and owner-checked,
loaned only for the request, and returned on both success and failure. Store Block traffic uses its
own transfer page and principal, so a client page cannot be substituted for the Store page.
