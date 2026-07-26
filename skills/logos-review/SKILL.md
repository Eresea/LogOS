---
name: logos-review
description: Review LogOS OS project — architecture, code quality, capability model, ring boundaries
category: software-development
---

# LogOS Project Review Skill

Use this skill when reviewing the LogOS operating system project. It provides structured checklists for architecture compliance, code quality, capability model adherence, and ring boundary enforcement.

## When to Use

- Reviewing PRs or changes to LogOS
- Auditing architecture compliance
- Verifying capability model correctness
- Checking ring boundary violations
- Pre-release quality gates

---

## 1. Architecture Review Checklist

### Ring Boundary Enforcement

Run the dependency visualization script to verify:

```bash
python3 scripts/arch-deps.py | dot -Tsvg > arch.svg
```

**Verify:**
- [ ] **Core (Ring 0)** imports only: `uefi`, `core`, `alloc` — never Foundation/System/Sessions/Runtime/Experience
- [ ] **Foundation (Ring 1)** imports only Core contracts — never System+
- [ ] **System (Ring 2)** imports only Core + Foundation contracts — never Sessions+
- [ ] **Sessions (Ring 3)** imports only Core + Foundation + System contracts — never Runtime+
- [ ] **No inward dependencies** — outer rings never called by inner rings

### Capability Model Compliance

For each new/modified capability use:
- [ ] Capability granted via `CapabilityManager::grant()` with explicit `CapabilityKind`
- [ ] Every `allows()` check matches a granted capability kind
- [ ] Revocation tested: `revoke()` → subsequent `allows()` returns `false`
- [ ] Generation overflow handled: `generation.wrapping_add(1)` on revoke
- [ ] No ambient authority — every privileged operation requires capability argument

### Service Manifest Compliance (Platform v1+)

For each service:
- [ ] `service-manifests/<name>.toml` exists with all required fields
- [ ] Capabilities declared match actual usage in code
- [ ] Dependencies form acyclic graph (topological sort succeeds)
- [ ] Resource quotas (memory, IPC) enforced by Supervisor
- [ ] Health check endpoint implemented and registered

---

## 2. Code Quality Review

### Bootstrap Pattern Detection

**Flag any of these patterns (should use dynamic allocator):**
```rust
// ❌ Fixed arrays — replace with CapabilityVec
const FOO: usize = 8;
slots: [Option<T>; FOO]

// ❌ Inline asm outside hal.rs
asm!("out dx, al", ...)

// ❌ Option/unwrap()/unwrap() in kernel paths
.capability.unwrap()
result.expect("msg")

// ❌ Direct port I/O outside hal.rs
inb(0x60)
outb(0x64, 0x20)
```

**Preferred patterns:**
```rust
// ✅ Dynamic capability-backed allocation
let mut vec = CapabilityVec::new_in(allocator);
vec.push(item);

// ✅ HAL abstraction
hal.outb(port, value);

// ✅ Structured errors with context
Err(KernelError::CapabilityDenied { required, held })

// ✅ Explicit capability checks
if !caps.allows(cap, CapabilityKind::Service) { return Err(...); }
```

### Error Handling Standards

- [ ] All fallible operations return `Result<T, KernelError>`
- [ ] `KernelError` variants include contextual fields (addresses, capabilities, sizes)
- [ ] No `panic!` in production paths — only `debug_assert!` or `unreachable!` with comments
- [ ] Recovery console commands return structured `Result`, not print-only

### Memory Safety

- [ ] No `unsafe` without `// SAFETY:` comment explaining invariants
- [ ] All `unsafe` blocks minimized — prefer safe abstractions
- [ ] `PhysicalMemory::release_page()` validates page belongs to managed ranges
- [ ] `VirtualMemory::Mapping::release()` restores CR3 before releasing page tables

---

## 3. Capability Model Review

### Capability Kinds Audit

Check `src/capabilities.rs` for completeness:

| CapabilityKind | Used By | Granted To | Revocation Tested |
|----------------|---------|------------|-------------------|
| `Debug` | `debug::write*` | Kernel self-check | ✅ |
| `Service` | IPC send/reply, service register | VirtIO, Console | ✅ |
| `Recovery` | `commands::invoke("recovery")` | Normal console | ✅ |
| *(add new kinds)* | | | ❌ |

**For each new capability:**
- [ ] Added to `CapabilityKind` enum
- [ ] Grant path exists in appropriate service setup
- [ ] Revocation tested in `self_check()` or unit test
- [ ] Documented in `docs/capability-delegation.md`

### Delegation Rules

- [ ] Delegated capabilities use narrowed `CapabilityKind` (or future `Capability<T>`)
- [ ] Delegation carries explicit scope/expiry where applicable
- [ ] Revocation of parent revokes all delegated (generation-based)
- [ ] Audit log entry on grant/delegate/revoke

---

## 4. Ring-Specific Review Guides

### Ring 0 — Core (`src/main.rs`, `src/scheduler.rs`, `src/memory.rs`, `src/capabilities.rs`, `src/interrupts.rs`, `src/ipc.rs`, `src/virtual_memory.rs`, `src/acpi.rs`, `src/pci.rs`)

**Must not contain:**
- [ ] Filesystem, network, or display logic
- [ ] Keyboard layout handling
- [ ] Font rasterization
- [ ] Shell/command parsing
- [ ] WASM runtime references
- [ ] User identity/authentication

