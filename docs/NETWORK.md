# Network

> **Status:** Network v1 complete — bounded protocol core, DHCP/ARP/UDP service path, Core relay,
> hermetic QEMU resilience proofs, and restart cleanup complete
>
> **Owner:** Foundation network driver and System Network service

> **Milestone:** Network v1 complete; all promoted network proofs pass in the hermetic QEMU suite.

## Goal

Prove bounded, capability-controlled packet connectivity through a replaceable Network service
without making sockets, raw packets, or a general firewall part of the native v1 contract.

## Delivery rules

Completed items preserve evidence even when enabling work landed out of phase. Continue from
**Current implementation → Deferred work and next steps**, then close the remaining checklist
items in dependency order.

- Read the whole current phase before editing code.
- Read every caller of code changed in the phase.
- Fix an in-scope prerequisite, run its smallest proof, and continue the phase.
- Record unrelated, non-blocking work under **Deferred** and continue.
- Stop only for missing external access, missing hardware/tooling, or a new irreversible or
  cross-ring decision not settled by [ADR-0015](adr/0015-network-v1-boundary.md).
- Treat a failed build, test, boot, packet exchange, or recovery check as work to fix.
- Check an item only after its named automated proof passes.
- End every phase with `cargo fmt --check`, `cargo clippy -- -D warnings`, and the phase's smallest
  host or QEMU proof.
- Commit each independently bootable phase separately.

## Fixed v1 decisions

- Core owns VirtIO transport negotiation, RX/TX queues, DMA buffers, interrupts, timeouts, reset,
  and physical-address translation.
- The Ring-2 Network service owns Ethernet, ARP, IPv4, ICMP echo, DHCP, UDP, endpoint state, and
  protocol timers.
- Core passes complete Ethernet frames through two Network-owned shared pages. The device never
  DMA-writes service or client memory.
- Clients receive generation-tagged UDP endpoint handles, not raw packet access or POSIX sockets.
- `NetworkBind`, `NetworkSend`, and `NetworkReceive` are separate revocable capability kinds.
- Each capability carries an exact 64-bit network scope: protocol plus local port for bind/receive,
  or protocol plus remote IPv4 address and port for send. Wildcards and CIDR scopes are deferred.
- A denied request is rejected in Core before service wake-up, page loan, protocol-state change, or
  NIC access.
- Support one legacy transitional VirtIO network device with one RX queue and one TX queue.
- Require the device-provided MAC feature. Negotiate no checksum, segmentation, mergeable-buffer,
  control-queue, or multiqueue offload in v1.
- Use a 1500-byte IPv4 MTU, 1514-byte Ethernet frames without FCS, and UDP payloads of at most 1472
  bytes. Do not send or reassemble IPv4 fragments.
- Keep four Core-owned RX frame buffers, one Core-owned TX frame buffer, eight UDP endpoint slots,
  eight ARP entries, four queued UDP datagrams, and one pending client operation globally.
- Return `Busy` instead of allocating or growing a queue when a fixed in-flight limit is reached.
- The Network service remains allocation-free after startup; all tables and packet buffers are
  fixed-size and independent of traffic volume.
- Network failure never blocks local boot. The service reports `Offline` or `Degraded`, retries
  configuration on bounded timers, and leaves Terminal, Store, and recovery usable.
- Device reset or Network service restart invalidates endpoints, cancels pending operations,
  returns page loans, clears protocol state, increments the interface generation, and reacquires
  DHCP configuration. Clients must bind again.
- The automated QEMU peer is local and hermetic. Network v1 tests require no TAP device,
  administrator privilege, DNS, public internet, or host network configuration.

Changing a fixed decision requires updating this document and the architecture; changing the
cross-ring boundary requires superseding [ADR-0015](adr/0015-network-v1-boundary.md).

## Current implementation

- `logos-abi`, `logos-core`, and `logos-service-rt` contain the bounded Network request, reply,
  capability, wait, event, and fixed-page contracts.
- `logos-net` contains allocation-free Ethernet, ARP, IPv4, UDP, ICMP, DHCP parsing/encoding, and
  bounded endpoint, ARP, datagram, pending-operation, reset, and DHCP state.
