# Flow

Flow is the statically typed language for command composition, automation, and system interaction in LogOS.

Flow source files use the `.flow` extension.

---

## 1. Purpose

Flow provides a safe, structured, capability-aware language for:

- interactive terminal commands;
- reusable scripts;
- system administration;
- service orchestration;
- scheduled automation;
- recovery procedures;
- remote operation;
- AI-generated workflows;
- application and tool integration.

Flow does not replace Rust as the implementation language of LogOS. It expresses operations through LogOS services.

> Make system automation composable, type-safe, inspectable, and safe to execute.

---

## 2. Architectural Position

Flow belongs primarily to the **Sessions** and **Runtime** rings.

```text
Experience
    |
Runtime
    |-- Flow execution runtime
    |-- packages
    |-- detached and scheduled jobs
    |
Sessions
    |-- shell
    |-- command registry
    |-- interactive Flow evaluation
    |-- terminal model
    |
System
    |-- supervisor
    |-- identity
    |-- storage
    |-- networking
    |-- audit
    |
Foundation
    |
Core
```

The language remains separate from the shell and terminal:

```text
Terminal
    |
Shell
    |
Flow evaluator
    |
Command registry
    |
LogOS services
```

The terminal is a presentation surface. The shell manages interactive sessions. Flow expresses typed operations. The command registry exposes system operations. The runtime executes scripts and manages their lifecycle.

Flow must never be required for Core correctness. The recovery console must remain usable without it.

---

## 3. Naming Register

| Concept               | Name           |
| --------------------- | -------------- |
| Language              | Flow           |
| Source file           | `.flow`        |
| Compiler and CLI      | `flow`         |
| Interactive evaluator | Flow REPL      |
| Language server       | `flow-ls`      |
| Formatter             | `flow fmt`     |
| Static checker        | `flow check`   |
| Package manifest      | `flow.toml`    |
| Lock file             | `flow.lock`    |
| Compiled artifact     | `.flowc`       |
| Package archive       | `.flowpkg`     |
| Main specification    | `docs/FLOW.md` |

Examples:

```text
backup.flow
repair-network.flow
deploy.flow
rotate-secrets.flow
```

Initial tooling:

```text
flow run backup.flow
flow check backup.flow
flow fmt backup.flow
flow explain backup.flow
flow capabilities backup.flow
```

Only `.flow` source files are required initially. The compiled and package formats remain provisional until their versioning models stabilize.

---

## 4. Design Principles

### 4.1 Typed by default

Invalid operations should be rejected before execution whenever possible.

```flow
let disk: DeviceRef = device("nvme0");
service.restart(disk);
```

Expected diagnostic:

```text
error[F021]: type mismatch

expected ServiceRef
found DeviceRef

service.restart(disk)
                ^^^^
```

### 4.2 Structured values

Commands exchange typed values rather than text streams. Formatting happens only when values are rendered.

### 4.3 Capability-aware execution

Scripts receive no ambient authority. Required capabilities must be declarable, inferable where possible, reviewable before execution, enforced at runtime, and auditable.

### 4.4 Predictable semantics

Avoid:

- implicit casts;
- implicit text parsing;
- hidden global state;
- shell quoting complexity;
- context-sensitive syntax;
- silent error suppression;
- unrestricted dynamic evaluation.

### 4.5 Interactive and reusable

The same language must work in a one-line shell command, saved script, scheduled job, remote session, package, and AI-generated workflow.

### 4.6 AI-friendly

Flow should have familiar syntax, a small surface, formal grammar, machine-readable command schemas, structured diagnostics, deterministic formatting, and versioned documentation.

### 4.7 Mechanism over policy

Flow defines computation and service invocation. System policy remains in the appropriate LogOS services.

---

## 5. Goals

Flow should provide:

- static type checking;
- local type inference;
- typed commands;
- typed pipelines;
- records and enums;
- `Option<T>`;
- `Result<T, E>`;
- pattern matching;
- concise error propagation;
- resource references;
- capability declarations;
- async operations;
- cancellation;
- bounded execution;
- modules;
- packages;
- deterministic formatting;
- machine-readable diagnostics;
- REPL support;
- remote execution support.

---

## 6. Non-Goals

Flow v1 will not provide:

- a replacement for Rust;
- unsafe memory access;
- raw pointers;
- manual memory management;
- classes or inheritance;
- unrestricted macros;
- runtime reflection;
- operator overloading;
- mutable global state;
- native driver or kernel development;
- full POSIX or Bash compatibility;
- a UI framework;
- general-purpose systems programming.

---

## 7. Syntax Direction

Flow uses a modern C#-inspired syntax while retaining proven semantic ideas from Rust such as enums, pattern matching, `Result`, `Option`, and `?`. The goal is to maximize readability and familiarity while preserving strong static guarantees.

Flow v1 should not expose:

- lifetimes;
- traits;
- macros;
- explicit borrow syntax;
- `unsafe`;
- advanced generics;
- memory layout controls.

Illustrative example:

```flow
fn repair_failed_services() -> Result<RepairReport, RepairError> {
    let failed = services()
        |> where(service => service.health == Health.Failed);

    let mut restarted = [];

    for service in failed {
        restarted.push(service.restart()?);
    }

    Ok(RepairReport {
        restarted,
        completed_at: time.now(),
    })
}
```

