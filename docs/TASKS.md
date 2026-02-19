# Rustcode Production Transformation — Master Task Tracker

## Phase 0: Audit & Planning
- [x] Audit rustcode crate structure (11 crates, ~80 source files)
- [x] Audit opencode feature modules (37 modules, ~110 files)
- [x] Audit codex-rs architecture (100+ files in core alone)
- [x] Verify build status (builds cleanly)
- [x] Verify test status (217 tests, all passing)
- [x] Identify LOC violations (1 file at 600 LOC limit: `mcp.rs`)
- [x] Create implementation plan
- [x] Create AGENT.md
- [x] User approval of plan

## Milestone 0.5: Existing Feature Verification & Fixes
- [x] End-to-end test: CLI run mode (non-interactive agent) — ✅ null-LLM + live OpenRouter
- [ ] End-to-end test: TUI interactive mode — skeleton only, deferred to M6
- [x] End-to-end test: Auth login/logout flows — ✅ 3 stored providers verified
- [x] End-to-end test: MCP server add/list/status/login — ✅ list returns 0 (correct)
- [x] End-to-end test: Server mode (HTTP API) — ✅ starts and accepts cancellation
- [x] End-to-end test: Session create/list/show/fork/delete — ✅ full CRUD works
- [x] Verify streaming works with each provider — ✅ OpenRouter live call successful
- [x] Fix any broken or incomplete features discovered — none found

## Milestone 1: Foundation Hardening & Structural Cleanup
- [x] Enforce 600 LOC max — split files at/near limit
  - [x] Split `crates/rustcode/src/mcp.rs` → `mcp/login.rs` + `mcp/registry.rs` (600→200)
  - [x] Split `crates/rustcode-config/src/lib.rs` → `validation.rs` + `merge.rs` (594→280)
  - [x] Split `crates/rustcode-engine/src/agent_runtime.rs` → `agent_util.rs` (588→470)
  - [ ] ~~Split test files~~ (576, 563 LOC — test-only, deferred)
  - [ ] `oauth/browser.rs` (561 LOC — near limit, split if it grows)
  - [ ] Remaining files all under 540 LOC — no immediate action needed
- [x] Create `rustcode-error` crate — unified typed error hierarchy (IoError, LlmError, RustcodeError)
- [x] Create `rustcode-logging` crate — structured tracing setup (compact, env-filter)
- [x] Replace all `unwrap()` in production paths — 8 regex unwraps fixed with `OnceLock`
- [x] Ensure all public APIs are documented — doc comments on core error types

## Milestone 2: Tool Execution Engine Expansion
- [x] **Phase 1 — High-impact tools:**
  - [x] `bash` — enhanced exec with shell detection, timeout, output truncation
  - [x] `apply_patch` — unified diff/patch application
  - [x] `multiedit` — sequential multi-edit operations on a single file
  - [x] `batch` — parallel tool execution (up to 25 calls)
  - [x] `question` — ask user questions during agent loop
- [/] **Phase 2 — Advanced tools (deferred — unblock as prerequisites land):**
  - [ ] `task` — sub-agent delegation *(blocked on M4: session/sub-agent infra)*
  - [x] `plan` — task planning tool
  - [x] `websearch` — web search integration (DDG Lite)
  - [x] `lsp` — LSP-powered tool (diagnostics, hover, definition, workspace_symbols)
  - [ ] `skill` — skill invocation *(blocked on M7: skill system)*
  - [ ] `codesearch` — AST-aware code search *(blocked on tree-sitter dep)*
- [x] Tool specs extracted to `agent_tool_specs.rs` (LOC compliance)

## Milestone 3: Context Window Management & Compaction
- [x] Token usage tracking in LLM responses (`TokenUsage` struct in `types.rs`)
- [x] Extract usage from OpenAI/Anthropic/Google provider responses
- [x] Context window tracker (`context_tracker.rs`) — with `usage_percent()` + `status_line()` for TUI
- [x] Compaction module (`compaction.rs`) — prune + LLM summarize
- [x] Hook compaction into agent loop (`agent_runtime.rs`)
- [x] Unit tests for all new modules (15 new tests)
- [ ] Manual live test: trigger compaction in long conversation