- The kernel binds the legacy VirtIO network device when available, maps two Network-owned pages,
  loads the Ring-2 payload, and delivers one validated RX frame event at a time.
- `logos-network-service` handles bounded device transport, DHCP acquisition, ARP resolution,
  `Status`, `Bind`, `SendTo`, `ReceiveFrom`, `Close`, and `Cancel` state operations. Core validates
  capability scope and copies client payloads through the fixed Network TX page. Cancellation,
  close, reset, and ARP single-flight cleanup release pending client state.
- QEMU uses deterministic user-mode DHCP for the transport proof. The independent raw-Ethernet
  DHCP peer drives `network/configuration`; `network/device-bind` exercises a real Bind request
  through the Core capability and service relay, and `network/unauthorized-operation` proves
  denied Bind/SendTo/ReceiveFrom requests stop in Core.

### Transport milestone: DHCP over Core-owned VirtIO

This milestone completes transport only. The native service gate now carries bounded `Info`,
`Transmit`, and `Reset` requests/replies, timestamped timer events, fixed-page RX/TX frames, and
one in-flight TX completion. The Network service encodes Discover/Request frames, strictly checks
Offer/Ack responses, and reports `Offline` until DHCP binds.

Completed proof items:

- [x] Core-owned NIC info, generation validation, TX submission/completion, reset, and finite
      deadlines.
- [x] One timestamped RX, TX-completion, or timer delivery per Network wake.
- [x] Fixed Network RX/TX pages used for DHCP frames without allocation.
- [x] DHCP Discover/Request encoding, Offer/Ack validation, retry, renewal, rebinding, NAK, and
      lease-expiry state transitions.
- [x] Deterministic QEMU user-mode DHCP: `10.0.2.0/24`, guest `10.0.2.15`, gateway `10.0.2.2`.
- [x] QEMU proof `network/transport-dhcp` asserts the structured bound configuration and rejects
      malformed or stale DHCP responses before `Bound`.

QEMU's built-in DHCP server remains the transport proof source; the independent peer covers the
configuration and client paths. Host tests cover fixed-slot exhaustion, ARP expiry, exact matching,
and failure cleanup. The `Bind`/`SendTo`/`ReceiveFrom`/`Echo` relay, capability gate, cancellation,
timeout, malformed-frame, and reconnect paths are live.

### Milestone close

The permanent Network suite is `cargo run -p logos-test -- suite network`; the PR suite includes it.
Each promoted proof requires a structured reply and exact endpoint or payload assertion. Fault-heavy
peer behavior remains test-only and does not expand the production ABI.

## Contract and invariants

### Core device boundary

Core exposes a bounded frame interface only to the Network service:

- `Info` returns MAC address, MTU, link state, interface generation, counters, and fixed limits.
- `Transmit` consumes one complete Ethernet frame from the Network TX page.
- `Receive` is a Core-delivered event containing one frame copied into the Network RX page.
- `Cancel` cancels a matching unsubmitted transmit request.
- `Reset` quiesces the device, reclaims every descriptor and DMA page, rebuilds both queues, posts
  clean RX buffers, and increments the interface generation.
- Every request and reply carries a nonzero request ID; replies must match the active request.
- Nested frame transmit/reply traffic preserves the parent client request ID without reusing it as
  the device request ID.
- `Transmit`, `Cancel`, and `Reset` carry finite monotonic deadlines. An expired request is never
  submitted to hardware.
- A TX completion is success only for the matching descriptor chain and generation.
- An RX completion must name a posted descriptor and report a length from the VirtIO header through
  one complete frame. Unknown IDs, duplicate completions, impossible lengths, or queue-index jumps
  fail and reset the device.
- RX DMA pages are zeroed before every post. Core copies only the validated frame length and clears
  the unused portion of the Network RX page before delivery.
- Core keeps completed frames in the four-buffer RX pool until delivery. When all four are occupied,
  it drops new frames, increments `rx_dropped`, and keeps the device and service live.