Exact syntax remains provisional until the grammar phase.

---

## 8. Lexical Structure

- UTF-8 source files;
- `//` single-line comments;
- `/* ... */` block comments;
- source spans retained for every token and syntax node;
- identifiers should be Unicode-aware where practical;
- public APIs should prefer ASCII identifiers.

Initial keywords:

```text
let mut fn return if else for while match in break continue
true false use module pub enum record type async await spawn
try catch capabilities
```

Keep the final keyword set minimal.

---

## 9. Core Type System

### 9.1 Primitive types

```text
Unit
Never
Bool
Int
Float
Text
Bytes
Duration
Timestamp
```

Possible later additions:

```text
Decimal
Uuid
Uri
Version
```

### 9.2 Collections

```text
List<T>
Map<K, V>
Set<T>
Table<T>
Stream<T>
```

`Table<T>` represents records with a stable schema. `Stream<T>` represents asynchronous or incremental values.

### 9.3 Option and Result

```text
Option<T>
Result<T, E>
```

```flow
let owner: Option<UserRef> = service.owner;

match owner {
    Some(user) => user.inspect(),
    None => print("No owner"),
}
```

```flow
let report = service.restart(network)?;
```

### 9.4 Records

```flow
record RestartReport {
    service: ServiceRef,
    previous_state: ServiceState,
    current_state: ServiceState,
    completed_at: Timestamp,
}
```

Anonymous records may also be supported:

```flow
let summary = {
    restarted: 4,
    failed: 1,
};
```

### 9.5 Enums

```flow
enum ServiceState {
    Starting,
    Running,
    Stopping,
    Failed,
}
```

Enums may carry data:

```flow
enum ServiceError {
    Busy { retry_after: Duration },
    CapabilityDenied { required: Capability },
    NotFound { service: ServiceRef },
    Internal { message: Text },
}
```

### 9.6 Resource references

```text
ResourceRef<T>
ServiceRef
DeviceRef
UserRef
StoreRef
SecretRef
SessionRef
PackageRef
TaskRef
```

Resource references are not interchangeable with `Text`.

```flow
let network: ServiceRef = service("network");
let disk: DeviceRef = device("nvme0");
```

### 9.7 Capabilities

Capabilities may be represented as typed values when explicit delegation is required:

```text
Capability<T>
Permit<T>
```

Interactive scripts may use the current session capability context. Packages, automation, and delegated operations require explicit declarations.

### 9.8 Generics

Flow v1 supports generics only for built-in types:

```text
Option<T>
Result<T, E>
List<T>
Map<K, V>
Set<T>
Stream<T>
```

User-defined generics are postponed.

---

## 10. Type Inference and Conversion

Flow infers local types where unambiguous:

```flow
let count = 42;
let name = "network";
let services = system.services();
```

Explicit annotations remain available:

```flow
let network: ServiceRef = service("network");
```

Public functions should declare parameter and return types.

No implicit conversions:

```flow
let count: Int = "42"; // error
let count = Int.parse("42")?;
```

---

## 11. Variables and Mutability

Variables are immutable by default:

```flow
let state = ServiceState::Running;
```

Mutation must be explicit:

```flow
let mut attempts = 0;
attempts += 1;
```

Mutable global variables are excluded from v1.

---

## 12. Expressions and Control Flow

Flow is expression-oriented:

```flow
let status =
    if service.running {
        "running"
    } else {
        "stopped"
    };
```

Loops:

```flow
for service in services {
    print(service.name);
}
```

Pattern matching:

```flow
match service.restart(network) {
    Ok(report) => print(report),
    Err(ServiceError::Busy { retry_after }) => {
        sleep(retry_after);
        retry;
    }
    Err(ServiceError::CapabilityDenied { required }) => {
        audit.record(required);
    }
    Err(error) => return Err(error),
}
```

Enum matches should be exhaustive.

---

## 13. Functions and Closures

```flow
fn add(left: Int, right: Int) -> Int {
    left + right
}
```

Public function:

```flow
pub fn restart_network() -> Result<RestartReport, ServiceError> {
    let network = service("network");
    network.restart()
}
```

Closures support filtering and mapping:

```flow
let failed =
    services()
        |> where(service => service.health == Health.Failed);
```

Captured variables are immutable by default.

### 13.1 Parameters and named arguments

Parameter declarations use `name: Type`, with optional default values:

```flow
fn waitUntilOnline(
    interface: InterfaceRef,
    timeout: Duration = 30s,
) -> Result<Interface, NetworkError>
```

Named arguments use `name: value`:

```flow
network.waitUntilOnline(interface, timeout: 10s)
```

### 13.2 Extension methods

Flow supports extension methods so strongly typed APIs can be extended without inheritance or modifying the original type.

Illustrative syntax:

```flow
extension List<Service> {
    fn failed(self) -> List<Service> {
        self
            |> where(service => service.health == Health.Failed)
    }
}
```

Usage:

```flow
services().failed()
```

Exact extension declaration syntax remains provisional until the grammar phase.

---

## 14. Typed Pipelines

The proposed pipeline operator is `|>`.

```flow
services()
    |> where(service => service.health == Health.Failed)
    |> map(service => service.id)
    |> restart_each();
```

The type checker should infer:

```text
List<Service>
-> List<Service>
-> List<ServiceRef>
-> List<RestartResult>
```

