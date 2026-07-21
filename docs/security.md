# Security

## Model

LogOS is capability-first, not ACL-first. Identity answers who; capabilities answer what; secrets answer how something authenticates. Protected resources are accessed through unforgeable kernel capabilities that may be delegated, temporary, revoked, and audited.

Primary identities are users, services, applications, and AI agents. All receive immutable identifiers.

## Identity and secrets services

The Identity Service owns user and device identity, sessions, passkeys, certificates, OAuth/OpenID Connect, SSH identities, and hardware-backed identity where available.

The Secrets Service is the credential broker, not a browsable password database. It stores passwords, passkeys, API keys, OAuth tokens, SSH keys, certificates, and encryption keys in encrypted vaults. Applications request operations such as `SignChallenge`, `GetOAuthToken`, or `AuthenticateToWebsite`; the service should authenticate without exposing the underlying secret whenever possible.

AI agents receive narrowly scoped capabilities and never receive a secret by default. Browsers integrate with the service instead of owning separate credential stores.