- Core alternates ready RX delivery with client request delivery so sustained input cannot starve
  endpoint operations.

The Network service uses its existing native-service context as one multiplexed gate. It does not
post a long-lived receive request that would block clients. While waiting, it supplies its next
monotonic deadline; Core wakes it for one frame, one client request, reset, cancellation, or timer
expiry.

### Client datagram boundary

The v1 operations are:

- `Status`: return interface generation, link/configuration state, MAC, IPv4 address, subnet mask,
  router, MTU, and bounded counters.
- `Bind`: reserve one nonzero UDP port and return an owner-bound, generation-tagged endpoint handle.
- `SendTo`: send one payload from a bound endpoint to one exact capability-scoped IPv4/UDP peer.
- `ReceiveFrom`: return one payload plus source IPv4 address and port for a bound endpoint.
- `Echo`: exchange one ICMP echo with one exact capability-scoped IPv4 peer for diagnostics.
- `Cancel`: cancel the caller's matching pending `SendTo`, `ReceiveFrom`, or `Echo` request.
- `Close`: cancel pending work, discard queued datagrams for the endpoint, and invalidate its handle.

Rules common to every operation:

- Decode integer wire fields before converting enums; reject unknown discriminants and nonzero
  reserved fields.
- Validate field combinations, checked lengths, request ID, handle generation, page ownership,
  scope, and deadline before changing state.
- `SendTo` requires exactly `1..=1472` payload bytes in a readable loaned page.
- `ReceiveFrom` requires a writable loaned page and returns only the exact copied length.
- Return every page loan on success, denial, invalid input, cancellation, timeout, service exit,
  and device reset.
- A second pending client operation returns `Busy`; it does not overwrite or implicitly cancel the
  first operation.
- A second bind of the same UDP port returns `AddressInUse`. Port zero and automatic ephemeral-port
  assignment are deferred.
- Handles are valid only for their owner, interface generation, and live endpoint slot.
- Bind authority is rechecked while an endpoint is used. Revoking it closes the endpoint. Send and
  receive authority are checked on every operation.
- Revocation cancels queued or unsubmitted work. A frame already submitted to the NIC cannot be
  recalled; completion and audit state must report that fact without claiming it was prevented.
- Malformed, unauthorized, stale, late, and mismatched requests receive structured errors and never
  become success.

The v1 status set is `Complete`, `Denied`, `Invalid`, `Busy`, `Full`, `Offline`, `NoRoute`,
`AddressInUse`, `MessageTooLarge`, `TimedOut`, `Cancelled`, `Reset`, and `Io`.

### Protocol behavior

All multibyte network fields are big-endian. Parsers accept byte slices, perform checked arithmetic,
and never cast untrusted bytes to Rust enums or packed structs.

#### Ethernet and ARP

- Send Ethernet II frames and pad frames shorter than 60 bytes; no FCS crosses the driver boundary.
- Accept only the configured unicast MAC and broadcast destination. Drop unknown EtherTypes, VLAN
  tags, multicast, and oversized frames with a bounded counter.
- Accept only Ethernet/IPv4 ARP with hardware/protocol lengths 6/4 and request/reply opcodes.
- Reply to valid ARP requests for the configured local IPv4 address.
- Learn an ARP reply only when it matches an outstanding resolution target. A request for the local
  address may refresh its sender entry; unrelated ARP traffic cannot replace a live entry.
- Resolve the destination directly when it is inside the configured subnet; otherwise resolve the
  configured router. Return `NoRoute` if no router exists for an off-subnet destination.
- Retry ARP three times at one-second intervals, then fail the pending operation. Expired entries are
  reused before the oldest live entry.

#### IPv4 and ICMP

- Validate version, header length, total length, header checksum, destination address, and payload
  bounds before dispatch.
- Accept IPv4 options by honoring the header length but do not interpret them.
- Drop packets with `MF` or a nonzero fragment offset; never expose a partial datagram.
- Send with TTL 64 and a monotonically wrapping identification field.
- Validate ICMP checksums. Support echo request and echo reply only.
- Reply to a valid echo request for the configured address and allow one outbound echo operation at
  a time. Match replies by peer, identifier, sequence, and interface generation.