Invalid pipelines fail statically:

```flow
services()
    |> map(|service| service.name)
    |> restart_each();
```

```text
error[F142]: invalid pipeline input

restart_each expects:
    List<ServiceRef>

pipeline provides:
    List<Text>
```

The same syntax should support `List<T>`, `Table<T>`, and `Stream<T>`.

Streaming pipelines must preserve back-pressure and bounded buffering.

### 14.1 Result and Option pipeline composition

Flow should support concise happy-path composition for `Result<T, E>` and `Option<T>`, but the syntax and semantics are not yet approved.

The language needs an explicit, coherent model for operations equivalent to:

- mapping a successful or present value;
- binding a function that returns another `Result` or `Option`;
- observing or transforming errors;
- recovering from errors;
- mapping absent values;
- preserving the wrapped type across pipelines.

A future design may introduce dedicated operators or concise pipeline modifiers rather than silently lifting every function call.

Illustrative intent only:

```flow
service("network")
    |> inspect()
    |> restart()
```

The final design must make it obvious whether each stage performs normal piping, `map`, `bind` / `andThen`, error mapping, or recovery. Automatic happy-path propagation must not silently catch, discard, or coerce errors.

This remains an open language-design issue.

---

## 15. Commands and Service Contracts

Every LogOS command exposes a typed signature:

```flow
fn service.restart(
    service: ServiceRef,
    mode: RestartMode,
) -> Result<RestartReport, ServiceError>;
```

The same schema powers:

- compilation;
- autocomplete;
- documentation;
- discovery;
- capability checks;
- remote invocation;
- AI tool descriptions;
- graphical forms;
- audit classification.

Discovery:

```flow
commands.search("restart service");
commands.describe("service.restart");
schemas.describe("RestartReport");
```

Namespaces should describe responsibility:

```text
service.restart
service.inspect
network.interfaces
store.open
session.list
audit.query
package.install
```

---

## 16. Error Model

Expected failures use `Result<T, E>`. Exceptions are not the primary error model.

Propagation:

```flow
let report = service.restart(network)?;
```

Recovery:

```flow
match service.restart(network) {
    Ok(report) => report,
    Err(ServiceError::Busy { retry_after }) => {
        sleep(retry_after);
        service.restart(network)?
    }
    Err(error) => return Err(error),
}
```

A Flow runtime panic must remain inside the script isolation boundary and produce structured diagnostics containing script identity, package identity, source location, stack trace, capability context, active command, and audit correlation.

---

## 17. Concurrency and Asynchrony

Initial primitives:

```text
async
await
spawn
timeout
select
cancel
```

Example:

```flow
async fn wait_for_network() -> Result<Interface, NetworkError> {
    network.events()
        |> where(|event| event.state == InterfaceState::Online)
        |> first()
        |> await
}
```

Structured concurrency is preferred. Child tasks belong to a parent scope unless explicitly detached by authorized runtime policy.

Leaving a scope should await or cancel child tasks and release their resources.

---

## 18. Resource Lifetimes

Flow does not require Rust-style borrow checking, but it does require deterministic cleanup.

Potential syntax:

```flow
using session = remote.connect(machine)?;
session.run(command)?;
```

Resource states may include:

- owned;
- shared;
- scoped;
- persistent reference.

The final model must remain simpler than Rust ownership.

---

## 19. Capabilities and Security

### 19.1 No ambient authority

Scripts receive only capabilities granted by:

- the current session;
- a package declaration;
- an automation policy;
- explicit delegation;
- a recovery context.

### 19.2 Package declaration

```toml
[capabilities]
service.inspect = ["service:/network"]
service.restart = ["service:/network"]
audit.write = true
```

### 19.3 Preflight

```text
flow capabilities repair-network.flow
```

Example output:

```text
Required capabilities:

- service.inspect(service:/network)
- service.restart(service:/network)
- audit.write
```

Static analysis does not replace runtime enforcement.

### 19.4 Secrets

Secrets use typed references:

```flow
let token: SecretRef = secrets.get("deployment-token")?;
```

A secret cannot be rendered or converted to `Text` implicitly.

---

## 20. Modules and Packages

Module imports:

```flow
use system.service;
use std.time;
use project.recovery;
```

Package layout:

```text
network-recovery/
├── flow.toml
├── flow.lock
├── src/
│   ├── main.flow
│   └── health.flow
├── tests/
│   └── recovery.flow
└── README.md
```

Manifest:

```toml
[package]
name = "network-recovery"
version = "0.1.0"
flow = "1"

[entry]
script = "src/main.flow"

[dependencies]
system-tools = "1.2"

[capabilities]
service.inspect = ["service:/network"]
service.restart = ["service:/network"]
audit.write = true
```

Packages should be versioned, signed, reproducible, inspectable, capability-declared, removable, and cacheable.

---

## 21. Standard Library

Keep the standard library small.

### Core

```text
Option
Result
List
Map
Set
Stream
Iterator
```

### Collections

```text
map
filter
where
fold
reduce
sort
group
first
last
count
collect
```

### Time

```text
time.now
sleep
timeout
Duration
Timestamp
```

### Serialization

```text
json.encode
json.decode
toml.encode
toml.decode
```

### Logging

```text
log.trace
log.debug
log.info
log.warn
log.error
```

Logging produces structured events.

Flow should avoid assuming all storage is a traditional filesystem:

```flow
let workspace = store.open("workspace:/main")?;
let file = workspace.read("config/settings.toml")?;
```

### 21.1 Collection literals

Collection types such as `List<T>`, `Map<K, V>`, and `Set<T>` belong to the standard library rather than being compiler-specialized implementations.

The language should still provide concise literals for common collections:

```flow
let services = [network, storage, audit];

let priorities = {
    "network": 1,
    "storage": 2,
};
```

The exact map and set literal syntax remains provisional.

### 21.2 String interpolation

Flow supports string interpolation for readable diagnostics, logging, paths, and interactive output:

```flow
let message = $"Restarted {service.name} in {report.duration}";
```

Interpolated expressions must satisfy an explicit display contract. Secrets and protected values must not become renderable implicitly.

---

## 22. Shell Integration

Flow should support concise interactive use:

```flow
services()
    |> where(service => service.health == Health.Failed)
```

Interactive features:

- history;
- multiline editing;
- autocomplete;
- signature help;
- inline diagnostics;
- value inspection;
- cancellation;
- job management;
- command discovery;
- capability previews.

The shell preserves typed values internally. Rendering occurs only at the terminal boundary.

---

## 23. Compiler Architecture

```text
Source
  |
Lexer
  |
Parser
  |
AST
  |
Name resolution
  |
HIR
  |
Type checking
  |
Capability analysis
  |
Bytecode IR
  |
Interpreter / VM
```

### Lexer

Tokenization, source spans, comments, escapes, numeric literals, duration literals.

### Parser

Syntax trees, error recovery, and incomplete input handling for the REPL.

### AST

Accurate source representation with spans for diagnostics, formatting, and refactoring.

### HIR

Normalized syntax with desugared pipelines, resolved names, and normalized patterns.

### Type checker

Inference, unification, built-in generics, exhaustiveness, command validation, pipeline validation, and resource-type validation.

### Capability analysis

Collect statically visible requirements and produce preflight reports. Runtime enforcement remains mandatory.

### Execution progression

1. tree-walking interpreter;
2. compact bytecode VM;
3. optional WASM component compilation later.

Correctness and diagnostics take priority over optimization.

---

## 24. Runtime

The Flow runtime owns:

- execution frames;
- runtime values;
- tasks;
- cancellation;
- resource scopes;
- capability context;
- command invocation;
- module loading;
- execution budgets;
- diagnostics.

Each execution receives:

- memory limits;
- instruction or CPU budgets;
- task limits;
- stream limits;
- timeout policies;
- capability boundaries;
- cancellation support.

Pure language evaluation should remain deterministic. System effects occur through explicit commands.

Scheduled and detached programs belong to the Runtime ring, not the interactive shell session.

---

## 25. Bytecode

The likely long-term execution model is a validated bytecode VM.

Requirements:

- compact;
- versioned;
- safe to deserialize;
- interruptible;
- inspectable;
- source-mappable;
- independent of host pointer size;
- explicit about service contract dependencies.

The `.flowc` extension must not be declared stable until bytecode versioning is finalized.

---

## 26. Relationship with WASM

Flow is best for:

- system automation;
- shell commands;
- orchestration;
- administration;
- short scripts;
- AI-generated workflows.

WASM is best for:

- larger applications;
- portable components;
- reusable libraries;
- third-party packages;
- computational workloads.

Future integration:

```text
Flow script
    |
imports WASM component
    |
invokes typed interface
```

Flow may eventually compile to WASM, but this is not an early requirement.

---

## 27. Compatibility

Bash, PowerShell, Nushell, and legacy tools should be supported at the edge through compatibility environments or explicit adapters.

Native Flow remains typed and capability-aware.

Legacy processes expose:

```text
stdin bytes
stdout bytes
stderr bytes
exit status
```

They do not define native Flow pipeline semantics.

---

## 28. AI Integration

The repository should provide:

```text
docs/
├── FLOW.md
├── GRAMMAR.ebnf
├── STANDARD_LIBRARY.md
├── COMMANDS.md
├── AI_CONTEXT.md
└── examples/
```

Machine-readable command schema:

```json
{
    "name": "service.restart",
    "description": "Restart a supervised service",
    "parameters": {
        "service": { "type": "ServiceRef" },
        "mode": {
            "type": "RestartMode",
            "default": "Graceful"
        }
    },
    "returns": "Result<RestartReport, ServiceError>",
    "capabilities": ["service.restart"]
}
```

Structured diagnostics:

```text
flow check script.flow --json
```

Expected AI workflow:

```text
generate
-> flow fmt
-> flow check --json
-> correct
-> flow capabilities
-> request approval
-> execute
```

Scripts or packages declare compatible versions:

```toml
[package]
flow = "1"
logos-api = "1"
```

---

## 29. Diagnostics and Formatting

Every diagnostic includes:

- stable error code;
- severity;
- message;
- source span;
- expected and actual types;
- contextual notes;
- reliable suggestions;
- machine-readable representation.

Example:

```text
error[F142]: invalid pipeline input
  --> repair.flow:8:8

8 |     |> restart_each();
  |        ^^^^^^^^^^^^

expected:
    List<ServiceRef>

received:
    List<Text>

help:
    map each service to `service.id` before calling `restart_each`
```

`flow fmt` must be deterministic and idempotent.

---