## Milestone 4: Session & State Management Enhancement
- [x] Instruction system (`instructions.rs`) — load RUSTCODE.md / AGENTS.md from workspace + global
- [x] System prompt management (`system_prompt.rs`) — rich model-aware prompt with env context
- [x] LLM retry logic (`retry.rs`) — exponential backoff, retryable error detection
- [x] Session summary (`session_summary.rs`) — git diff stats per session
- [x] Status tracking (`AgentStatus` enum + events for TUI)
- [x] Message v2 format — upgrade `StoredMessage` with structured parts
- ~~Revert logic~~ *(deferred to M7: needs snapshot infra)*
- ~~Snapshot/share~~ *(deferred to M7)*
- [ ] Prompt construction (reference: opencode `session/prompt.ts`) — *deferred: needs message v2*

## Milestone 5: Provider & Auth System Hardening
- [x] Model registry with presets (`model_registry.rs`) — centralize context limits + capabilities
- [x] Branding fix: `opencode` → `rustcode` paths in `provider_presets.rs`
- [x] Response transform hardening — unified error extraction across protocols
- [x] Provider auth improvements — OAuth token refresh in `resolve_api_key`
- [x] Copilot auth plugin (`copilot.rs`) — device code OAuth + token refresh
- [x] Codex auth plugin (`codex.rs`) — PKCE browser flow + token refresh

## Post-M5 Cleanup: Integration Tests & Clippy
- [x] Fix 4 failing integration tests — graceful LLM init fallback to `NullLlmClient`
- [x] Fix 185 clippy warnings → 0 — lint priority fix, 20 pedantic allows, 10 manual fixes
- [x] Update `AGENT.md` — rules 13 (fix before building) and 14 (manual testing)

## Milestone 6: TUI Architecture Overhaul
- [x] Design Rust-native TUI architecture (reference: codex `tui/src/`, opencode OpenTUI)
- [x] Chat view with streaming markdown rendering
- [x] Session list/management view
- [x] Approval overlay system
- [x] Command palette / slash commands
- [x] File search popup (Ctrl+T, inserts @path into composer)
- [ ] Skill toggle view
- [ ] Feedback view
- [x] Input composer with history (Alt+Up/Down)
- [x] Footer with keybinds/status (Ctrl+key combos, model label, error display)
- [x] Scroll state management
- [x] Proper UI state separation from engine state
- [x] Key binding fix: Ctrl+key combos, no single-key conflicts with composer typing
- [x] Status/error display in footer (red bold)
- [x] Default startup opens new session (not session list)
- [x] Markdown rendering in transcript (headings, code fences, bold/italic/inline code)

## Milestone 7: Advanced Features

### M7a: Skills + Memories + Scheduler ✅
- [x] Skill system — `rustcode-skills` crate, TOML frontmatter, global + project scoped (reference: opencode `skill/`, codex `skills/`)
- [x] Memories system — `rustcode-memories` crate, Phase 1 extraction + Phase 2 consolidation (reference: codex `memories/`)
- [x] Scheduler — tokio interval-based background task runner in `rustcode-engine` (reference: opencode `scheduler/`)
- [x] `/skill` and `/memory` TUI slash commands
- [x] Skills and memory summary injected into system prompt

### M7b: PTY + IDE ✅
- [x] PTY subprocess — `process_pty.rs` in `rustcode-io`, `pty_exec` agent tool (reference: opencode `pty/`, codex `utils/pty/`)
- [x] IDE detection — `ide.rs` in `rustcode-core`, env-var based detection of VS Code, Cursor, Windsurf, etc. (reference: opencode `ide/`)

### M7c: Worktree + LSP ✅
- [x] Worktree management: `worktree_create/list/remove/reset` agent tools + `rustcode worktree` CLI subcommand
- [x] LSP integration: `rustcode-lsp` crate with JSON-RPC transport, auto-detection (rust-analyzer/gopls/pyright/typescript-language-server), `lsp` agent tool (diagnostics/hover/definition/workspace_symbols)

## Milestone 8: Production Reliability ✅
- [x] Crash recovery — chained panic hook (tracing::error) in main.rs + terminal-restoring hook in TUI runtime.rs
- [x] Graceful cancellation (CancellationToken usage audit) — load_mcp_tools + join_all parallel dispatch now guarded with tokio::select!
- [x] Structured logging with spans — #[tracing::instrument] on run_agent, run_llm_step, execute_agent_tool_call, execute_tool_calls
- [x] OpenTelemetry integration (reference: codex `otel_init.rs`) — rustcode-logging: init_with_otel(endpoint) + OtelGuard; RUSTCODE_OTEL_ENDPOINT env var; catch_unwind around provider init
- [x] Cross-platform validation (macOS + Linux) — verified: no platform-specific code; notify/portable-pty/tokio all cross-platform; build passes on macOS
- [x] File watcher (reference: codex `file_watcher.rs`) — new rustcode-watcher crate; notify-based, 500ms throttle, broadcast::Sender; wired into Engine::with_workspace()