#### DHCP

- Use client port 68 and server port 67 with the device MAC as the client identifier.
- Implement `Init`, `Selecting`, `Requesting`, `Bound`, `Renewing`, and `Rebinding` states.
- Validate BOOTP fields, transaction ID, client MAC, DHCP message type, server identifier, offered
  address, and every option length before accepting an offer or acknowledgement.
- Require a lease time and contiguous subnet mask. Router is optional; DNS options are ignored.
- Use server-provided T1/T2 values when valid; otherwise use one-half and seven-eighths of the lease.
- Use capped `1, 2, 4, 8` second acquisition retries. Continue retrying every eight seconds while
  offline without blocking local boot.
- On NAK or lease expiry, clear address, route, ARP, endpoint, queued datagram, and pending operation
  state before returning to `Init`.
- A device or service reset starts a new transaction and never reuses the previous lease as if it
  were still valid.

#### UDP

- Validate IPv4 payload length, UDP length, destination port, and every nonzero UDP checksum. Accept
  a zero UDP checksum because IPv4 permits it; generate a checksum on every outbound datagram and
  encode a computed zero as all ones.
- Route only to an endpoint bound to the exact destination port and current interface generation.
- Deliver directly to a matching pending receive or enqueue in the four-datagram global queue.
- Drop a datagram when no endpoint exists or the receive queue is full; increment distinct counters.
- Preserve payload bytes exactly. Ethernet padding and bytes after the IPv4 total length are never
  included.

### Lifecycle and diagnostics

- Start after monotonic time and the VirtIO network driver are available; do not depend on Store,
  Terminal, DNS, secrets, or wall-clock time.
- Report `Offline` until DHCP reaches `Bound`, `Online` while the lease is usable, and `Degraded`
  after a recoverable driver or protocol failure.
- Keep counters for RX/TX frames, RX/TX bytes, malformed frames, unsupported frames, RX pool drops,
  UDP no-endpoint drops, UDP queue drops, timeouts, cancellations, resets, and denied operations.
- Counters saturate instead of wrapping and reset only when the Network service generation changes.
- Emit one bounded diagnostic for bind, online, lease loss, timeout, reset success/failure, service
  restart, and resource reclamation. Do not log payload bytes.
- Audit capability denial, endpoint bind/close, reset, and service restart with principal, scope,
  request ID, and outcome; never audit packet payloads.
- If Network startup or restart fails, mark it unavailable and leave local services and the direct
  recovery console operational.

## V1 implementation checklist

### Phase 1: freeze the contracts and test seams

- [x] Accept ADR-0015 for Core-owned NIC DMA and the Network-service datagram boundary.
- [x] Add `NetworkScope`, endpoint handles, interface information, operations, requests, replies,
      statuses, limits, and counters to `logos-abi`.
- [x] Widen only the internal capability resource field needed to store a 64-bit network scope;
      preserve existing 32-bit scoped capability helpers.
- [x] Add `NetworkBind`, `NetworkSend`, and `NetworkReceive` capability kinds.
- [x] Define one checked validator for every operation and valid field combination.
- [x] Add checked wire conversion for every enum and status.
- [x] Add Network request/reply encoding to the native-service context without changing its page
      size.
- [x] Add the finite-deadline Network wait/event operation to the existing service gate.
- [x] Match replies by request ID, endpoint generation, owner, and interface generation.
- [x] Add `logos-abi` tests covering every accepted operation shape and exact scope encoding.
- [x] Add rejection tests for unknown enums, reserved fields, zero IDs, stale handles, invalid page
      use, length overflow, zero/expired deadlines, and mismatched replies.
- [x] Run `cargo test -p logos-abi -p logos-core -p logos-service-rt`.
- [x] Run the normal headless boot proof.
- [x] Commit the ABI and gate contract.

### Phase 2: build the allocation-free protocol core

- [x] Create `logos-net` as a `no_std`, host-testable library with no allocator or runtime
      dependency.