## 30. Language Server

`flow-ls` should provide:

- syntax and type diagnostics;
- autocomplete;
- command completion;
- signature help;
- hover documentation;
- capability previews;
- go to definition;
- find references;
- rename;
- formatting;
- code actions;
- service schema integration.

The language server reuses compiler crates.

---

## 31. Testing Strategy

Test:

- lexer token coverage and spans;
- parser validity and recovery;
- formatter idempotence;
- type inference and mismatches;
- pipeline propagation;
- enum exhaustiveness;
- runtime cleanup and cancellation;
- execution limits;
- capability grants, denial, and revocation;
- command registry integration;
- remote execution;
- audit correlation.

Fuzz:

- lexer;
- parser;
- bytecode validator;
- package decoder;
- structured diagnostic serialization.

---

## 32. Suggested Repository Structure

Early structure:

```text
flow/
├── Cargo.toml
├── README.md
├── docs/
│   ├── FLOW.md
│   ├── GRAMMAR.ebnf
│   └── AI_CONTEXT.md
├── crates/
│   ├── flow-core/
│   ├── flow-runtime/
│   └── flow-cli/
├── examples/
└── tests/
```

Split into more crates only when implementation pressure proves the boundaries.

Possible later crates:

```text
flow-source
flow-lexer
flow-syntax
flow-parser
flow-ast
flow-hir
flow-types
flow-check
flow-diagnostics
flow-runtime
flow-command-schema
flow-cli
flow-ls
```

---

# 33. Implementation Roadmap

## Phase 0 — Language Charter

- [ ] approve the Flow name;
- [ ] approve the `.flow` extension;
- [ ] define goals and non-goals;
- [ ] define architectural ownership;
- [ ] define initial syntax direction;
- [ ] define primitive types;
- [ ] define the command schema concept;
- [ ] create `docs/FLOW.md`;
- [ ] create `docs/GRAMMAR.ebnf`;
- [ ] add Flow to the LogOS architecture annex;
- [ ] record unresolved decisions.

**Exit criterion:** scope and ownership are clear enough to begin implementation without inventing architecture inside compiler code.

---

## Phase 1 — Source and Diagnostics

- [ ] source file abstraction;
- [ ] source spans;
- [ ] line and column mapping;
- [ ] stable diagnostic codes;
- [ ] human-readable diagnostics;
- [ ] JSON diagnostics;
- [ ] snapshot tests.

**Exit criterion:** every later compiler stage can report consistent human- and machine-readable errors.

---

## Phase 2 — Lexer

- [ ] identifiers and keywords;
- [ ] numbers and strings;
- [ ] booleans;
- [ ] punctuation and operators;
- [ ] comments;
- [ ] duration literals;
- [ ] invalid-token recovery;
- [ ] lexer tests;
- [ ] lexer fuzzing.

**Exit criterion:** valid source tokenizes deterministically and invalid input receives precise diagnostics.

---

## Phase 3 — Parser and AST

Initial syntax:

- [ ] literals;
- [ ] variables;
- [ ] `let`;
- [ ] blocks;
- [ ] function calls;
- [ ] function declarations;
- [ ] `if`;
- [ ] `for`;
- [ ] records;
- [ ] lists;
- [ ] field access;
- [ ] pipelines;
- [ ] return statements;
- [ ] parameter default values;
- [ ] named arguments;
- [ ] collection literals;
- [ ] string interpolation;
- [ ] extension method declarations and calls.

Infrastructure:

- [ ] parser;
- [ ] AST;
- [ ] precedence rules;
- [ ] error recovery;
- [ ] incomplete REPL input detection;
- [ ] AST printer;
- [ ] parser tests;
- [ ] parser fuzzing.

**Exit criterion:** representative Flow scripts parse into a stable AST.

---

## Phase 4 — Formatter

- [ ] `flow fmt`;
- [ ] pipeline formatting;
- [ ] block formatting;
- [ ] record formatting;
- [ ] import formatting;
- [ ] idempotence tests.

**Exit criterion:** formatting is deterministic and stable enough to establish syntax conventions.

---

## Phase 5 — Name Resolution and HIR

- [ ] lexical scopes;
- [ ] local variable resolution;
- [ ] function resolution;
- [ ] module placeholders;
- [ ] command namespace placeholders;
- [ ] HIR representation;
- [ ] pipeline desugaring;
- [ ] normalized patterns;
- [ ] unresolved-name diagnostics.

**Exit criterion:** every symbol is resolved or diagnosed before type checking.

---

## Phase 6 — Initial Type System

Initial types:

- [ ] `Unit`;
- [ ] `Bool`;
- [ ] `Int`;
- [ ] `Float`;
- [ ] `Text`;
- [ ] `List<T>`;
- [ ] records;
- [ ] function types.

Infrastructure:

- [ ] type representation;
- [ ] local inference;
- [ ] unification;
- [ ] explicit annotations;
- [ ] function checking;
- [ ] field checking;
- [ ] type mismatch diagnostics.

**Exit criterion:** invalid typed programs fail before execution with precise diagnostics.

---

## Phase 7 — Interpreter

- [ ] runtime values;
- [ ] stack frames;
- [ ] function calls;
- [ ] variables;
- [ ] blocks;
- [ ] conditions;
- [ ] loops;
- [ ] lists;
- [ ] records;
- [ ] built-in `print`;
- [ ] execution errors;
- [ ] interruption support.

