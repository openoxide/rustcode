# Rustcode Pre-Build Architecture Audit

Date: 2026-02-17
Mode: Slow Deep Research
Scope: Compare `opencode` and `codex` from UX, runtime, and architecture to derive a better `rustcode` design.

## Phase 0 - Environment Preparation and Real Usage

### Toolchains and build prerequisites
- `opencode` required Bun `^1.3.9`; environment had `1.3.4`, upgraded to `1.3.9`.
- `codex` pinned Rust `1.93.0`; environment had `1.75.0`, installed `1.93.0`.

### Build outcomes
- `opencode` dependencies installed (`bun install`, 3784 packages). Full build with `--single` stalled in cross-platform install step; successful local build with `./packages/opencode/script/build.ts --single --skip-install`.
- `codex` built successfully from source (`cargo build -p codex-cli`).

### Build-time metrics
- `opencode` local standalone build (`--single --skip-install`):
  - wall: `1.29s`
  - peak RSS: `~1.35GB` (`1351485744` bytes)
- `codex` CLI build:
  - wall: `212.95s` (`3m32s`)
  - peak RSS: `~2.30GB` (`2303606784` bytes)

### CLI behavior probes (help, invalid, minimal config, edge cases)

#### opencode
- `--help` cold: `1.22s`, peak RSS `~296MB`.
- `--version` warm: `0.44s`, peak RSS `~283MB`.
- Invalid top-level token (`opencode definitely-not-a-command`) is interpreted as project path, error is directory-related.
- Invalid run flag (`opencode run --not-a-flag`) exits non-zero but UX is weak because parser treats many tokens as message args.
- Minimal config (empty HOME/XDG) still works.
- Malformed `opencode.jsonc` produces good JSONC diagnostics (line/column + offending line).
- JSON streaming mode emits usable event objects (`step_start`, `tool_use`, `text`, `step_finish`).

#### codex
- `--help` cold: `1.99s`, peak RSS `~10MB`.
- `--version` warm: `0.00-0.01s`, peak RSS `~9MB`.
- `codex help definitely-not-a-command` returns precise clap error and usage.
- `codex exec --not-a-flag` returns precise parser error and tip.
- Minimal config (empty HOME/XDG) still works.
- Malformed `config.toml` under `CODEX_HOME` produces precise source diagnostics.
- Invalid UTF-8 stdin prompt returns explicit decode error guidance (no panic).
- JSON mode emits clean event stream (`thread.started`, `item.started/completed`, `turn.completed`).

### Large input / large file / interruption / network stress

#### opencode
- 200k-char prompt: completed; wall `19.71s`, peak RSS `~313MB`.
- Large binary attachment (`200MB`) explicitly rejected as binary file (good guardrail), then network error surfaced.
- Interactive TUI Ctrl+C: single Ctrl+C exits cleanly from main TUI.
- `run` mode Ctrl+C during active run exits with code 1 and caret marker (`^C`), minimal graceful messaging.
- Long-running `serve` sample (14s): RSS stable (`348448 -> 348256 KB`).
- Forced proxy/network failure surfaces connection errors quickly.

#### codex
- 200k-char prompt: completed; wall `9.21s`, peak RSS `~28MB`.
- Large file probe (`wc -c` on 200MB file via shell tool): completed; wall `8.23s`, peak RSS `~27MB`.
- Interactive TUI Ctrl+C behavior:
  - first Ctrl+C during trust prompt transitions into startup UI,
  - second Ctrl+C exits cleanly.
- Forced proxy/network failure retries 5 times with explicit reconnect logs, then final explicit stream error (`24.42s`).
- Long-running `app-server` sample (14s): RSS stable (`51584 -> 51552 KB`).

### Parallel command execution probe
- `codex exec --json` with explicit parallel command request emitted two concurrent command starts before completion (`item_2`, `item_3` both `in_progress`).
- Equivalent `opencode` probe executed commands sequentially (tool call timestamps ordered, no overlap).

### UX notes (direct usage)
- `opencode`: visually rich, but parser ergonomics are ambiguous due default positional project mode; high baseline memory/startup overhead.
- `codex`: stricter UX with trust gate and explicit parser errors; stronger machine-readable output contracts in `--json`.

## Phase 1 - Static Code Archaeology

## 1) Dependency and module graph

### codex (Rust workspace)
- Workspace crates: `68`.
- Runtime dependency DAG: acyclic (normal dependencies).
- Out-degree hubs:
  - `codex-core` (`83`)
  - `codex-tui` (`71`)
  - `codex-app-server`, `codex-cli` (`30`)
