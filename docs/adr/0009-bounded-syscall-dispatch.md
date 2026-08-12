# ADR-0009: Bounded syscall dispatch

- Status: Accepted
- Date: 2026-08-12

## Decision

Vector 49 is the software syscall gate. The kernel reads the saved register frame only for the
registered user process, accepts syscall number `Yield` (`rax = 1`), writes a zero result, and
rejects unknown user numbers as a fatal proof failure.

## Scope

The ABI has no user pointers, buffers, capability transfer, or general service calls yet. Those are
separate bounded extensions after the dispatch seam is proven.