Milestone:

```flow
fn main() {
    let x = 42;
    print(x);
}
```

**Exit criterion:** `flow run hello.flow` executes a statically checked program.

---

## Phase 8 — Enums, Option, Result, and Match

- [ ] enums;
- [ ] enum payloads;
- [ ] `Option<T>`;
- [ ] `Result<T, E>`;
- [ ] `match`;
- [ ] pattern binding;
- [ ] exhaustiveness checking;
- [ ] `?` propagation;
- [ ] `Never`;
- [ ] unreachable-code analysis.

**Exit criterion:** expected failures can be modeled without exceptions and non-exhaustive matches are rejected.

---

## Phase 9 — Command Schema

- [ ] schema format;
- [ ] command name resolution;
- [ ] parameter checking;
- [ ] return-type checking;
- [ ] command documentation;
- [ ] namespace organization;
- [ ] mock command registry;
- [ ] schema versioning proposal.

**Exit criterion:** Flow statically validates calls against a mock LogOS service catalogue.

---

## Phase 10 — Resource References

- [ ] `ResourceRef<T>`;
- [ ] `ServiceRef`;
- [ ] `DeviceRef`;
- [ ] `StoreRef`;
- [ ] parsing;
- [ ] display rules;
- [ ] equality;
- [ ] serialization;
- [ ] invalid-reference diagnostics.

**Exit criterion:** resource categories cannot be mixed accidentally and can cross command and remote boundaries safely.

---

## Phase 11 — Typed Pipelines

- [ ] pipeline type propagation;
- [ ] generic built-in operators;
- [ ] `map`;
- [ ] `where`;
- [ ] `filter`;
- [ ] `fold`;
- [ ] `collect`;
- [ ] pipeline diagnostics;
- [ ] collection pipeline tests;
- [ ] design explicit `Result<T, E>` and `Option<T>` pipeline composition;
- [ ] define syntax for mapping, binding, error mapping, and recovery;
- [ ] reject ambiguous implicit lifting.

**Exit criterion:** a pipeline from `List<Service>` to typed restart results checks end to end, while wrapped-value pipeline semantics are documented even if deferred.

---

## Phase 12 — REPL and Shell Integration

- [ ] incremental parsing;
- [ ] multiline input;
- [ ] session variables;
- [ ] history;
- [ ] result rendering;
- [ ] interruption;
- [ ] basic completion;
- [ ] shell integration contract;
- [ ] terminal integration contract.

**Exit criterion:** the normal LogOS shell evaluates Flow expressions while values remain typed until rendering.

---

## Phase 13 — Capability Analysis and Enforcement

- [ ] capability schema;
- [ ] static capability collection;
- [ ] `flow capabilities`;
- [ ] capability-aware command calls;
- [ ] runtime capability context;
- [ ] denial diagnostics;
- [ ] revocation handling;
- [ ] audit correlation.

**Exit criterion:** unauthorized service operations cannot execute and required authority can be reviewed beforehand.

---

## Phase 14 — Async, Streams, and Cancellation

- [ ] `async`;
- [ ] `await`;
- [ ] `spawn`;
- [ ] task values;
- [ ] `Stream<T>`;
- [ ] stream iteration;
- [ ] cancellation;
- [ ] timeouts;
- [ ] structured concurrency;
- [ ] bounded buffering;
- [ ] back-pressure.

**Exit criterion:** scripts safely consume service event streams and cancellation releases resources.

---

## Phase 15 — Modules

- [ ] module files;
- [ ] imports;
- [ ] public and private symbols;
- [ ] module graph;
- [ ] cycle detection;
- [ ] module diagnostics;
- [ ] standard-library module loading.

**Exit criterion:** multi-file Flow programs can be checked and executed.

---

## Phase 16 — Packages

- [ ] `flow.toml`;
- [ ] package identity;
- [ ] entry points;
- [ ] dependencies;
- [ ] `flow.lock`;
- [ ] capability declarations;
- [ ] integrity hashes;
- [ ] local cache;
- [ ] package signing design.

**Exit criterion:** a Flow package can be installed and executed reproducibly with inspectable capabilities.

---

## Phase 17 — Bytecode VM

- [ ] instruction set;
- [ ] bytecode encoder;
- [ ] validator;
- [ ] VM;
- [ ] source maps;
- [ ] instruction budgets;
- [ ] cancellation points;
- [ ] VM tests;
- [ ] validator fuzzing.

**Exit criterion:** checked programs execute through an interruptible validated VM.

---

## Phase 18 — Language Server

- [ ] `flow-ls`;
- [ ] diagnostics;
- [ ] completion;
- [ ] hover;
- [ ] signature help;
- [ ] go to definition;
- [ ] references;
- [ ] rename;
- [ ] formatting;
- [ ] code actions;
- [ ] command schema integration;
- [ ] capability preview.

**Exit criterion:** Flow development is practical in external editors and the future LogOS editor.

---

## Phase 19 — AI Tooling

- [ ] `flow check --json`;
- [ ] `flow explain --json`;
- [ ] command catalogue export;
- [ ] type schema export;
- [ ] `AI_CONTEXT.md`;
- [ ] compact language reference;
- [ ] generation examples;
- [ ] validation loop examples;
- [ ] capability review workflow.