- [x] Implement checked Ethernet, ARP, IPv4, ICMP echo, DHCP, and UDP parsing and encoding.
- [x] Implement Internet and UDP pseudo-header checksums without alignment assumptions.
- [x] Keep parsing separate from state transitions so malformed frames cannot partially mutate
      protocol state.
- [x] Model protocol progress as input frame/timer events producing bounded frame/reply actions.
- [x] Keep every table and output buffer caller-owned or fixed-size.
- [x] Add one truncation test that feeds every strict prefix of each valid frame and requires error.
- [x] Add table tests for bad lengths, checksums, fragmentation, options, duplicate/truncated DHCP
      options, unsupported protocols, Ethernet padding, and maximum UDP payload.
- [x] Use independent RFC byte vectors, not frames produced by the encoder under test, for parser
      expectations.
- [x] Round-trip each encoder through the parser only as a secondary test.
- [x] Run `cargo test -p logos-net` and `cargo clippy -p logos-net -- -D warnings`.
- [x] Run the normal headless boot proof.
- [x] Commit the protocol core.

### Phase 3: implement bounded protocol state

- [x] Implement the DHCP state machine, retry schedule, renewal, rebinding, NAK, and lease expiry.
- [x] Implement subnet routing and the fixed ARP cache with expiry and matching resolution.
- [x] Implement eight owner/generation-tagged UDP endpoint slots and unique port binding.
- [x] Implement four queued received datagrams and one pending client operation globally.
- [x] Implement exact-source receive results and exact-destination sends.
- [x] Implement ICMP echo request/reply matching and timeout.
- [x] Make cancellation, deadline expiry, endpoint close, lease loss, and reset release every held
      slot and buffer exactly once.
- [x] Add fake-clock host tests for every DHCP transition and retry boundary.
- [x] Add host tests for on-subnet/off-subnet routing, ARP retry/failure/expiry, endpoint exhaustion,
      duplicate bind, queue full, `Busy`, cancellation, timeout, lease loss, and generation reset.
- [x] Assert after every failure-path test that no request, page, endpoint, or datagram slot remains
      unintentionally held.
- [x] Run `cargo test -p logos-net`.
- [x] Run the normal headless boot proof.
- [x] Commit the bounded protocol engine.

### Phase 4: bind and recover the VirtIO network device

- [x] Discover PCI vendor/device `1af4:1000` and bind only the network interface class.
- [x] Enable bus mastering and verify an I/O BAR, queue availability, queue size, MAC feature, and
      stable device MAC before setting `DRIVER_OK`.
- [x] Configure legacy queue 0 as RX and queue 1 as TX with independent indices and descriptor
      ownership.
- [x] Use the existing checked contiguous VirtQueue allocation; unwind the first queue if the second
      queue or any DMA page allocation fails.
- [x] Allocate four RX DMA pages and one TX DMA page, each containing a separate VirtIO header and
      frame descriptor as required by the legacy layout.
- [x] Zero headers and RX data, post all RX buffers, then activate the device.
- [x] Keep offload fields zero and reject a device requiring unsupported behavior.
- [x] Route the device interrupt through the shared VirtIO vector and probe only its own ISR status.
- [x] Validate used-ring progress, descriptor IDs, generations, and lengths before reclaiming a
      buffer.
- [x] Repost a clean RX buffer immediately after its frame is copied or dropped.
- [x] Implement TX deadline, cancellation-before-submit, completion, reset, and counter updates.
- [x] Reset the device before returning a timeout for a frame already submitted to hardware.
- [x] Make reset quiesce first, invalidate pending work, rebuild both queues, repost RX, and expose a
      new interface generation.
- [x] Reclaim all queue pages and DMA pages on bind failure, reset failure, replacement, and shutdown.
- [x] Add Core self-checks for impossible completion IDs/lengths, RX pool exhaustion, stale
      generation, TX `Busy`, timeout-once behavior, partial-bind cleanup, and reset reclamation.