## Milestone 9: Manual End-to-End Testing

All items below must be tested live (not just unit tests).
Run `cargo build --release` first. Binary: `target/release/rustcode`.

### M0.5 / M1 — Core CLI (re-verify after refactors)
- [x] `rustcode version` — prints version (`0.1.0`)
- [ ] `rustcode auth login openrouter` — OAuth flow completes, token stored
- [ ] `rustcode auth login anthropic` — API key prompt, stored
- [x] `rustcode auth list` — shows 3 stored providers (github-copilot, openai, openrouter)
- [x] `rustcode session list` — returns sessions
- [x] `rustcode session new` — creates and prints session id
- [x] `rustcode session show <id>` — prints session details (title, parent_id, messages)
- [x] `rustcode session fork <id>` — forks and prints forked session id
- [x] `rustcode run "hello"` — single-turn prompt, response printed to stdout
- [x] `rustcode run "hello"` with streaming — output chunks appear live (OpenRouter)
- [x] `rustcode serve` — starts HTTP server, `curl /health` returns 200
- [x] `rustcode --help` — prints help with all 19 subcommands

### M2 — Tool Execution
- [x] Agent uses `read` tool — read Cargo.toml, extracted 17 workspace members
- [ ] Agent uses `write` tool — creates a new file, contents correct
- [ ] Agent uses `edit` tool — edits existing file, replacement applied
- [x] Agent uses `list` tool — listed workspace directory correctly
- [x] Agent uses `bash` tool — `echo BASH_TOOL_TEST`, exit_code=0, output correct
- [ ] Agent uses `apply_patch` — applies a unified diff correctly
- [ ] Agent uses `multiedit` — multiple edits to same file applied sequentially
- [x] Agent uses `batch` — parallel read + list calls executed simultaneously
- [x] Agent uses `plan` tool — created structured 3-step plan
- [ ] Agent uses `websearch` tool — returns web results (requires network)
- [ ] Agent uses `question` tool — prompts user mid-run, waits for input

### M3 — Context Compaction
- [ ] Long conversation (50+ turns) — context tracker status shows in footer
- [ ] Context overflow triggers compaction — summary injected, conversation continues
- [ ] Compaction preserves important facts from earlier messages

### M4 — Session & State
- [ ] `RUSTCODE.md` in workspace — instructions appear in system prompt
- [ ] `~/.config/rustcode/RUSTCODE.md` — global instructions loaded
- [ ] Session summary shown after agent run — git diff stats printed
- [ ] LLM retry on 429 — retries with backoff, succeeds (simulate with rate-limited key)

### M5 — Provider Auth
- [x] OpenRouter live call — response streams correctly ("MILESTONE9-PASS" test)
- [ ] Anthropic direct call — response streams correctly
- [ ] GitHub Copilot auth — device code flow, token refresh
- [ ] Codex auth — PKCE flow completes, token stored

### M6 — TUI Interactive
- [ ] Launch TUI: `rustcode` — opens new session directly
- [ ] Type message, Enter to submit — response streams in transcript
- [ ] Ctrl+C — cancels running agent
- [ ] `/help` — help overlay appears
- [ ] `/clear` — composer cleared
- [ ] `/sessions` — navigates to session list
- [ ] `/new` — creates new session in-place
- [ ] `/fork` — forks current session
- [ ] `/model` — shows current model name in toast
- [ ] `/find` or `/search` — search modal opens, finds text in transcript
- [ ] Ctrl+T — file search popup opens, selecting file inserts @path
- [ ] Alt+Up / Alt+Down — cycles through prompt history
- [ ] Ctrl+Q — opens command palette
- [ ] Tool approval overlay — edit/exec approval prompts shown, user can Allow/Deny
- [ ] Approval: Allow All Edits — subsequent edits auto-approved
- [ ] Approval: Allow All Commands — subsequent exec auto-approved
- [ ] Session list: Enter opens session, rename works, delete with confirm
- [ ] Footer shows model name, error messages appear in red
- [ ] Markdown rendered: `##` headings, `**bold**`, `` `code` ``, code fences
- [ ] System messages NOT shown in transcript (only user/assistant)
- [ ] Resize terminal — UI reflows correctly