**Exit criterion:** an external AI can generate, validate, repair, and explain Flow scripts using exported context.

---

## Phase 20 — Remote Execution

- [ ] remote execution request;
- [ ] source upload;
- [ ] package reference execution;
- [ ] typed output streaming;
- [ ] cancellation;
- [ ] capability negotiation;
- [ ] audit correlation;
- [ ] resumable job references.

**Exit criterion:** the same script runs locally or remotely without semantic changes and remote clients receive structured values.

---

## Phase 21 — Persistent Automation

- [ ] persistent job definitions;
- [ ] schedules;
- [ ] event triggers;
- [ ] execution history;
- [ ] retry policies;
- [ ] backoff;
- [ ] capability snapshots;
- [ ] owner identity;
- [ ] suspension;
- [ ] reboot recovery.

**Exit criterion:** Flow defines durable automation independent of an interactive session.

---

## Phase 22 — Package Distribution

- [ ] registry protocol;
- [ ] signatures;
- [ ] publisher identities;
- [ ] dependency policy;
- [ ] review metadata;
- [ ] capability summaries;
- [ ] reproducible builds;
- [ ] registry mirroring;
- [ ] offline installation.

**Exit criterion:** third-party packages can be evaluated, installed, audited, and removed safely.

---

## Phase 23 — Compatibility Environment

- [ ] external process service;
- [ ] byte-stream adapters;
- [ ] exit status model;
- [ ] environment isolation;
- [ ] Bash compatibility investigation;
- [ ] Nushell interoperability investigation;
- [ ] migration tooling;
- [ ] structured-data adapters.

**Exit criterion:** legacy tools are usable without weakening native Flow semantics.

---

## Phase 24 — Flow 1.0

- [ ] stable grammar;
- [ ] stable core type system;
- [ ] stable command schema;
- [ ] stable capability model;
- [ ] stable package manifest;
- [ ] stable diagnostics format;
- [ ] stable formatter;
- [ ] language server;
- [ ] reference documentation;
- [ ] security review;
- [ ] performance review;
- [ ] compatibility policy;
- [ ] migration policy.

Flow 1.0 should not be declared until existing scripts can be expected to remain valid across normal LogOS updates.

---

## 34. Early Milestones

### Milestone A — Parse

```flow
let message = "Hello, Flow";
print(message);
```

### Milestone B — Check

```flow
let count: Int = "invalid";
```

### Milestone C — Run

```flow
fn main() {
    for value in [1, 2, 3] {
        print(value);
    }
}
```

### Milestone D — Model errors

```flow
fn divide(left: Int, right: Int) -> Result<Int, MathError> {
    if right == 0 {
        Err(MathError::DivisionByZero)
    } else {
        Ok(left / right)
    }
}
```

### Milestone E — Call a LogOS service

```flow
fn main() -> Result<Unit, ServiceError> {
    let network = service("network");
    service.restart(network)?;
    Ok(())
}
```

### Milestone F — Typed pipeline

```flow
services()
    |> where(service => service.health == Health.Failed)
    |> map(service => service.id)
    |> restart_each();
```

### Milestone G — Capability preflight

```text
flow capabilities repair-network.flow
```

### Milestone H — Remote execution

```text
flow run --on machine:/home-server repair-network.flow
```

---

## 35. Open Decisions

### Syntax

Approved:

- Closures use `parameter => expression`.
- Enum members use `Type.Member`.
- The pipeline operator is `|>`, representing left-to-right value piping into free functions.
- Blocks use braces (`{ ... }`).
- Whitespace is not semantically significant.
- `flow fmt` provides deterministic formatting.
- The overall syntax direction is modern C#-inspired while retaining Rust's `Result`, `Option`, `match`, and `?`.
- Parameter declarations use `name: Type = default_value`.
- Named arguments use `name: value`.
- Flow has no general-purpose `null`; absence is represented with `Option<T>`.
- Extension methods are supported for strongly typed APIs.
- Collection implementations belong to the standard library, while concise collection literal syntax remains a language feature.
- String interpolation is supported, subject to strict display and secret-safety rules.

Remaining decisions:

- [ ] semicolon policy;
- [ ] final pipeline operator;
- [ ] closure syntax;
- [ ] module syntax;
- [x] named arguments use `name: value`; declarations use `name: Type = default`;
- [ ] duration literals;
- [ ] resource literal syntax;
- [ ] `using` syntax.
- [ ] happy-path pipeline composition syntax for `Result<T, E>` and `Option<T>` (`map`, `bind`, error mapping, and recovery);

### Type system

- [ ] nominal versus structural records;
- [ ] numeric conversion rules;
- [x] no general-purpose `null`; use `Option<T>`;
- [ ] user-defined generics;
- [ ] effect typing;
- [ ] capability typing depth;
- [ ] union types;
- [ ] table semantics.

### Runtime

- [ ] interpreter lifespan;
- [ ] bytecode design;
- [ ] garbage collection strategy;
- [ ] memory quotas;
- [ ] instruction budgets;
- [ ] persistent task serialization.

### Service integration

- [ ] schema source format;
- [ ] schema versioning;
- [ ] remote schema discovery;
- [ ] overload policy;
- [ ] compatibility across service versions.

### Packages

- [ ] registry protocol;
- [ ] signing model;
- [ ] dependency resolver;
- [ ] lock-file format;
- [ ] capability approval UX.

