# Bounded task contract template

Copy this into every task that crosses a service, device, or privilege boundary. Keep it with the
task/issue or its reviewed design note; it is not a new runtime API.

```text
Owner:                 subsystem that owns the suspended operation and its cleanup
Capability:            exact grant checked before acceptance (and its resource scope)
Generation:            identity checked on every resume, completion, cancel, and replacement
Resource bound:        fixed slots/pages/bytes/queues; exhaustion result
Timeout:               authoritative clock/deadline and terminal timeout transition
Cancellation:          caller/owner allowed to cancel; exact resources reclaimed
Failure/restart:       terminal status, stale-work rule, and restart/rebind owner
Modified boundary:     concrete endpoint/service/ring seam changed by this task
Frozen boundaries:     contracts and behavior intentionally not changed
Reference implementation: existing bounded state machine to follow
Tests:                 fast host acceptance tests and their exact assertions
Proof IDs:              required QEMU/fault-containment proofs, or an explicit reason none apply
```

An operation is one fixed slot plus typed phase state. Validate capability before accepting it;
retain owner, generation, request identity, deadline, and loan/resource state while suspended;
make completion, timeout, cancellation, fault, and replacement terminal transitions that reclaim
the slot. Notifications only request scheduler work; they never replace the authoritative state.
