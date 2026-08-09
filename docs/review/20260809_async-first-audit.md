# Async-first/state-transition audit

- Date: 2026-08-09
- Starting SHA: `b8bb796af79c9e9ab39a2dc27219c7a9b26b2068`
- Scope: production runtime, scheduler, native services, typed ABI pages, and current architecture notes.

The audit applies the categories from ADR-0028. A `waiting_*` field is compliant when it owns the
operation and the service can return to its entry point; it is not evidence of a problem by itself.

| subsystem | path | current model | category | risk | recommended action |
| --- | --- | --- | --- | --- | --- |
| Network | `src/platform/network.rs::poll` and `take_wake` | Device, readiness, stream, timeout, reset, and generation state are owned by `NetworkRuntime`; bounded wake targets are drained by `platform::runtime`. | A | Low | Preserve; use as the composition pattern. |
| Network service | `crates/logos-network-service/src/main.rs` `waiting_receive`, `waiting_accept`, `waiting_send`, `waiting_echo` | Fixed pending request slots resume from device/event state. | B | Medium | Keep for bootstrap; migrate to independent operation slots after the current Network proof cycle. |
| Network client | `crates/logos-service-rt/src/lib.rs` wait/finish helpers | Blocking-looking helpers wrap typed page state and authoritative completion/readiness. | A | Low | No change. |
| Scheduler | `src/platform/runtime.rs::drain_network_wakes` | Top-level composition drains bounded notifications, validates generation through scheduler handles, then runs tasks. | A | Low | Keep scheduler ownership here. |
| Remote startup | `src/platform/remote.rs::RemoteRuntime::start` | Remote state previously called `scheduler.run(gateway)` while starting transport. | C | High | Fixed in this change: Remote reports the gateway handle; runtime starts it. |
| Sessions | `src/platform/session.rs::SessionsRuntime::relay` and `invoke_native` | One logical request is completed through deliver -> effect -> reply, with direct wake/run between phases. | C/B | High | Retain as the current single-request bootstrap wrapper; convert to explicit phase state and a bounded runnable notification. |
| Storage relay | `src/platform/storage.rs::relay_store_request` | Store request, Block dispatch, durable reply, and page-loan return are completed in a loop that runs Storage directly. | C/D | High | Preserve durability semantics; extract bounded relay phases before adding concurrent Store requests. |
| Storage startup | `src/platform/storage.rs::run_startup` | Bootstrap waits for service status while advancing bounded Block state. | B/D | Medium | Keep until service startup is independently state-driven. |
| Storage service | `crates/logos-storage-service/src/main.rs::BlockBackend` | Block client convenience calls are synchronous at the sector/durability boundary; Store replies only after commit/flush semantics. | D | Medium | Keep semantic boundary; later split internal block operations if concurrency requires it. |
| Remote request | `src/platform/runtime.rs::handle_remote_request` | Remote control is persisted before and after session invocation, but the path nests Storage and Sessions completion. | C/D | High | Define `Received -> Persisting -> Invoking -> Completing -> ReadyToReply`; defer extraction. |
| Gateway | `crates/logos-gateway-service/src/main.rs` | Bootstrap service loop waits on typed Network/Remote pages and advances one connection at a time. | B | Medium | Do not expand; replace with owned Gateway operation state after real TCP proof. |
| Sessions service | `crates/logos-sessions-service/src/main.rs` | Waits on SessionPage, requests one privileged Effect, then publishes terminal reply. | A/B | Low | The service owns the request lifecycle; platform relay choreography remains the debt. |
| Polling | `src/platform/runtime.rs` network/storage/gateway polls | Bounded composition polls inspect state and return; no generic reactor or event bus exists. | A | Low | Keep; reject loops that wait without a state transition. |
| ABI pages | `crates/logos-abi/src/service.rs` | Lifecycle, generation, readiness/completion, and bounded notification fields are authoritative state. | A | Low | Add version/sequence guidance; no generic Signals primitive. |

## Conclusions

The current Network stream architecture is compliant and remains the reference model. The high-risk
remaining work is concentrated in the bootstrap composition wrappers for Sessions, Storage, and
Remote persistence. This task removes the direct Remote startup coupling only; the larger conversions
are intentionally scoped as bounded follow-up work because they change request ownership and durable
reply timing. No async runtime, future type, executor, universal event bus, or ABI v5 is warranted.