- [x] Add a QEMU bind proof that reads the configured MAC and receives one host frame.
- [x] Run `cargo test -p logos-core`.
- [x] Run `cargo run -p logos-test -- run network/device-bind`.
- [x] Commit the bootable NIC driver.

### Phase 5: start the Network service and acquire configuration

- [x] Create `logos-network-service` as a separately built Ring-2 payload using
      `logos-service-rt` and `logos-net`.
- [x] Keep the service independent of the VirtIO network driver; Core owns device access per ADR-0015.
- [x] Give it one RX page and one TX page, mapped writable and non-executable.
- [x] Configure both pages with the Network service's real principal; use no numeric owner literals.
- [x] Complete the service handshake before accepting a frame or client request.
- [x] Multiplex frame and client events through the single context gate.
- [x] Alternate ready frame and client delivery; deliver at most one event per wake.
- [x] Drive DHCP using the next-deadline wakeup and report state without busy-spinning.
- [x] Return to the wait gate while offline; do not block Terminal or recovery startup.
- [x] Add the Network payload to `scripts/run.ps1` and the boot image.
- [x] Add the hermetic QEMU DHCP peer with wire parsing independent of `logos-net`.
- [x] Promote `network/configuration` and prove DISCOVER/OFFER/REQUEST/ACK plus exact configuration.
- [x] Drop the first offer and prove the bounded retry sends a second discover at the specified
      boundary without sleeping in the guest.
- [x] Run `cargo run -p logos-test -- run network/configuration`.
- [x] Commit the online Network service.

### Phase 6: expose capability-scoped datagrams and echo

- [x] Relay only Network requests to the Network task and preserve request IDs end to end.
- [x] Check wire shape, capability kind, exact scope, owner, endpoint generation, page direction,
      and deadline before waking Network.
- [x] Implement bind, send, receive, cancel, close, and status in the service.
- [x] Return exact source metadata and payload length on receive.
- [x] Return page loans after the payload is copied, even while ARP or TX completion remains pending.
- [x] Reject a second pending operation as `Busy` without altering the first.
- [x] Close endpoints and cancel work when bind authority is revoked or the client exits.
- [x] Deny unauthorized Bind, SendTo, and ReceiveFrom before Network wake-up or NIC access.
- [x] Deny wrong kind, wrong local port, wrong remote address/port, stale, revoked, and wrong-owner
      capabilities before Network or NIC access.
- [x] Add Core tests for every denial and page-loan return path.
- [x] Add service tests for endpoint ownership, exact scopes, cancellation races, late replies, and
      close cleanup.
- [x] Promote `network/icmp-echo`, `network/udp-round-trip`, and `network/backpressure-cancel`.
- [x] Promote `network/unauthorized-operation`.
- [x] Prove host-to-guest and guest-to-host ICMP echo.
- [x] Prove a UDP payload in both directions with exact bytes and source endpoint.
- [x] Prove denial leaves Network counters, endpoint slots, page loans, and host-peer traffic
      unchanged except for the denial counter/audit event.
- [x] Run the four promoted QEMU proofs.
- [x] Commit the capability-scoped datagram API.

### Phase 7: prove loss, timeout, reset, and restart

- [x] Extend the host peer with deterministic drop, delay, duplicate, malformed, and capture actions.
- [x] Keep peer parsing independent of `logos-net`; do not let implementation and oracle share the
      same codec.
- [x] Add test-only virtual-time and reset controls through the existing `LOGOS/1` boundary.
- [x] Promote `network/packet-loss` with deterministic DHCP loss and malformed/duplicate ICMP frames;
      UDP and ARP failure cleanup remain covered by host state tests.
- [x] Assert bounded retry or the documented structured failure; never accept silent success.
- [x] Promote `network/timeout` and prove one deadline expiration produces one terminal reply and
      releases all held resources.
- [x] Promote `network/reset-reconnect`; reset before the proof and assert post-reset progress.
- [x] Assert every old endpoint and late completion is rejected after the generation change.
- [x] Assert DHCP reacquires configuration and a new endpoint completes a later UDP exchange without
      reboot.
