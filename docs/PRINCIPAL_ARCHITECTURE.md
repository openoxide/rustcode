# Rustcode Principal Architecture

Date: 2026-02-17
Status: v1 design baseline for implementation

## 1. Architectural Overview
Rustcode uses a hexagonal architecture with an event-driven application core. Domain logic is isolated in `rustcode-core`; all side effects (filesystem, process execution, network/LLM, UI rendering) are adapter concerns.

The execution path is deterministic:
`CLI parse -> Command router -> immutable CommandContext -> Engine -> Event stream -> Renderer (stdout JSON/human or TUI)`.

Why this shape:
- It prevents UI/domain coupling and keeps TUI replaceable.
- It preserves testability by running the engine against in-memory adapters.
- It supports long-running sessions via structured, cancellable tasks.
- It enforces explicit contracts around events, errors, and plugin hooks.

Rejected alternatives:
- Fat `main.rs` with direct imperative command handling: fast initially, degrades into untestable control flow.
- Shared mutable singleton state: simpler wiring, unstable under concurrency and plugin growth.
- UI-driven core (TUI owns state): blocks headless execution and breaks machine interfaces.

## 2. Module Tree
```text
rustcode/
├── Cargo.toml
├── rust-toolchain.toml
├── docs/
│   └── PRINCIPAL_ARCHITECTURE.md
└── crates/
    ├── rustcode-cli/      # clap parser, command routing, startup wiring
    ├── rustcode-core/     # domain commands/events/context/errors/ports
    ├── rustcode-engine/   # orchestrator, command execution state machine
    ├── rustcode-config/   # layered config loading + validation + trust policy
    ├── rustcode-io/       # fs/process adapters and boundary checks
    ├── rustcode-llm/      # model client adapter contracts
    ├── rustcode-plugins/  # plugin registry + lifecycle hooks
    └── rustcode-tui/      # event consumer UI runtime
```

Domain boundaries:
- `core`: no IO, no UI, only contracts and domain types.
- `engine`: no direct stdout/stderr; emits events only.
- `cli` and `tui`: adapters that render and collect input.
- `config`: produces immutable config snapshots.

Config precedence baseline:
- `defaults -> global config -> user config -> project config (trusted only) -> env -> CLI flags`
- Project-local config is rejected unless explicitly trusted via CLI/env or trusted-project list.

## 3. Core Trait/Interface Definitions
```rust
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute(
        &self,
        command: Command,
        context: CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError>;
}

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: Event) -> Result<(), PublishError>;
}

#[async_trait]
pub trait ProcessPort: Send + Sync {
    async fn run(&self, program: &str, args: &[String], cwd: &Path)
        -> Result<ProcessOutput, IoError>;
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    async fn on_event(&self, event: &Event, ctx: &CommandContext) -> Result<(), PluginError>;
}
```

Design rule: traits live at boundaries, concrete implementations in adapters, and domain crate depends on traits only.

## 4. Event and Command Model
Commands are typed enums, not stringly-dispatched handlers. Events are append-only, structured, and renderer-agnostic.

Command shape:
- `Run { prompt }`
- `Exec { command, args }`
- `Tui`
- `Serve { listen }`
- `Version`

Event shape:
- Metadata: `id`, `timestamp`, `scope`
- Payload: `CommandAccepted`, `OutputChunk`, `Warning`, `Failure`, `Completed`

Rules:
- Engine emits all observable behavior as events.
- Renderers are pure projections (`Event -> terminal bytes`).
- JSON mode and TUI consume the same canonical event stream.

## 5. Concurrency Model
Runtime model is structured concurrency on Tokio:
- One orchestrator task per command execution.
- Bounded channel between engine and renderer (`mpsc`) to avoid unbounded memory growth.
- Cancellation propagated through task ownership and cancellation tokens.
- Background workers (LLM/tool calls) scoped to execution lifetime; no orphaned tasks.

State ownership:
- Immutable `CommandContext` (shared by `Arc`).
- Subsystems own their state privately.
- Avoid `Mutex`; prefer message passing and ownership transfer.

Rejected approach:
- Ad-hoc `tokio::spawn` without lifecycle tracking, which causes task leaks and nondeterministic shutdown.

## 6. Failure Model
Errors are explicit, typed domains:
- `ConfigError`
- `DispatchError` (planned)
- `ExecutionError`
- `ToolError` (planned)
- `TransportError` (planned)

Propagation policy:
- Adapter-level errors convert into domain errors with context.
- Engine emits `Failure` events for user-facing paths.
- Process exits are non-zero only at the CLI boundary.
- Panics are treated as bugs; expected failures must be represented as `Result`.

Shutdown policy:
- SIGINT/SIGTERM trigger cancellation.
- In-flight tasks receive cancellation and bounded drain window.
- Event channel closes only after final terminal events are emitted.

## 7. Extensibility Approach
Extension axis is ports/adapters, not core rewrites:
- Plugin interface for event hooks and future command/tool registration.
- Static plugins first for safety and ABI stability.
- WASM/dynamic plugin loading deferred until trait boundary stabilizes.

Versioning:
- `rustcode-core` contract changes are semver-governed.
- Event schema version added before external integrations are promoted.
- Backward compatibility tested with golden JSON snapshots.

Current extension seam in implementation:
- `PermissionPolicy` hook for path operations (`list/read/write/edit`) allowing policy replacement without engine refactor.

Why not dynamic loading first:
- It front-loads ABI/security complexity and slows stabilization of core contracts.

## 8. Common Rust CLI/TUI Pitfalls
- Letting `main.rs` become an orchestration monolith.
- Mixing render logic with execution logic.
- Unbounded channels causing memory spikes in streaming mode.
- Hidden global config state accessed from deep call stacks.
- Blocking filesystem/process calls on async runtime threads.
- Plugin hooks allowed to panic and poison execution flow.
- Non-deterministic cancellation semantics (double Ctrl+C races).

Mitigation in this design:
- Event-only render boundary.
- Bounded queues + explicit cancellation.
- Typed config snapshot passed through context.
- Adapter isolation and domain-only core.

## 9. Trade-off Analysis
Primary trade-offs:
- More upfront structure vs faster one-file prototype.
- Slightly higher abstraction cost vs long-term extensibility safety.
- Message-driven architecture adds event plumbing but unlocks multiple UIs and machine interfaces.

What we optimize for:
- Reliability under concurrency.
- Debuggability and observability.
- Extensible architecture without core rewrites.

What we intentionally defer:
- Dynamic plugin loading.
- Advanced UI features.
- Aggressive micro-optimizations before benchmark evidence.