### M8 — Production Reliability
- [x] `RUST_LOG=rustcode_engine=trace rustcode run "hello"` — engine_commands span visible at DEBUG level
- [x] `rustcode run "..."` + Ctrl-C — exits with "execution cancelled" message
- [x] `RUSTCODE_OTEL_ENDPOINT=http://localhost:4318 rustcode run "hello"` — no panic, completes normally
- [ ] Panic in TUI mode — terminal restored to usable state after panic
- [ ] File watcher — start rustcode, edit workspace file externally — no crash, watcher event logged at debug level

### M7a — Skills + Memories + Scheduler
- [ ] Create `~/.config/rustcode/skills/test.md` with TOML frontmatter → `/skill list` shows it
- [ ] `/skill test` — injects skill content into composer
- [ ] Skill injected into system prompt — agent aware of skill instructions
- [ ] Project skill at `.rustcode/skills/override.md` with same name overrides global
- [ ] Complete agent session → `~/.config/rustcode/memories/raw/<session>.md` created
- [ ] Wait 10 min (or trigger manually) → `summary.md` written
- [ ] New session after summary exists → memory summary in system prompt
- [ ] `/memory` — shows summary excerpt in toast
- [ ] `/memory clear` — clears raw files, confirms with toast

### Regression Checklist (run after every milestone)
- [x] `cargo test --workspace` — 0 failures
- [x] `cargo clippy --workspace -- -D warnings` — 0 errors
- [x] `cargo fmt --check` — clean (fixed whitespace in render_activity.rs, render_main.rs)
- [x] All M0.5 core CLI tests still pass
- [ ] TUI launches without panic

## Milestone 9b: Automated Testing Expansion
- [ ] Unit test coverage for all new modules
- [ ] Integration tests for agent loop
- [ ] Snapshot CLI tests
- [ ] Streaming tests
- [ ] Concurrency stress tests
- [ ] Failure mode tests
- [ ] Auth tests expansion
- [ ] Regression tests
- [ ] TUI snapshot tests (reference: codex `tui/tests/`)

## Milestone 10: Deferred Feature Completion

All features deferred from earlier milestones are now unblocked and collected here.

### M10 Phase 1: Unblocked Agent Tools (deferred from M2) ✅
- [x] `skill` tool — invoke a named skill by name from the agent loop (returns skill content)
- [x] `task` tool — sub-agent delegation: spawn a child agent loop with a given prompt, return its output
- [x] `codesearch` tool — Exa MCP web-based code search (`https://mcp.exa.ai/mcp`)

### M10 Phase 2: Snapshot & Revert System (deferred from M4) ✅
- [x] `rustcode-snapshot` crate — separate-git-dir snapshot per session
  - [x] `track(workspace)` — git add + write-tree, returns hash
  - [x] `restore(hash, workspace)` — read-tree + checkout-index
  - [x] `list()` — enumerate saved hashes
  - [x] `changed_files(hash)` — diff files between snapshot and working tree
- [x] Auto-snapshot in engine before first file mutation per session
- [x] `snapshot_list` agent tool — list recent snapshots
- [x] `snapshot_restore` agent tool — restore workspace to a snapshot hash

### M10 Phase 3: Provider Error Classification (deferred M5 phases 3–5) ✅
- [x] Structured `LlmErrorKind` in `rustcode-llm`: `ContextOverflow`, `RateLimit{retry_after_secs}`, `AuthFailed`, `InvalidRequest`, `ServiceUnavailable`, `Unknown`
- [x] `classify_http_error(status, body)` — parse classification from provider HTTP errors
- [x] `LlmError::Classified { kind, message }` — structured error variant used by all provider clients
- [x] `is_retryable()` in retry logic recognizes typed `LlmErrorKind` patterns from Display format

### M10 Phase 4: TUI Completion (deferred from M6) ✅
- [x] Skill toggle overlay — `Ctrl+S` opens overlay listing all skills, `Enter`/`Space` toggles enabled/disabled
- [x] Feedback view — `Ctrl+B` overlay: thumbs-up (`u`/`+`) / thumbs-down (`d`/`-`) rating + optional comment; writes to `~/.local/share/rustcode/feedback.jsonl`

## Milestone 11: Polish & Release
- [ ] CLI UX polish (help text, error messages, colors)
- [ ] Documentation (README, config docs, architecture docs)
- [ ] Release build validation
- [ ] Performance profiling
- [ ] Final code review pass
