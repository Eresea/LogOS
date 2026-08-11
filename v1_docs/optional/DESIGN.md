# LogOS Design Document: Architecture Improvements & Development Workflow

> **Status:** Living design document  
> **Created:** 2026-07-25  
> **Target:** Post-Core v1, pre-Console v1 completion

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Critical Infrastructure Improvements](#2-critical-infrastructure-improvements)
3. [High-Priority Architecture Changes](#3-high-priority-architecture-changes)
4. [Code Quality & Maintainability](#4-code-quality--maintainability)
5. [Development Workflow Enhancements](#5-development-workflow-enhancements)
6. [Testing Strategy](#6-testing-strategy)
7. [Documentation Gaps](#7-documentation-gaps)
8. [Implementation Priority Matrix](#8-implementation-priority-matrix)

---

## 1. Executive Summary

LogOS Core v1 demonstrates a working capability-based kernel with UEFI boot, cooperative scheduling, IPC, VirtIO driver recovery, and dual console paths. The architecture (6-ring onion model) is well-documented and principled.

**However**, the codebase relies heavily on **fixed-size arrays** (`// ponytail:` comments), **ad-hoc error handling** (`Option<T>` everywhere), **inline assembly scattered** across modules, and **hardcoded service registration**. These are intentional bootstrap decisions that now block outward progress toward Platform v1.

This document proposes **concrete, incremental improvements** that:
- Replace bootstrap patterns with proper abstractions
- Enable the Supervisor (Ring 2) to manage services declaratively
- Make the codebase testable, auditable, and extensible
- Improve developer iteration speed

---

## 2. Critical Infrastructure Improvements

### 2.1 Dynamic Capability-Backed Allocator

**Current State:**  
Every subsystem uses fixed arrays: `TASKS=8`, `CAPABILITIES=5`, `SERVICES=4`, `MESSAGES=4`, `QUEUE_PAGES=8`, `DEVICES=8`, `RANGES=8`, `ROUTES=64`, `EVENTS=64`, `SCANCODES=16`, `CELLS=64`, `SCROLLBACK=8`.

**Proposed Design:**
```rust
// src/alloc.rs (new)
pub trait CapabilityAllocator {
    fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>>;
    fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout);
    fn quota_remaining(&self) -> usize;
}

pub struct CapabilityVec<T, A: CapabilityAllocator> {
    ptr: NonNull<T>,
    len: usize,
    cap: usize,
    allocator: A,
    _phantom: PhantomData<T>,
}
```

**Why:**  
- **Enforces capability model at memory level** — services can't exceed granted quotas
- **Enables dynamic service scaling** — no more "retain eight; add dynamic storage when..."
- **Foundation for WASM runtime quotas** — same allocator pattern applies to Ring 4
- **Removes 15+ `ponytail` TODOs** with one coherent abstraction

**Migration Path:**  
1. Implement `CapabilityAllocator` for `PhysicalMemory` (Ring 0)
2. Create `CapabilityVec` / `CapabilityHashMap` in `alloc.rs`
3. Replace `scheduler::TASKS`, `capabilities::CAPABILITIES`, `ipc::MESSAGES` first
4. Incrementally migrate remaining fixed arrays

---

### 2.2 Service Manifest Format & Supervisor Skeleton

**Current State:**  
Services registered imperatively in `main.rs:176-182` with hardcoded capability grants and no dependency declaration.

**Proposed Design:**
```toml
# service-manifests/virtio-balloon.toml
[service]
name = "virtio-balloon"
version = "0.1.0"
ring = 1  # Foundation

[capabilities]
required = ["service", "virtio:pci:0x1af4:0x1002"]
grants = ["block:inflate", "block:deflate"]

[dependencies]
required = ["pci", "interrupts", "memory"]
optional = ["acpi-pci-routing"]

[lifecycle]
restart_policy = "on_failure"
max_restarts = 3
backoff_ms = 1000
health_check = "ipc_ping"
health_interval_ms = 5000

[resources]
max_memory_pages = 16
max_ipc_messages = 32
```

```rust
// src/supervisor.rs (new)
pub struct ServiceManifest {
    pub name: String,
    pub version: Version,
    pub ring: Ring,
    pub capabilities: CapabilitySpec,
    pub dependencies: DependencyGraph,
    pub lifecycle: LifecyclePolicy,
    pub resources: ResourceQuotas,
}

pub struct Supervisor {
    manifests: HashMap<ServiceId, ServiceManifest>,
    running: HashMap<ServiceId, ServiceInstance>,
    capability_mgr: CapabilityManager,
    scheduler: Scheduler,
}
```

**Why:**  
- **Declarative > imperative** — enables tooling (validation, visualization, dry-run)
- **Boot profiles** (normal/recovery/diagnostic) become data, not code paths
- **Service replacement without kernel rebuild** — core Platform v1 requirement
- **Audit trail** — every capability grant/denial traceable to manifest

**Migration Path:**  
1. Define TOML schema + `serde` parsing (enable `std` feature for host tools)
2. Build `Supervisor` with dependency resolution (topological sort)
3. Port `VirtioBalloon` registration to manifest-driven
4. Add `boot-profile.toml` selecting active manifests

---

### 2.3 Hardware Abstraction Layer (HAL)

**Current State:**  
Inline `asm!` and port I/O in `interrupts.rs`, `keyboard.rs`, `pci.rs`, `virtio.rs`, `acpi.rs`, `virtual_memory.rs`. No mockability.

**Proposed Design:**
```rust
// src/hal.rs (new)
pub trait Hal: Send + Sync {
    // Port I/O
    fn inb(&self, port: u16) -> u8;
    fn outb(&self, port: u16, value: u8);
    fn inw(&self, port: u16) -> u16;
    fn outw(&self, port: u16, value: u16);
    fn inl(&self, port: u16) -> u32;
    fn outl(&self, port: u16, value: u32);

    // CPU control
    fn read_cr3(&self) -> u64;
    fn write_cr3(&self, value: u64);
    fn hlt(&self);
    fn cli(&self);
    fn sti(&self);
    fn pause(&self);

    // Memory barriers
    fn compiler_fence(&self, ordering: Ordering);
    fn memory_fence(&self, ordering: Ordering);

    // Timer
    fn read_tsc(&self) -> u64;
}

// Concrete implementations
pub struct X86Hal;           // Real hardware (current inline asm)
pub struct UefiHal;          // UEFI runtime services
pub struct MockHal;          // Host-side testing with recorded I/O
```

**Why:**  
- **Unit test kernel logic on host** — no QEMU needed for scheduler, IPC, capability tests
- **Architecture portability** — ARM64/RISC-V HAL implementations swap in
- **Deterministic replay** — `MockHal` records I/O for failure reproduction
- **UEFI runtime services** — clean separation from boot services exit

**Migration Path:**  
1. Extract all `asm!`/`inb`/`outb` into `X86Hal` implementation
2. Update `interrupts.rs`, `keyboard.rs`, `pci.rs`, `virtio.rs` to take `&dyn Hal`
3. Add `#[cfg(test)]` module using `MockHal` for scheduler/IPC/capability tests
4. Gate `MockHal` behind `test` feature flag

---

## 3. High-Priority Architecture Changes

### 3.1 Structured Error Types with Context

**Current State:**  
Functions return `Option<T>` or `bool` — callers can't distinguish "device not found" from "out of memory" from "capability denied."

**Proposed Design:**
```rust
// src/error.rs (new)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    // Memory
    OutOfMemory { requested: usize, available: usize },
    InvalidAddress { address: u64, reason: &'static str },
    MappingFailed { permission: Permission, page: u64 },

    // Capabilities
    CapabilityDenied { required: CapabilityKind, held: CapabilityKind },
    CapabilityExpired { generation: u16, current: u16 },
    CapabilityTableFull { capacity: usize },

    // Scheduler
    TaskTableFull { capacity: usize },
    TaskNotFound { handle: TaskHandle },
    TaskGenerationMismatch { expected: u16, actual: u16 },

    // IPC
    ChannelFull { capacity: usize },
    ChannelEmpty,
    InvalidDestination { handle: ServiceHandle },
    CapabilityMismatch { required: CapabilityKind },

    // VirtIO
    InvalidBar { bar: u8, value: u32 },
    QueueAllocationFailed { pages_requested: usize, pages_available: usize },
    DeviceActivationFailed { status_port: u16, status: u8 },
    InterruptRoutingFailed { gsi: u32, reason: &'static str },
    DriverStateInvalid { expected: DriverState, actual: DriverState },

    // ACPI
    RsdpNotFound,
    RsdpChecksumFailed { offset: usize },
    XsdtNotFound,
    MadtNotFound,
    PciRoutingTableNotFound,
    AmlParseError { offset: usize, expected: &'static str },
}
```

**Why:**  
- **Actionable diagnostics** — recovery console can suggest specific remediation
- **Structured audit logs** — `system.audit` (Ring 2) gets machine-parseable errors
- **Capability-aware errors** — distinguishes "you can't" from "it's broken"
- **Enables automated recovery** — Supervisor matches error → policy

---

### 3.2 Async/Await IPC + Lightweight Executor

**Current State:**  
Synchronous `Channel::send`/`receive` with spinlocks. Tasks block via `TaskState::Blocked(Event)` polled by scheduler.

**Proposed Design:**
```rust
// src/executor.rs (new)
pub struct Executor {
    scheduler: Scheduler,
    waker_registry: WakerRegistry,
}

pub struct WakerRegistry {
    // Maps Event -> Vec<Waker>
    waiters: HashMap<Event, Vec<Waker>, CapabilityAllocator>,
}

impl Executor {
    pub fn spawn<F: Future<Output = ()> + 'static>(&mut self, future: F) -> TaskHandle;
    pub fn run_until_stalled(&mut self);
    pub fn wake_event(&mut self, event: Event);
}

// Async IPC
impl Channel {
    pub async fn send_async(&self, caps: &CapabilityManager, cap: Capability, 
                           dest: ServiceHandle, msg: Message) -> Result<RequestId, IpcError>;
    pub async fn receive_async(&self) -> Option<Envelope>;
}
```

**Why:**  
- **Composable async workflows** — `select!`, `join!`, timeout, cancellation
- **Natural backpressure** — `await` yields instead of spinning
- **Foundation for WASM async host imports** — same executor pattern
- **Structured cancellation** — `Drop` of future = automatic cancel

**Migration Path:**  
1. Add `core::future` + `core::task` types (no std needed)
2. Implement `Waker` backed by `scheduler::Event` + generation-tagged handles
3. Add `async` variants alongside sync `send`/`receive`
4. Convert `VirtioService::ServiceTask` to async

---

### 3.3 UTF-8 Grapheme Cluster Support in Terminal

**Current State:**  
`terminal.rs` handles UTF-8 byte sequences but treats each byte as a cell. No grapheme cluster awareness (emoji, combining marks, CJK).

**Proposed Design:**
```rust
// Add to Cargo.toml
unicode-segmentation = "1.10"
unicode-width = "0.1"

// src/terminal.rs
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

impl Model {
    pub fn insert_grapheme(&mut self, grapheme: &str) -> bool {
        let width = grapheme.width();
        if self.column + width > self.columns(display) { return false; }
        // Insert at grapheme boundary
        self.graphemes.insert(self.grapheme_cursor, grapheme.to_string());
        self.grapheme_cursor += 1;
        true
    }

    pub fn move_left_grapheme(&mut self) -> bool { /* ... */ }
    pub fn delete_grapheme(&mut self) -> bool { /* ... */ }
}
```

**Why:**  
- **Correct international text** — user-facing terminal must handle real languages
- **Selection/copy works** — grapheme boundaries = UTF-8 boundaries for clipboard
- **Prevents rendering corruption** — half-emoji, split combining marks
- **Foundation for `session.terminal` (Slate) contract** — spec requires UTF-8 boundary validation

---

## 4. Code Quality & Maintainability

### 4.1 Consolidate Self-Checks into Test Harness

**Current State:**  
Each module has `pub fn self_check() -> bool` called from `main.rs`. No isolation, no parallel execution, no reporting.

**Proposed Design:**
```rust
// src/testing.rs (new)
pub trait SelfTest {
    fn name(&self) -> &'static str;
    fn run(&mut self) -> TestResult;
}

pub struct TestResult {
    pub passed: bool,
    pub duration_cycles: u64,
    pub details: Option<&'static str>,
}

pub fn run_all_tests(hal: &dyn Hal) -> Vec<TestResult> {
    let mut tests: Vec<Box<dyn SelfTest>> = vec![
        Box::new(MemoryTest::new(hal)),
        Box::new(SchedulerTest::new(hal)),
        Box::new(CapabilityTest::new(hal)),
        Box::new(IpcTest::new(hal)),
        Box::new(VirtioTest::new(hal)),
        Box::new(TerminalTest::new(hal)),
    ];
    tests.iter_mut().map(|t| t.run()).collect()
}
```

**Why:**  
- **Parallel test execution** — independent tests run concurrently
- **Structured output** — machine-parseable for CI
- **Host-side execution** — with `MockHal`, tests run in `cargo test` without QEMU
- **Regression detection** — timing + pass/fail tracked over time

---

### 4.2 Add Clippy Lints & Custom Lint Rules

**Current State:**  
`cargo clippy -- -D warnings` in `scripts/check.ps1`. No project-specific lints.

**Proposed Custom Lints** (via `clippy_driver` or `dylint`):
- `logos_fixed_array` — flag `[T; N]` where `N > 4` without capability allocator
- `logos_inline_asm` — flag `asm!` outside `hal.rs`
- `logos_option_unwrap` — flag `.unwrap()`/`.expect()` in kernel paths
- `logos_capability_check` — flag capability use without explicit grant audit

**Why:**  
- **Enforces architectural rules automatically** — no manual review needed
- **Catches bootstrap patterns early** — prevents new `ponytail` comments
- **Documents intent in code** — lints are executable documentation

---

### 4.3 Structured Logging for Trace Export

**Current State:**  
`trace.rs` uses ring buffer of `Event` enum. `console::Shell::trace` command prints raw messages.

**Proposed Design:**
```rust
// src/trace.rs enhancement
pub struct TraceEvent {
    pub timestamp: u64,        // TSC cycles
    pub cpu_id: u8,            // For SMP future
    pub event: Event,
    pub task: Option<TaskHandle>,
    pub service: Option<ServiceHandle>,
    pub fields: [(&'static str, u64); 4],  // Fixed small array
    pub field_count: u8,
}

impl TraceEvent {
    pub fn new(event: Event) -> Self { ... }
    pub fn with_task(mut self, task: TaskHandle) -> Self { ... }
    pub fn with_field(mut self, key: &'static str, value: u64) -> Self { ... }
}

// Export formats
pub fn export_json(&self, writer: &mut dyn Write) -> Result<(), Error>;
pub fn export_cbor(&self, writer: &mut dyn Write) -> Result<(), Error>;
pub fn export_protobuf(&self, writer: &mut dyn Write) -> Result<(), Error>;
```

**Why:**  
- **Machine-readable traces** — external analyzers, AI tooling, CI dashboards
- **Correlation IDs** — link IPC request→reply→driver interrupt→completion
- **Minimal overhead** — fixed-size fields, no allocation in hot path
- **Remote debugging** — `system.remote` (Gateway) streams structured traces

---

## 5. Development Workflow Enhancements

### 5.1 Host-Side Unit Testing (No QEMU Required)

**Current Workflow:**  
Every change → `.\scripts\run.ps1` → QEMU boots → 15s wait → manual verification.

**Proposed Workflow:**
```bash
# Fast path (host)
cargo test --features test,mock_hal --lib

# Integration path (QEMU)
cargo test --features integration --test qemu_integration
```

**Implementation:**
```toml
# Cargo.toml
[features]
default = []
test = ["std", "mock_hal"]
mock_hal = ["alloc", "std"]
integration = ["std", "qemu_test_harness"]

[dev-dependencies]
qemu-test-harness = { path = "../qemu-test-harness" }  # New crate
```

**qemu-test-harness crate** provides:
- QEMU process management (spawn, wait, kill)
- Serial port capture
- Automated health-check parsing
- Screenshot/framebuffer comparison

**Why:**  
- **Sub-second feedback** for 90% of changes (scheduler, IPC, capabilities, terminal model)
- **CI/CD friendly** — GitHub Actions runs `cargo test` in 30s, QEMU tests in 2min
- **Bisect-friendly** — `cargo bisect` works on host tests

---

### 5.2 Pre-Commit Hooks with Real Checks

**Current State:**  
`scripts/check.ps1` runs formatting, clippy, and QEMU verify. Manual invocation.

**Proposed `.pre-commit-config.yaml`:**
```yaml
repos:
  - repo: local
    hooks:
      - id: cargo-fmt
        name: cargo fmt --check
        entry: cargo fmt --check
        language: system
        types: [rust]
      
      - id: cargo-clippy
        name: cargo clippy -- -D warnings
        entry: cargo clippy -- -D warnings
        language: system
        types: [rust]
        pass_filenames: false
      
      - id: cargo-test-host
        name: cargo test (host)
        entry: cargo test --features test,mock_hal --lib
        language: system
        types: [rust]
        pass_filenames: false
        stages: [pre-commit, pre-push]
      
      - id: docs-spellcheck
        name: markdown spellcheck
        entry: cspell docs/**/*.md
        language: system
        types: [markdown]
```

**Why:**  
- **Catches issues before push** — no CI failures on trivial formatting
- **Runs host tests automatically** — regression caught at commit time
- **Documentation quality** — spellcheck prevents typo accumulation

---

### 5.3 Automated QEMU Verification in CI

**Current State:**  
`verify.ps1` runs headless QEMU with 15s timeout. Manual/local only.

**Proposed GitHub Actions Workflow:**
```yaml
# .github/workflows/verify.yml
name: Verify
on: [push, pull_request]

jobs:
  host-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --features test,mock_hal --lib
  
  qemu-verify:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install QEMU + OVMF
        run: |
          sudo apt-get update && sudo apt-get install -y qemu-system-x86_64 ovmf
      - name: Run verification
        run: |
          export OVMF_CODE=/usr/share/OVMF/OVMF_CODE.fd
          cargo build --release --target x86_64-unknown-uefi
          ./scripts/verify.ps1  # or native test harness
      - name: Upload artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: qemu-serial-log
          path: target/qemu-verify.log
```

**Why:**  
- **Every PR verified** — no "works on my machine" regressions
- **Artifact retention** — failed boot logs downloadable for debugging
- **Fast feedback** — host tests in ~30s, QEMU in ~2min

---

### 5.4 Architecture Decision Records (ADRs)

**Current State:**  
Architectural decisions in `ARCHITECTURE.md`, `ONION_RINGS.md`, `NAMING.md` — mixed with reference docs.

**Proposed Structure:**
```
docs/adr/
├── 0001-ring-model.md
├── 0002-capability-model.md
├── 0003-hal-abstraction.md
├── 0004-service-manifest-format.md
├── 0005-async-ipc-design.md
├── 0006-wasm-runtime-placement.md
└── template.md
```

**Template:**
```markdown
# ADR 000X: <Title>

## Status
Proposed | Accepted | Superseded | Deprecated

## Context
What is the issue that motivates this decision?

## Decision
What is the change we're making?

## Consequences
### Positive
- ...

### Negative
- ...

### Neutral
- ...

## Alternatives Considered
- Alternative A: ...
- Alternative B: ...

## References
- Related ADRs, issues, PRs
```

**Why:**  
- **Decisions are discoverable** — not buried in long markdown files
- **Historical context preserved** — why we chose X over Y
- **Reviewable** — new contributors understand *why* not just *what*
- **Supersession tracking** — explicit when decisions change

---

### 5.5 Dependency Visualization Script

**Proposed `scripts/arch-deps.py`:**
```python
#!/usr/bin/env python3
"""Generate GraphViz DOT of crate internal dependencies."""
import re
from pathlib import Path

def extract_mods(rs_file):
    content = Path(rs_file).read_text()
    return re.findall(r'^mod (\w+);', content, re.MULTILINE)

def extract_uses(rs_file):
    content = Path(rs_file).read_text()
    # crate::module::item
    return re.findall(r'use crate::(\w+)::', content)

def main():
    src = Path("src")
    modules = {}
    for rs in src.glob("*.rs"):
        if rs.name == "main.rs": continue
        mod_name = rs.stem
        modules[mod_name] = {
            "submods": extract_mods(rs),
            "uses": extract_uses(rs),
        }
    
    print("digraph LogOS {")
    print("  rankdir=LR;")
    for mod_name, info in modules.items():
        for sub in info["submods"]:
            print(f'  "{mod_name}" -> "{sub}" [style=dashed];')
        for use in info["uses"]:
            print(f'  "{mod_name}" -> "{use}" [color=blue];')
    print("}")

if __name__ == "__main__":
    main()
```

**Output:** `scripts/arch-deps.py | dot -Tsvg > arch.svg`

**Why:**  
- **Visualizes ring boundaries** — catches inward dependency violations
- **CI gate** — fail build if Core imports Foundation
- **Onboarding** — new contributors see architecture at a glance

---

## 6. Testing Strategy

### 6.1 Test Pyramid for LogOS

```
                    ┌─────────────────┐
                    │  E2E (QEMU)     │  ← 5-10 tests: boot, recovery, 
                    │  Integration    │      driver recovery, IPC round-trip
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
       ┌───────────┐  ┌───────────┐  ┌───────────┐
       │ Component │  │ Component │  │ Component │  ← 20-30 tests: service lifecycle,
       │  Tests    │  │  Tests    │  │  Tests    │      terminal editing, capability grants
       └─────┬─────┘  └─────┬─────┘  └─────┬─────┘
             │              │              │
      ┌──────┴──────┐ ┌────┴────┐ ┌──────┴──────┐
      ▼             ▼ ▼         ▼ ▼             ▼
   ┌───────┐    ┌───────┐ ┌───────┐    ┌───────┐
   │ Unit  │    │ Unit  │ │ Unit  │    │ Unit  │  ← 100+ tests: scheduler, allocator,
   │ Tests │    │ Tests │ │ Tests │    │ Tests │      parser, grapheme, error types
   └───────┘    └───────┘ └───────┘    └───────┘
```

### 6.2 Property-Based Testing Targets

| Module | Properties to Test |
|--------|-------------------|
| `scheduler` | No task runs twice without wake; generation increments on complete; wake respects generation |
| `capabilities` | Revoked capability never passes `allows`; grant/revoke roundtrip; generation overflow |
| `ipc` | Send/receive FIFO; capacity bound; capability checked on send & reply |
| `terminal` | UTF-8 roundtrip; grapheme cluster integrity; selection boundaries; scrollback capacity |
| `memory` | Allocate/release identity; owned pages reusable; range validation |
| `virtio` | Queue allocation contiguous; submit/complete correlation; recover restores state |

---

## 7. Documentation Gaps

| Gap | Location | Priority |
|-----|----------|----------|
| Capability delegation protocol | `docs/security.md` + new `docs/capability-delegation.md` | 🔴 |
| Driver recovery contract | `src/virtio.rs` comments + `docs/driver-recovery.md` | 🔴 |
| Session capability model | `docs/ARCHITECTURE.md` §14 + new `docs/session-capabilities.md` | 🟠 |
| WASM host interface (WIT) | New `wit/` directory + `docs/wasm-host-interface.md` | 🟠 |
| Boot profile specification | New `docs/boot-profiles.md` | 🟠 |
| Update package format | New `docs/update-format.md` | 🟡 |
| Remote protocol specification | New `docs/remote-protocol.md` | 🟡 |

---

## 8. Implementation Priority Matrix

| Task | Effort | Impact | Blocks | Suggested Order |
|------|--------|--------|--------|-----------------|
| **Capability allocator** | M | 🔴 Critical | All dynamic services | 1 |
| **HAL abstraction** | M | 🔴 Critical | Host testing, arch ports | 2 |
| **Host test harness** | S | 🔴 Critical | CI, fast feedback | 3 |
| **Service manifest + Supervisor** | L | 🔴 Critical | Platform v1 | 4 |
| **Structured error types** | M | 🟠 High | Audit, recovery automation | 5 |
| **Async IPC + executor** | M | 🟠 High | WASM async, composability | 6 |
| **Grapheme cluster terminal** | S | 🟠 High | Console v1 exit criteria | 7 |
| **ADR process** | XS | 🟢 Nice | Architecture governance | 8 |
| **Dependency viz script** | XS | 🟢 Nice | Ring boundary enforcement | 9 |
| **Custom clippy lints** | M | 🟢 Nice | Bootstrap pattern prevention | 10 |

**Effort:** XS=<1day, S=1-3days, M=1-2weeks, L=2-4weeks

---

## Appendix: Migration Checklist

### Phase 1: Foundation (Weeks 1-2)
- [ ] Add `alloc.rs` with `CapabilityAllocator` + `CapabilityVec`
- [ ] Extract `hal.rs` with `X86Hal` + `MockHal`
- [ ] Enable `test` feature + `cargo test --lib` passing
- [ ] Migrate `scheduler`, `capabilities`, `ipc` to dynamic allocator

### Phase 2: Host Testing (Week 3)
- [ ] Add `qemu-test-harness` crate
- [ ] GitHub Actions workflow for host + QEMU tests
- [ ] Pre-commit hooks configured
- [ ] All existing `self_check()` converted to `#[test]`

### Phase 3: Platform Primitives (Weeks 4-6)
- [ ] Service manifest TOML schema + parser
- [ ] Supervisor with dependency resolution
- [ ] Structured error types across kernel
- [ ] Async IPC + executor integration

### Phase 4: Console v1 Polish (Weeks 7-8)
- [ ] Grapheme cluster support in terminal
- [ ] Search in scrollback
- [ ] Recovery console extracted to separate crate
- [ ] ADRs for all major decisions

---

*This document is living. Update as decisions are made and implementation progresses.*
