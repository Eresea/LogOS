# ADR-0053: Fixed Fetch service and staged Storage writes

- Status: Accepted
- Date: 2026-08-17

## Decision

Add Fetch as the eighth fixed ring-3 service. Fetch owns URL parsing, HTTP framing, download limits,
progress, cancellation, and coordination. Network remains a TCP/UDP transport boundary and Storage
remains the durable namespace owner.

Network ABI v2 carries request and response payloads inline with a 192-byte cap and preserves the
originating Commands or Fetch endpoint. Storage ABI v2 adds one volatile staged-write slot with
contiguous chunks and an atomic durable commit. Fetch has no manager capability and adds no syscall.

## Consequences

Fetch is bounded to one operation, numeric IPv4 HTTP, port 80 by default, 2xx responses, and the
existing Storage file limit. HTTPS, DNS, redirects, compression, authentication, trailers, and
background jobs remain deferred. Service restart abandons volatile Fetch state; Storage never exposes
staged bytes as a normal file.
