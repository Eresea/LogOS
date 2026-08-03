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

## Network endpoints

Network bind, send, and receive use separate revocable capabilities. Each v1 grant authorizes one
exact UDP local port or one exact IPv4 remote endpoint; ICMP echo authority names one exact remote
IPv4 address. Core rejects wrong-kind, wrong-scope, stale, revoked, and wrong-owner requests before
Network wake-up, page loan, protocol-state change, or NIC access. Endpoint handles are owner- and
generation-bound. Reset, service restart, client exit, or revoked bind authority invalidates them
and returns all page loans. Replies are accepted only when their request ID, operation, endpoint
generation, interface generation, lengths, and source metadata match the outstanding request. Core-owned
bounce buffers prevent the Network service from programming DMA, but IOMMU protection from a malicious
physical device remains deferred.

## Remote Foundation

Remote Foundation accepts one client static X25519 key only after a local enrollment effect. The
The trust owner derives separate device and storage keys from the UEFI root, seeds ephemeral
generation from firmware entropy, and wipes the root before normal service startup. The
Gateway uses Noise IK with the machine static key held by the System trust owner; it receives no
long-term key material and receives only a `Service` capability for `ping`. TCP authority is scoped
to local port 7443, while accepted stream handles are owner- and generation-bound. Enrollment and
remote-session records are separately derived-key, authenticated-encrypted Store objects. A failed
authentication, malformed Noise message, stale sequence, or protected-record failure denies remote
work before Session dispatch; corruption disables remote access rather than falling back. Revocation
advances the enrollment generation and invalidates the in-memory session record. A pending durable
command is never replayed after reset.

## Native-service fault containment

Ring-3 CPU faults and typed service panics return to Core with a normalized failure record. Core
invalidates task, endpoint, request, and page-loan generations before constructing a replacement
from immutable staged bytes. Replacement maps only declared service resources. A fault in Core
retains the fatal halt path; timer preemption of an uncooperative service remains deferred.
