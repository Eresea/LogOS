# ADR-0016: ELF page admission

## Status

Accepted

## Decision

An accepted `ElfLoadPlan` is converted into a fixed `LoadedImage` containing
page-aligned virtual mappings, owned physical frames, entry point, and an
eight-page user stack. Segment overlap, address overflow, frame exhaustion,
and mapping capacity fail the admission and release every frame acquired by
that attempt.

The current slice records ownership and mapping intent. Hardware page-table
construction and segment byte copying are separate steps so they can be
verified without dereferencing arbitrary physical addresses in host tests.

## Consequences

- Process admission can be rolled back without leaking physical frames.
- Large service state remains outside fixed scheduler stacks.
- The implementation still cannot run a loaded image until the architecture
  page-table and scheduler-binding slice is complete.