- [x] Reuse the platform Network service restart containment proofs for idle/pending cleanup and
      pair them with `network/reset-reconnect` for configuration reacquisition and later progress.
- [x] Inject malformed frames and prove they are dropped without state mutation, panic, or response.
- [x] Preserve debug, control, QMP, QEMU stderr, and directional peer-frame logs for every failed
      case. Logs include metadata and lengths, never application payload contents.
- [x] Make every implemented Network proof fail if its semantic assertion is unavailable; no
      unconditional success marker or skipped scenario satisfies a checklist item.
- [x] Add completed Network proofs to the PR suite and keep fault-heavy repeats in nightly.
- [x] Run `cargo run -p logos-test -- suite network`.
- [x] Run `cargo run -p logos-test -- suite pr`.
- [x] Commit the Network v1 resilience proof.

### Phase 8: close the milestone

- [x] Check every v1 scope item and exit criterion against a passing proof ID.
- [x] Update `docs/ARCHITECTURE.md` if implementation changed the accepted boundary.
- [x] Update `docs/boot-sequence.md` with Network dependencies, non-blocking offline boot, and
      restart path.
- [x] Update `docs/security.md` with the implemented exact endpoint capability enforcement.
- [x] Update `docs/ROADMAP.md` to mark Network v1 complete.
- [x] Change this document's status to complete.
- [x] Run `cargo fmt --check`.
- [x] Run the prescribed host clippy checks with `-D warnings`.
- [x] Run the prescribed host-compatible workspace tests.
- [x] Run `cargo run -p logos-test -- suite network`.
- [x] Run `scripts/run.ps1 -Headless` and verify local services remain usable while the peer is absent.
- [x] Run `scripts/check.ps1`.
- [x] Commit the documentation-only milestone close separately.

## Automated proof matrix

| Proof ID | Layer | Required semantic assertion |
| --- | --- | --- |
| `network/transport-dhcp` | QEMU | Core TX submits Discover/Request; RX delivers Offer/Ack; final configuration is `10.0.2.15/24` via `10.0.2.2`; malformed/stale DHCP is not bound |
| `network/device-bind` | QEMU | Exact NIC class/MAC, four posted RX buffers, and one received host frame |
| `network/configuration` | QEMU | Valid DHCP acquisition, dropped-offer retry, and exact address/mask/router/lease |
| `network/icmp-echo` | QEMU | Valid echo in both directions with matching ID/sequence and checksum |
| `network/udp-round-trip` | QEMU | Exact payload and source/destination endpoints in both directions |
| `network/unauthorized-operation` | QEMU | Bind/send/receive denial occurs before service wake-up or NIC/page effects |
| `network/backpressure-cancel` | QEMU | Second request is `Busy`; cancellation returns resources and permits later progress |
| `network/packet-loss` | QEMU | Deterministic DHCP loss and malformed/duplicate ICMP traffic produce bounded progress |
| `network/timeout` | QEMU | Virtual deadline produces one timeout reply and no held resources |
| `network/reset-reconnect` | QEMU | Reset leaves DHCP-bound Network usable and echo succeeds without reboot |

Host tests remain the primary proof for parsers, checksums, state transitions, bounds, and malformed
input. QEMU tests prove the real PCI, interrupt, DMA, service-gate, capability, and recovery paths.
The host peer is the independent wire oracle. A proof passes only on structured state and byte-level
assertions; matching a diagnostic line alone is insufficient.

## V1 scope

### Device boundary

- [x] Discover and drive one legacy transitional VirtIO network device.
- [x] Keep DMA, queue, interrupt, timeout, reset, and reclamation in Core.
- [x] Deliver bounded complete Ethernet frames through Network-owned pages.

### Protocol service

- [x] Run one independently restartable Ring-2 Network payload.
- [x] Support Ethernet II, ARP, IPv4 without fragmentation, ICMP echo, DHCP, and UDP.
- [x] Keep all protocol state allocation-free and explicitly bounded.

### Client boundary

- [x] Expose asynchronous bind, send, receive, echo, cancel, close, and status operations.
- [x] Require exact, separate bind/send/receive capabilities.
- [x] Enforce owner, generation, deadline, page, length, and backpressure invariants.

