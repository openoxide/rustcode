# Rustcode Production Transformation — Implementation Plan

## Overview

Transform rustcode from an incomplete prototype into a production-grade Rust CLI with full feature parity with [opencode](file:///Users/xenrugin/final-arc/opencode) and best architectural decisions from [codex-rs](file:///Users/xenrugin/final-arc/codex/codex-rs).

### Current State

| Metric | Value |
|--------|-------|
| Crates | 11 |
| Source files | ~80 `.rs` files |
| Tests | 217 (all passing) |
| Build status | ✅ Clean |
| LOC violations (≥600) | 1 (`mcp.rs` at exactly 600) |
| Files near limit (>500) | 8 |

### Gap Analysis Summary

| Feature Area | Rustcode | Opencode | Codex-rs |
|---|---|---|---|
| **Tools** | 7 basic (read, write, bash, ls, glob, webfetch, edit) | 19 (+ apply_patch, multiedit, codesearch, grep, plan, task, todo, batch, question, skill, websearch, lsp) | registry + orchestrator + parallel |
| **TUI** | Skeleton event counter | Full Zig OpenTUI | 60+ file ratatui TUI |
| **Compaction** | ❌ None | Full compaction + summary | compact task |
| **Patch/Diff** | ❌ None | `apply_patch.ts` | `apply-patch/` crate |
| **Skills** | ❌ None | discovery + loader + render | 8-file skills system |
| **Memories** | ❌ None | ❌ None | 6-file memories system |
| **LSP** | ❌ None | client + server + language | ❌ None |
| **Snapshot/Share** | ❌ None | snapshot + share system | ❌ None |
| **Provider** | 4 protocols (OpenAI, Anthropic, Google, Vercel) | models + auth + transform | models_manager with presets |
| **Auth** | ✅ OAuth + API key store | provider auth | login + keyring |
| **Session** | ✅ File-based store | SQL + message-v2 + processor | state/session + rollout |
| **Config** | ✅ Multi-layer + trust | config module | config crate |
| **MCP** | ✅ HTTP + stdio sessions | mcp + oauth | mcp + rmcp-client |
| **Structured logging** | Basic tracing | log util | otel_init |

> [!IMPORTANT]
> **600 LOC max rule**: Every source file must stay under 600 lines. Files currently at/near the limit must be split as part of Milestone 1.

## Proposed Changes

### Milestone 1: Foundation Hardening & Structural Cleanup

**Goal**: Split oversized files, create shared error/logging crates, eliminate `unwrap()` in prod paths.

---

#### [MODIFY] [mcp.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode/src/mcp.rs)
Split into `mcp/mod.rs`, `mcp/login.rs`, `mcp/manage.rs` — separate login flows from server management commands.

#### [MODIFY] [lib.rs (config)](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode-config/src/lib.rs)
Split into `lib.rs` (re-exports), `loader.rs` (ConfigLoader + apply_file), `validation.rs` (validate functions), `types.rs` (FileConfig structs).

#### [MODIFY] [agent_runtime.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode-engine/src/agent_runtime.rs)
Split into `agent_runtime.rs` (run_agent loop), `agent_permissions.rs` (resolve_permission_action, approval_fields), `agent_util.rs` (helpers).

#### [MODIFY] [lib.rs (state)](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode-state/src/lib.rs)
Split into `lib.rs` (re-exports), `store.rs` (SessionStore), `recorder.rs` (FileTranscriptRecorder), `util.rs` (helpers).

#### [MODIFY] [main.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode/src/main.rs)
Extract initialization logic into `init.rs`, keep `main()` as thin orchestrator.

#### [MODIFY] [auth.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode/src/auth.rs)
Split into `auth/mod.rs`, `auth/interactive.rs`, `auth/catalog.rs` (already exists).

#### [MODIFY] [transforms.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode-llm/src/transforms.rs)
Split by provider: `transforms/openai.rs`, `transforms/anthropic.rs`, `transforms/google.rs`, `transforms/vercel.rs`.

#### [NEW] rustcode-error crate
Unified error types with `thiserror`. Replaces scattered error types across crates with a single hierarchy.

#### [NEW] rustcode-logging crate  
Structured tracing setup with env-filter configuration, file rotation, and span-based context.

---

### Milestone 2: Tool Engine Expansion

**Goal**: Implement all 12 missing tools to match opencode, refactor tool registry following codex pattern.

Reference: opencode [tool/](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/tool), codex [tools/](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/tools)

#### [MODIFY] [agent_tools.rs](file:///Users/xenrugin/final-arc/rustcode/crates/rustcode-engine/src/agent_tools.rs)
Refactor into `tools/mod.rs` with a `ToolRegistry` trait and per-tool modules:

#### [NEW] `tools/apply_patch.rs` — Unified diff/patch application
#### [NEW] `tools/multiedit.rs` — Multi-file edit operations
#### [NEW] `tools/codesearch.rs` — AST-aware code search
#### [NEW] `tools/grep.rs` — ripgrep integration
#### [NEW] `tools/plan.rs` — Task planning tool
#### [NEW] `tools/task.rs` — Sub-agent task delegation  
#### [NEW] `tools/todo.rs` — Todo tracking
#### [NEW] `tools/batch.rs` — Parallel tool execution
#### [NEW] `tools/question.rs` — User questions during agent loop
#### [NEW] `tools/skill.rs` — Skill invocation
#### [NEW] `tools/websearch.rs` — Web search integration
#### [NEW] `tools/lsp.rs` — LSP-powered tool

Each tool module: struct + `execute()` + `spec()`, all under 600 LOC.

---

### Milestone 3: Context Window Management & Compaction

Reference: opencode [compaction.ts](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/session/compaction.ts), codex [tasks/compact.rs](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/tasks/compact.rs)

#### [NEW] `rustcode-context` crate
- `token.rs` — Token counting/estimation
- `window.rs` — Context window tracking and budget
- `compaction.rs` — Compaction strategy engine
- `summary.rs` — Summary generation for compacted messages

---

### Milestone 4: Session & State Enhancement

Reference: opencode [session/](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/session), codex [state/](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/state)

#### [MODIFY] `rustcode-state` crate — Add:
- `instruction.rs` — System/user instruction management
- `prompt.rs` — Prompt construction pipeline
- `retry.rs` — Retry logic with backoff
- `revert.rs` — Revert/undo operations
- `status.rs` — Session status tracking
- `summary.rs` — Session summary generation
- `snapshot.rs` — Session snapshot support

---

### Milestone 5: Provider & Auth Hardening

Reference: opencode [provider/](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/provider), codex [models_manager/](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/models_manager)

#### [MODIFY] `rustcode-llm` — Add:
- `models/registry.rs` — Full model registry with capability metadata
- `models/presets.rs` — Predefined model configurations
- `models/cache.rs` — Model info caching

---

### Milestone 6: TUI Architecture Overhaul

Reference: codex [tui/src/](file:///Users/xenrugin/final-arc/codex/codex-rs/tui/src), opencode OpenTUI

**Design principles** (from codex TUI):
- Separate UI state from engine state
- Component-based architecture with `bottom_pane/` split
- Snapshot-testable rendering

#### [MODIFY] `rustcode-tui` — Full rewrite of interactive module:
- `views/chat.rs` — Main chat view with streaming markdown
- `views/sessions.rs` — Session list and management
- `views/approval.rs` — Tool approval overlay
- `components/composer.rs` — Input composer with history
- `components/palette.rs` — Command palette / slash commands
- `components/footer.rs` — Status bar with keybinds
- `components/search.rs` — File search popup
- `components/scroll.rs` — Scroll state management
- `state.rs` — UI state (separated from engine state)
- `input.rs` — Input handling + key mapping
- `render.rs` — Core render loop
- `runtime.rs` — Event loop orchestration
- `markdown.rs` — Streaming markdown renderer

---

### Milestone 7: Advanced Features

#### [NEW] `rustcode-skill` crate — Skill discovery, loading, rendering
Reference: opencode [skill/](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/skill), codex [skills/](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/skills)

#### [NEW] `rustcode-worktree` crate — Worktree management
Reference: opencode [worktree/](file:///Users/xenrugin/final-arc/opencode/packages/opencode/src/worktree)

#### Memories integration into `rustcode-state`
Reference: codex [memories/](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/memories)

---

### Milestone 8: Production Reliability

#### [MODIFY] Various crates — Add:
- Crash recovery (state checkpoint + replay)
- `CancellationToken` audit across all async paths
- OpenTelemetry spans (reference: codex [otel_init.rs](file:///Users/xenrugin/final-arc/codex/codex-rs/core/src/otel_init.rs))
- File watcher for config reloading
- Cross-platform path handling validation

---

### Milestones 9–10: Testing & Polish

See the [verification plan](#verification-plan) below.

---

## Verification Plan

### Existing Test Suite (Preserved)
```bash
# Run full test suite — must remain green at all times
cargo test --workspace
```
Current: 217 tests across all crates (28 + 6 + 59 + 17 + 17 + 7 + 30 + 2 + 32 + 4 + 2 + 1 + 5 + 7).

### Automated Tests — New

```bash
# After each milestone, run:
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

**Per-milestone test targets:**

| Milestone | Test Type | What to Test |
|-----------|-----------|-------------|
| M1 | Unit | All split modules maintain existing behavior. Import paths resolve. |
| M2 | Unit + Integration | Each new tool: spec generation, argument validation, happy path execution, error paths. Reference: existing `tests/tool_validation.rs` pattern. |
| M3 | Unit | Token counting accuracy, compaction trigger thresholds, summary quality. |
| M4 | Unit + Integration | Instruction resolution, prompt assembly, retry backoff, revert correctness. |
| M5 | Unit | Model registry lookup, preset loading, provider auth flows. |
| M6 | Snapshot | TUI rendering snapshots (reference: codex `tui/tests/` using `insta`). |
| M7 | Unit + Integration | Skill discovery, loader, system skill rendering. |
| M8 | Integration + Stress | Cancellation test, concurrent access, crash-recovery replay. |

### Manual Verification

After Milestone 6 (TUI):
1. Run `cargo run -- tui` — verify main chat view renders
2. Type a message → verify streaming response appears
3. Press `/` → verify command palette appears
4. Navigate session list → verify sessions load

After Milestone 10:
1. Run `cargo build --release` — verify release build succeeds
2. Run `./target/release/rustcode --help` — verify CLI help is complete
3. Full end-to-end session: start → chat → tool use → compaction → session resume
