# ADR-0003: Stage native services as versioned boot payloads

- Status: Accepted
- Date: 2026-07-28

## Context

Platform v1 must run `logos-terminal` as a separately built Sessions service. The current UEFI
image has no native image loader or service isolation mechanism, so treating an in-process callback
as a loaded service would not establish the required boundary.

## Decision

The first native-service loader increment will stage an independently built service image in the
QEMU boot payload. The UEFI boot binary asks firmware to validate the PE image, retains the loaded
image for the kernel, and validates a fixed header carrying the ABI version and service name before
exit from boot services. The PE image owns its entry point and image size; the header carries no
capability handles or hardware addresses.

The loader contract does not decide native-service address-space isolation. That remains the open
Core decision and must be settled before untrusted native code can run.

## Consequences

- Add the image header and validation before adding terminal-specific loader code.
- The QEMU payload must contain the service image and prove rejection of an invalid header.
- Do not create `logos-abi` until the independently built service needs shared Rust types.
- Keep the recovery console in the UEFI binary and retain its current QEMU proof.

## Alternatives considered

- Register an in-process terminal callback -- rejected because it is still linked into the UEFI
  image and has no load boundary.
- Define process isolation now -- deferred because the isolation model is an open architectural
  decision and is not needed to validate payload staging.