### Security

- [ ] capability delegation syntax;
- [ ] authority narrowing;
- [ ] secret-reference behavior;
- [ ] audit requirements;
- [ ] script identity;
- [ ] recovery-mode exceptions.

---

## 36. Architectural Review Checklist

### Language boundary

- Does this belong in Flow or in a LogOS service?
- Does it introduce general-purpose complexity without a system automation use case?

### Type safety

- Can invalid use be rejected before execution?
- Does the feature require implicit conversion?
- Are errors explicit?

### Authority

- What capability is required?
- Can it be inspected before execution?
- Is runtime enforcement preserved?

### Resource ownership

- What resources are opened?
- When are they released?
- What happens during cancellation?

### Failure containment

- Can the script crash another service?
- Can it exhaust machine resources?
- Can it escape its execution budget?

### Remote behavior

- Are semantics preserved remotely?
- Are values serialized structurally?
- Can execution be cancelled?

### AI behavior

- Can the feature be described through schemas?
- Can diagnostics explain mistakes?
- Does it introduce ambiguous syntax?

### Compatibility

- Does it break existing `.flow` files?
- Is a migration path available?
- Is versioning explicit?

---

## 37. Illustrative Network Recovery Script

```flow
use system.audit;
use system.network;
use system.service;
use std.time;

record RecoveryReport {
    interface: InterfaceRef,
    restarted_services: List<ServiceRef>,
    completed_at: Timestamp,
}

enum RecoveryError {
    InterfaceNotFound { name: Text },
    ServiceFailure { source: ServiceError },
    NetworkFailure { source: NetworkError },
}

pub async fn recover_network(
    interface_name: Text,
) -> Result<RecoveryReport, RecoveryError> {
    let interface =
        network.interfaces()
            |> where(|interface| interface.name == interface_name)
            |> first()
            |> ok_or(RecoveryError::InterfaceNotFound {
                name: interface_name,
            })?;

    let affected =
        service.list()
            |> where(|service| service.dependencies.contains(interface.id));

    let mut restarted = [];

    for target in affected {
        target
            .restart(RestartMode::Graceful)
            .map_err(|error| RecoveryError::ServiceFailure {
                source: error,
            })?;

        restarted.push(target.id);
    }

    network
        .wait_until_online(interface.id, timeout: 30s)
        .await
        .map_err(|error| RecoveryError::NetworkFailure {
            source: error,
        })?;

    let report = RecoveryReport {
        interface: interface.id,
        restarted_services: restarted,
        completed_at: time.now(),
    };

    audit.record("network.recovery.completed", report);

    Ok(report)
}
```

This example is illustrative. Its exact syntax is not normative until the grammar is approved.

---

## 38. Final Architectural Statement

Flow is:

> A small, statically typed, capability-aware language for structured system automation in LogOS.

Its defining properties are:

- typed commands;
- structured values;
- typed pipelines;
- explicit errors;
- resource-aware execution;
- capability preflight;
- runtime enforcement;
- cancellation;
- remote compatibility;
- machine-readable tooling.

Flow should remain easier to reason about than a general-purpose language and safer to automate with than a conventional text-oriented shell.

The final success criterion is:

> A human or AI agent can write a concise `.flow` script, verify its types and authority before execution, run it locally or remotely, and receive structured results without weakening LogOS's architectural boundaries.

## Section 39 — Revisions & Interactive Design Clarifications

> **Date:** 2026-07-25
> **Status:** Approved Language Architecture Clarifications
> **Scope:** Live REPL Execution, Compilation Tiering, and System Contrast

### 1. [REPL] Tiered Authority & Inline Escalation

- **Design Rule:** The preflight capability declaration (`flow capabilities <file>`) is enforced strictly for stored `.flow` scripts, packages, and background jobs.
- **Interactive Semantics:** The interactive REPL operates under the active user's session capability context. Unprivileged operations trigger non-blocking inline capability prompts rather than compilation errors.

### 2. [Runtime] Tiered Compilation & Execution Strategy

- **Design Rule:** Flow uses a tiered JIT/Interpreter pipeline to ensure zero-latency CLI use.
    1. **Live REPL / One-Liners:** Single-pass AST parsing $\rightarrow$ type unification $\rightarrow$ direct execution via interruptible VM bytecode interpreter ($<1\text{ms}$ startup).
    2. **Stored Scripts / Packages:** Full static type & capability analysis $\rightarrow$ compiled & cached `.flowc` bytecode.

### 3. [Diagnostics] Machine-Readable Feedback & AI Closed-Loop

- **Specification:** All Flow compiler and runtime diagnostics must expose a `--json` interface containing exact source spans, expected vs. actual types, required capabilities, and actionable fixes.
- **Rationale:** Enables local AI agents to perform deterministic code generation, static verification (`flow check --json`), self-correction, and capability negotiation without unstructured string parsing.

### 4. [System Paradigm] Shell Contrast & Data Flow

- **Core Principle:** Flow completely replaces Unix raw byte-stream pipes (`stdin`/`stdout`) with typed system values (`Table<T>`, `List<ResourceRef>`).
- **Presentation Isolation:** Value formatting (tables, JSON, plain text) occurs exclusively at the Terminal boundary (Ring 3/5). Intermediate commands in a pipeline always exchange typed objects.

---
