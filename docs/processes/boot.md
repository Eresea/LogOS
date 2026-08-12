# Boot process

Boot has three owners in one direction: **UEFI → Core → Runtime**. Ownership never moves backward.

1. **UEFI enters Core on the bootstrap CPU.**
2. **Core uses UEFI services to prepare the handoff.** It discovers a bounded set of healthy CPUs,
   establishes timing, reserves the AP startup page, and prepares bootstrap-CPU local state.
3. **Core exits UEFI boot services.** This is the irreversible ownership boundary; no later stage may
   depend on firmware services.
4. **Core prepares the bootstrap CPU.** It installs CPU-local execution and interrupt state, then
   enables and calibrates the local interrupt controller and timer while interrupts remain disabled.
5. **Core starts each application CPU.** Each CPU installs equivalent local state, joins the scheduler,
   declares itself online, and enters the scheduler idle path. Startup is bounded and failure is fatal.
6. **Core completes scheduler startup.** The bootstrap CPU arms its timer, joins the scheduler, and
   waits until every discovered CPU is online.
7. **Core hands off to Runtime.** Runtime is registered as an ordinary root task; it is not called by
   firmware and does not bypass the scheduler.
8. **The bootstrap CPU enters the scheduler.** From here, timer interrupts and explicit task actions
   drive execution.

## Structural checks

- Firmware-dependent work occurs before the UEFI exit boundary.
- A CPU joins the scheduler only after its local stack, descriptor tables, interrupt controller, and
  timer are ready.
- Runtime starts only after all CPUs and the scheduler are ready.
- Application CPUs do not own global boot policy; they initialize themselves and report readiness.
- Every wait and resource count is bounded; unrecoverable boot failures use the single fatal path.