- In-degree hubs:
  - `codex-core` (`23`)
  - `codex-protocol` (`22`)
  - `codex-utils-absolute-path` (`13`)
- Interpretation: clear architectural gravity around `codex-core` and protocol contracts.

### opencode (TypeScript monorepo, module-level scan of `packages/opencode/src`)
- Alias-import module edges observed: `237`.
- High fan-out source modules:
  - `session` (`70`)
  - `cli` (`35`)
  - `project` (`16`)
  - `server` (`14`)
- High in-degree modules:
  - `util` (`56`)
  - `bus` (`51`)
  - `flag` (`13`)
  - `storage` (`12`)
- Module-cycle warnings present in top-level module graph, notably around:
  - `cli <-> server`
  - `permission <-> session <-> tool`
- Interpretation: architecture is powerful but tighter coupled across core modules.

## 2) God-object indicators (largest files)

### codex
- `core/src/codex.rs` (`8437` LOC)
- `core/src/config/mod.rs` (`5237` LOC)
- `cli/src/main.rs` (`1438` LOC)
- Strongly indicates central core/controller concentration.

### opencode
- `lsp/server.ts` (`2046` LOC)
- `session/prompt.ts` (`1955` LOC)
- `config/config.ts` (`1485` LOC)
- `provider/provider.ts` (`1327` LOC)
- Indicates concentrated behavior in session/config/provider/lsp.

## 3) Entry flow call graphs

### opencode
1. `packages/opencode/src/index.ts`
2. yargs parse + global middleware (`Log.init`, one-time DB migration, env setup)
3. command dispatch (`RunCommand`, `TuiThreadCommand`, `ServeCommand`, etc.)
4. config/project bootstrap (`bootstrap` -> `Instance.provide` -> `InstanceBootstrap`)
5. execution:
   - `run`: create internal SDK client bound to `Server.App().fetch`, subscribe event stream, dispatch `session.prompt`/`session.command`
   - default TUI: spawn worker, RPC bridge, optionally start server, render opentui app
6. output rendering:
   - run mode: inline text/tool renderers or JSON events
   - TUI mode: Solid/opentui renderer tree

### codex
1. `codex-rs/cli/src/main.rs`
2. clap parse (`MultitoolCli`) + top-level override normalization
3. subcommand router (`Exec`, `Review`, `Mcp`, `AppServer`, default TUI, etc.)
4. config resolution (`ConfigBuilder` -> `load_config_layers_state`)
5. execution engine (`codex_exec::run_main`):
   - build thread manager
   - start/resume thread
   - submit operation (`Op::UserTurn`/`Op::Review`)
   - event loop over protocol events
6. output rendering:
   - human event processor (`stderr`)
   - JSON event processor (`stdout` JSONL)

## 4) Subsystem readout

### opencode
- CLI parsing: yargs command modules.
- Config system: layered JSON/JSONC + `.opencode` dir scanning + plugin dependency install.
- Engine: session loop in `session/prompt.ts` + `LLM.stream`.
- LLM interface: provider abstraction around `ai` SDK in `session/llm.ts`.
- File I/O: Bun APIs and utility wrappers.
- Plugin mechanism: dynamic plugins + built-ins loaded at runtime.
- Caching/state: instance-scoped state maps via `Instance.state`.
- Logging: custom `Log` utility.
- Telemetry: optional experimental telemetry hooks.
- Error model: mostly exception + named errors; many `process.exit` paths.
- Concurrency model: worker RPC + async loops + interval scheduler.

### codex
- CLI parsing: clap derive, strict subcommand types.
- Config system: layered TOML with explicit loader stack + trust model.
- Engine: message-driven `Op` submission and protocol event loop.
- LLM interface: model client abstractions in `codex-core`.
- File I/O: typed path handling (`AbsolutePathBuf`) + policy checks.
- Plugin/extensibility: MCP servers + skills system (no ad-hoc JS plugin runtime).
- Caching/state: thread manager, skills cache, rollout/session state DB.
- Logging/telemetry: tracing + optional OTEL layers.
- Error model: typed errors + `Result` propagation; strong diagnostics for config/parser issues.
- Concurrency model: Tokio channels/tasks, cancellation tokens, thread listeners.

## Phase 2 - Runtime Behavior Analysis

### Performance
- Startup latency winner: `codex` by large margin (milliseconds) vs `opencode` (~0.4s warm, >1s cold).
- Heavy input winner: `codex` (faster and lower RSS on 200k prompt test).

