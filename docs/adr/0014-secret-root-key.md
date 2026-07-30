# ADR-0014: Bootstrap durable-secret root key

- Status: Accepted
- Date: 2026-07-30

## Context

Persistence can encrypt secret objects before hardware-backed key storage exists, but machine
identity is public and cannot serve as an encryption key.

## Decision

Generate a random 256-bit root key from UEFI RNG and store it in a dedicated non-volatile UEFI
variable. Bind XChaCha20-Poly1305 ciphertext to the owner, machine identity, object name, and Store
version. If RNG or variable persistence is unavailable, durable secret writes are unavailable.

## Consequences

- Copying only the data disk does not reveal secret plaintext.
- Compromise of firmware variables defeats this protection; hardware-backed sealing remains future
  work.
- Core wipes its staging copy after delivering the key to the Secrets service.

## Alternatives considered

- Deriving from machine identity — rejected because the identity is not secret.
- Persisting an unencrypted key on the data disk — rejected because it provides no at-rest
  protection.
