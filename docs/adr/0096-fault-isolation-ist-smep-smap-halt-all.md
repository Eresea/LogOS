# ADR-0096: Fault Isolation: IST Stacks, SMEP/SMAP, and Halt-All Fatal

- Status: Accepted
- Date: 2026-10-15

## Context

The kernel ran every vector on one stack and ignored the CPU's supervisor-mode protections. A kernel
stack overflow turned #DF into a triple fault with no output, `arch::fatal` stopped only the calling
CPU while the others kept scheduling, the panic handler discarded its `PanicInfo`, and a kernel
bug that jumped to or read a user page succeeded silently.

## Decision

- **Diagnostics.** The panic handler prints `file:line:column` and the message through a fixed
  240-byte `core::fmt::Write` buffer to the debug console; a re-entrant panic falls back to a fixed
  string. Architectural exceptions that are not task faults (everything except #UD/#GP/#PF, which
  keep the contained-fault path) use per-vector stubs with a uniform frame and report vector, error
  code, RIP, CS, RFLAGS, RSP and, for #PF, CR2 in hex. Fatal task faults print the same fields.
- **Halt-all.** The first `fatal` caller sets `HALTED`, prints, and sends an NMI to every other CPU
  whose IDT is loaded (tracked in a bitmask). NMI is used because it reaches CPUs spinning with
  interrupts disabled and needs no new vector. The NMI stub parks the CPU when `HALTED` is set;
  an NMI with `HALTED` clear is itself reported as fatal. The send path never calls `fatal`.
- **IST.** #DF (IST1), NMI (IST2) and #MC (IST3) run on dedicated 16 KiB per-CPU stacks, fixed
  statics sized `MAX_CPUS * 3`, installed in each CPU's TSS and reserved from the frame pool.
- **SMEP/SMAP.** CR4.SMEP (bit 20) and CR4.SMAP (bit 21) are set on the BSP and every AP when
  CPUID leaf 7 EBX bits 7/20 report them. `arch::with_user_access` brackets a user-VA dereference
  with `stac`/`clac` (a plain call when SMAP is absent). Today no kernel path dereferences a user
  virtual address: user buffers are reached through identity-mapped physical frames, which are
  supervisor pages. New code that touches a user VA must use the helper. The BSP prints
  `LogOS vNext: SMEP/SMAP enabled`; `scripts/proof.sh` runs QEMU with `-cpu max` so TCG exposes both.

## Consequences

- Stack-overflow #DF, NMI and #MC remain diagnosable. `-cpu max` is required for proof runs to
  exercise SMEP/SMAP; CPUs without them boot unprotected and print a distinct marker.
- No guard pages are placed below the IST or scheduler stacks. The kernel image lives in the
  firmware-owned identity map, so an unmapped page would need a large-page split and an owner for
  the kernel root; that is deferred until a milestone gives page-table ownership to Core.
  The existing scheduler-stack canary remains the overflow check.
- An AP that has not loaded its IDT is not sent the halt NMI; it keeps running until it does.