### Memory
- Idle/help memory:
  - `opencode`: hundreds of MB baseline.
  - `codex`: tens of MB baseline.
- Long-running samples:
  - both stable over short observation windows (no immediate growth).

### Error handling
- `codex`: stronger parser/config diagnostics and clear non-zero exits; retry messaging for network failures is explicit.
- `opencode`: good JSONC errors, but command parser ambiguity and some mixed error flows (default positional path confusion).

### Concurrency
- `codex`: explicit concurrent command events observed.
- `opencode`: observed sequential execution in equivalent test prompt.
- Both support interrupt, but behavior differs:
  - `opencode run`: abrupt Ctrl+C UX.
  - `codex TUI`: requires repeated Ctrl+C in certain startup states.

## Phase 3 - Architectural Comparison

| System Area | opencode | codex | Winner | Why |
|---|---|---|---|---|
| CLI ergonomics | Rich commands, ambiguous default positional mode | Strict clap errors and guidance | codex | Better parser determinism and diagnostics |
| Abstraction cleanliness | Powerful but coupled modules, cycle signals | Clear crate boundaries and typed interfaces | codex | Stronger separation and compile-time contracts |
| Extensibility | Dynamic plugin runtime is flexible | MCP + skills are disciplined and typed | tie | opencode more open; codex safer contracts |
| Performance | Higher startup and baseline RSS | Much lower startup and memory footprint | codex | Measured runtime wins |
| Safety | Good safeguards but more implicit runtime behavior | Strong trust/config/sandbox policy framing | codex | Safer default operational model |
| Testability | Tests exist, but core hot paths are larger and dynamic | Extensive Rust tests and typed seams | codex | Better mockability and deterministic contracts |
| Developer experience | Fast TS/Bun iteration, rich TUI stack | Strong correctness model, slower build | tie | Depends on priority: iteration vs rigor |
| Architectural clarity | Command and session flow are understandable but broad | Clear command -> config -> op loop pipeline | codex | Cleaner control-flow narrative |

## Phase 4 - Extraction of Gold

### Keep
- `opencode`: flexible plugin ideas, rich TUI interaction patterns, practical JSON event mode.
- `codex`: strict config layering/trust model, message-driven event architecture, typed error+protocol boundaries, structured JSONL output.

### Remove / avoid
- Tight cross-module coupling and cycle-prone imports.
- Ambiguous positional CLI behavior.
- High baseline memory for simple commands.
- Global mutable state that is hard to reason about under concurrency.
- Output paths that mix human and machine concerns.

## Phase 5 - Rustcode Re-Architecture (proposed)

- Architecture style: hexagonal + message-driven core.
- Async-first runtime: Tokio.
- Domain isolation: command planning/execution in `rustcode-core`, IO in adapters.
- No direct printing from command handlers: emit structured domain events only.
- Explicit error domains:
  - `ConfigError`
  - `DispatchError`
  - `ExecutionError`
  - `ToolError`
  - `TransportError`
- State model:
  - immutable request context
  - actor-like services via channels
  - `Arc` + task ownership; avoid `Mutex` unless proven necessary
- Plugin path:
  - phase 1 static trait plugins
  - phase 2 optional WASM sandbox adapter
- UI:
  - TUI subscribes to event bus only
  - non-interactive mode renders same event stream in JSON/human adapters

## Phase 6 - Build Strategy and Targets

### Near-term milestones
1. Workspace skeleton + crate boundaries.
2. CLI parser + strict command router.
3. Layered config loader + trust rules.
4. Event protocol + renderer adapters (human + JSON).
5. Engine skeleton with cancellable task execution.
6. First built-in tools (`ls`, `read`, `edit`, `exec`).
7. Plugin trait surface.
8. TUI integration.
9. Benchmarks + load tests + docs.

### Initial measurable targets
- Warm startup: `<50ms` for non-interactive trivial commands.
- Idle RSS: significantly below `opencode` baseline (target low double-digit MB).
- Clean cancellation path for all long operations.
- JSON stream contract stable for automation.

### Key risks
- Over-centralizing logic into `core` god-objects.
- Async complexity creep from ad-hoc task spawning.
- Trait-object sprawl without bounded extension API.
- Mixing transport concerns into domain.

### Practical conclusion
- Use `codex` as the baseline for execution model, config trust, and event contracts.
- Borrow selectively from `opencode` where it is uniquely strong (plugin ergonomics and TUI interaction patterns).
- Build `rustcode` around explicit boundaries first, then optimize with benchmarks rather than pre-optimizing abstractions.