## Exit proof

Network v1 is complete only when permanent automated tests prove all of the following in QEMU:

- LogOS acquires the exact DHCP configuration from the hermetic host peer.
- Host-to-guest and guest-to-host ICMP echo succeed.
- UDP payloads and endpoint metadata survive a round trip byte-for-byte.
- Unauthorized bind, send, and receive operations are denied before service or NIC effects.
- Queue exhaustion returns bounded backpressure and cancellation permits later progress.
- Packet loss and expired deadlines produce bounded retries or structured failures.
- Device reset and Network service restart invalidate stale state, reclaim resources, reacquire
  configuration, and permit a later UDP exchange without reboot.
- Malformed traffic cannot panic the kernel/service, mutate protocol state, or elicit a response.
- Local Terminal, Store, and recovery remain usable while Network is absent, offline, or restarting.

## Crate boundary

- `logos-abi`: bounded Network wire contracts, endpoint handles, scopes, statuses, and limits.
- `logos-net`: `no_std`, allocation-free packet codecs and protocol state.
- `logos-network-service`: independently restartable Ring-2 Network payload.
- `logos-test`: independent host peer, deterministic faults, structured assertions, and artifacts.
- Existing kernel driver code: VirtIO network DMA, interrupts, frame pages, timeout, and reset until
  Ring-1 driver isolation is enforceable.

Do not split Ethernet, ARP, DHCP, IPv4, ICMP, UDP, checksums, endpoints, or timers into separate
crates unless a real dependency boundary requires it.

## Deferred

- TCP, DNS, TLS, trust stores, certificate validation, and secure enrollment.
- IPv6, IP fragmentation/reassembly, IPv4 multicast, VLANs, jumbo frames, raw sockets, and packet
  capture APIs.
- VirtIO modern transport, checksum/segmentation offloads, mergeable RX buffers, control queues,
  multiqueue, MSI/MSI-X, and multiple NICs.
- TAP/bridge integration, physical-NIC hardware coverage, hot-plug, and IOMMU-backed protection from
  a malicious device.
- Wildcard, CIDR, interface, rate, and byte-quota capability scopes; a separately replaceable
  firewall service.
- Automatic ephemeral ports, port reuse, connected UDP, broadcast/multicast datagrams, and more
  than one pending client operation.
- DHCP persistence, static configuration, link-local fallback, DHCPv6, and DNS option consumption.
- Public network reachability and internet-dependent tests.

See [Architecture](architecture.md#12-networking-model),
[Security](security.md), and the [Network boundary ADR](adr/0015-network-v1-boundary.md).

## Later versions

### V2 — Stream connectivity

- Capability-scoped TCP connect, listen, accept, close, and bounded stream I/O.
- Concurrent operations, ephemeral ports, connected endpoint state, and DNS resolution.
- Authenticated-transport integration; identity, trust policy, and key ownership stay in their System services.
- Public-network and longer-duration recovery proofs.

Remote Foundation may consume the smallest V2 slice first: TCP plus bounded streams to an enrolled address.

### V3 — Production networking

- IPv6, multiple NICs, routing policy, firewall service, and broader capability scopes and quotas.
- Modern VirtIO, MSI-X, multiqueue, justified offloads, physical hardware, hotplug, and IOMMU-backed isolation.

## Protocol references

- [VirtIO 1.3 network device](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html#x1-2420001)
- [QEMU network backends](https://www.qemu.org/docs/master/system/invocation.html)
- [ARP — RFC 826](https://www.rfc-editor.org/rfc/rfc826)
- [IPv4 — RFC 791](https://www.rfc-editor.org/rfc/rfc791)
- [ICMP — RFC 792](https://www.rfc-editor.org/rfc/rfc792)
- [UDP — RFC 768](https://www.rfc-editor.org/rfc/rfc768)
- [DHCP — RFC 2131](https://www.rfc-editor.org/rfc/rfc2131)
- [DHCP options — RFC 2132](https://www.rfc-editor.org/rfc/rfc2132)