**Must enforce:**
- [ ] Physical page ownership invariants
- [ ] Virtual mapping permissions
- [ ] Capability validation on every IPC send/reply
- [ ] Scheduler generation tags on task handles
- [ ] Interrupt-safe IPC production

### Ring 1 — Foundation (`src/display.rs`, `src/input.rs`, `src/text.rs`, `src/virtio.rs`, `src/keyboard.rs`)

**Must not contain:**
- [ ] Service restart policy
- [ ] User identities
- [ ] Persistent configuration
- [ ] Terminal editing/history
- [ ] Command execution

**Must provide:**
- [ ] Device-independent protocols (Display, Input, Block, NetDevice)
- [ ] Explicit capability requirements for hardware access
- [ ] Driver quiesce/reset/recovery paths

### Ring 2 — System (Future: `src/supervisor.rs`, `src/services.rs`)

**Must not contain:**
- [ ] Terminal rendering
- [ ] Desktop/compositor logic
- [ ] WASM runtime

**Must provide:**
- [ ] Service supervision (manifest-driven)
- [ ] Identity, secrets, time, storage, network services
- [ ] Audit events for privileged operations

### Ring 3 — Sessions (Future: `src/terminal.rs`, `src/commands.rs`, `src/mode.rs`, `src/console.rs`)

**Must not contain:**
- [ ] Direct hardware access
- [ ] Kernel memory management
- [ ] Driver binding

**Must provide:**
- [ ] Session identity + capability context
- [ ] Command registry with typed schemas
- [ ] Terminal model (Slate) — editor, scrollback, selection
- [ ] Remote gateway transport

---

## 5. Testing Review Gates

### Host Tests (Run in CI on every PR)

```bash
cargo test --features test,mock_hal --lib
```

**Must pass:**
- [ ] Scheduler: spawn, block, wake, generation overflow
- [ ] Capabilities: grant, allows, revoke, generation wrap
- [ ] IPC: send/receive FIFO, capacity bound, capability check
- [ ] Terminal: UTF-8, grapheme clusters, selection, scrollback, history
- [ ] Memory: allocate/release, owned pages, range validation
- [ ] VirtIO: queue allocation, submit/complete, recover

### QEMU Integration Tests (Run in CI on merge to main)

```bash
./scripts/verify.ps1
```

**Must demonstrate:**
- [ ] UEFI boot → self-check passed
- [ ] Recovery console: help, trace, ping, inflate, recover, version, exit
- [ ] Normal terminal: input, editing, UTF-8, blink, render
- [ ] VirtIO driver: bind, ping/pong, inflate, recover, cancel
- [ ] Recovery handoff: `recovery` command switches to recovery console
- [ ] ACPI power-off/reset functional

---

## 6. Documentation Review

### Required Updates per Change Type

| Change | Docs to Update |
|--------|----------------|
| New capability kind | `docs/capability-delegation.md`, `src/capabilities.rs` |
| New service | `service-manifests/<name>.toml`, `docs/architecture.md` |
| Ring boundary change | `docs/ARCHITECTURE.md`, `docs/ONION_RINGS.md`, ADR |
| New error variant | `docs/error-catalog.md` (create if needed) |
| IPC protocol change | `docs/ipc-protocol.md` + version bump |
| Boot sequence change | `docs/boot-sequence.md` |

### ADR Process

For architectural decisions:
- [ ] Create `docs/adr/XXXX-title.md` using template
- [ ] Link from related code/docs
- [ ] Review in PR — decisions are reviewable artifacts

---

## 7. Review Commands Quick Reference

```bash
# Architecture visualization
python3 scripts/arch-deps.py | dot -Tsvg > arch.svg

# Host unit tests (fast)
cargo test --features test,mock_hal --lib

# Full check (fmt, clippy, host tests)
cargo fmt --check && cargo clippy -- -D warnings && cargo test --features test,mock_hal --lib

# QEMU verification (slow, needs OVMF)
export OVMF_CODE=/path/to/OVMF_CODE.fd
./scripts/verify.ps1

# Check for bootstrap patterns (manual grep)
rg 'const \w+: usize = \d+' src/
rg 'asm!' src/ --glob '!hal.rs'
rg '\.unwrap\(\)|\.expect\(' src/
rg 'inb\(|outb\(' src/ --glob '!hal.rs'

# Capability usage audit
rg 'CapabilityKind::' src/
rg 'capabilities\.allows' src/
rg 'capabilities\.grant' src/
rg 'capabilities\.revoke' src/
```

---

## 8. Common Review Findings & Fixes

| Finding | Fix |
|---------|-----|
| Fixed array in new module | Use `CapabilityVec::new_in(allocator)` |
| Inline asm in driver | Move to `hal.rs` implementation |
| `unwrap()` in kernel path | Return `Result<T, KernelError>` |
| Missing capability check | Add `capabilities.allows(cap, Kind)` guard |
| Inner ring imports outer | Invert dependency — use callback/trait |
| Service hardcoded in main.rs | Add manifest + register via Supervisor |
| No revocation test | Add to module `self_check()` or unit test |
| UTF-8 handling in terminal | Use `unicode-segmentation` + `unicode-width` |

---

## 9. Skill Maintenance

- Update this skill when: new rings added, capability model changes, review process evolves
- Version: track in `docs/adr/` when review criteria change
- Test: run review checklist on known-good and known-bad PRs