# Boot process

## Hosted boot (v3)

1. **`logos-host` starts.** It creates the shared Display, Input and per-service `bootstrap`/`ipc`
   windows, fills the framebuffer config and present page, and zeroes the input rings.
2. **Core spawns each service image** in boot order with the window tokens in its environment and
   its stdin/stdout as the syscall link.
3. **The child maps its windows** at the fixed ABI addresses and enters the unchanged `_start`.
4. **Core serves syscalls** per service on a dedicated thread; the main thread presents the
   framebuffer whenever the present sequence changes.
5. **The service graph** (discovery, IPC, events, heartbeat, restart) is the active milestone; until
   it lands the services idle after their first directory lookup.

## Bare-metal boot (v2, frozen)

Boot has three owners in one direction: **UEFI → Core → Runtime**. Ownership never moves backward.

1. **UEFI enters Core on the bootstrap CPU.**
2. **Core uses UEFI services to prepare the handoff.** It discovers a bounded set of healthy CPUs,
   establishes timing, reserves the AP startup page, loads the five bounded service ELF images from
   the image ESP, and prepares bootstrap-CPU local state.
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
9. **Atrium establishes the GUI shell.** After Shell and LockScreen are available, Atrium publishes
   the bounded Boot/Locked/Home route, creates its delegated Display surface, and starts in the
   locked or first-boot claim section.
10. **Authentication enters Home.** LockScreen sends a bounded User request through Shell. A
    successful first-boot claim returns LockScreen to the login route; a valid explicit login session
    moves Atrium to Home and hides LockScreen. System then requests its read-only management surface;
    logout, restart, stale responses, and
    reboot discard all Atrium surfaces and return to the locked section.

## Structural checks

- Firmware-dependent work occurs before the UEFI exit boundary.
- A CPU joins the scheduler only after its local stack, descriptor tables, interrupt controller, and
  timer are ready.
- Runtime starts only after all CPUs and the scheduler are ready.
- Application CPUs do not own global boot policy; they initialize themselves and report readiness.
- Every wait and resource count is bounded; unrecoverable boot failures use the single fatal path.
