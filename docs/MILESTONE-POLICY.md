# Milestone Policy

This reference preserves roadmap-wide rules without loading them with every roadmap read.

## Architectural principles

1. Mechanism stays inward; policy moves outward.
2. Outer rings depend on inner contracts, never inner implementations.
3. Capabilities are explicit, narrow, policy-transferable, and revocable where practical.
4. Services own their state and resources; failure boundaries precede convenience APIs.
5. Local, remote, human, and agent operation share typed session and command contracts.
6. Structured values are native; text is presentation.
7. WASM is the default application sandbox; trusted native Rust is for hardware, boot, security policy, or measured latency needs.
8. Names communicate scope without binding implementation.
9. Every milestone ends with permanent automated QEMU proofs.
10. System capabilities compose independently releasable module slices; module versions do not advance in lockstep.

Compatibility belongs at the edges. The system remains recoverable, observable, and updateable without treating reboot as the normal failure response.

The architectural rings and placement rules live in [Architecture](architecture.md).

## Repository migration history

The kernel remained independently bootable while the workspace extracted `logos-core`, `logos-terminal`, `logos-abi`, `logos-service-rt`, and separately loaded Terminal and Sessions payloads. `logos-uefi` remains the boot binary. Each extraction retained the QEMU proof; stable ABI was added only when independently built services required it.

## Definition of done

Every milestone defines and tests ownership/reclamation, capabilities, cancellation, bounded resources, timeouts, versioning, structured errors, health, restart/recovery, privileged audit events, QEMU coverage, and non-goals.

A feature is complete only when its owner and versioned boundary are documented; every completion path reclaims resources; denial and failure are tested; diagnostics are actionable; and integration proofs cover exit criteria.

## Module independence

- Consumers depend on published contracts, never another module's implementation crate or private state.
- A module version names a contract and its guarantees, not a branch, crate revision, or implementation rewrite.
- Compatible implementation changes preserve the protocol identifier and version and must keep its contract proofs passing.
- An incompatible contract change introduces a new version. Consumers declare accepted versions and negotiate at discovery; adapters, when needed, live at the providing edge.
- Old and new implementations need coexist only for a documented migration or rollback proof.
- A capability milestone pins the required module slices and adds one composition proof. It does not merge their ownership, repositories, or release cadence.
- Each slice has its own owner, prerequisites, non-goals, host proof where practical, and QEMU proof at the hardware or isolation boundary.

See [ADR-0016](adr/0016-capability-slices.md).

## Continuous Core rule

Core work is pulled forward only for hardware privilege, a global ownership invariant, pre-service fault containment, or correctness that cannot safely live outward. Performance alone requires measurement.

Deferred Core candidates: APIC/HPET timing, MSI/MSI-X, SMP and per-CPU state, architecture separation, stronger property tests, persistent crash export, measured preemption, and IOMMU-backed DMA isolation.

## Current-cycle non-goals

- POSIX as the native architecture; arbitrary native third-party binaries; WASM in the kernel.
- Desktop, browser, or IDE work before remote, application, update, and recovery contracts.
- AI agents as trusted kernel actors; full real-hardware coverage before QEMU contracts stabilize.
- Unmeasured preemption, a monolithic shell, or thematic names that erase subsystem boundaries.

## Maintenance

Update the active milestone and exit criteria when implementation changes understanding. Update [Architecture](architecture.md) when ownership moves, [Naming](NAMING.md) when subsystem vocabulary changes, and add an [ADR](adr/README.md) for irreversible or cross-ring decisions. Preserve completed criteria as evidence; never rewrite past scope to disguise incomplete work.

Historical reviewed proposals remain under [`reviewed/`](reviewed/).
