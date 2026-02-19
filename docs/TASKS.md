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
  - [ ] `lsp` — LSP-powered tool *(blocked on M7: LSP integration)*
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
- [ ] Design Rust-native TUI architecture (reference: codex `tui/src/`, opencode OpenTUI)
- [ ] Chat view with streaming markdown rendering
- [ ] Session list/management view
- [ ] Approval overlay system
- [ ] Command palette / slash commands
- [ ] File search popup
- [ ] Skill toggle view
- [ ] Feedback view
- [ ] Input composer with history
- [ ] Footer with keybinds/status
- [ ] Scroll state management
- [ ] Proper UI state separation from engine state

## Milestone 7: Advanced Features
- [ ] Skill system (reference: opencode `skill/`, codex `skills/`)
- [ ] Worktree management (reference: opencode `worktree/`)
- [ ] PTY subprocess (reference: opencode `pty/`)
- [ ] Scheduler (reference: opencode `scheduler/`)
- [ ] LSP integration (reference: opencode `lsp/`)
- [ ] IDE integration hooks (reference: opencode `ide/`)
- [ ] Memories system (reference: codex `memories/`)

## Milestone 8: Production Reliability
- [ ] Crash recovery
- [ ] Graceful cancellation (CancellationToken usage audit)
- [ ] Structured logging with spans
- [ ] OpenTelemetry integration (reference: codex `otel_init.rs`)
- [ ] Cross-platform validation (macOS + Linux)
- [ ] File watcher (reference: codex `file_watcher.rs`)

## Milestone 9: Testing & Verification
- [ ] Unit test coverage for all new modules
- [ ] Integration tests for agent loop
- [ ] Snapshot CLI tests
- [ ] Streaming tests
- [ ] Concurrency stress tests
- [ ] Failure mode tests
- [ ] Auth tests expansion
- [ ] Regression tests
- [ ] TUI snapshot tests (reference: codex `tui/tests/`)

## Milestone 10: Polish & Release
- [ ] CLI UX polish (help text, error messages, colors)
- [ ] Documentation (README, config docs, architecture docs)
- [ ] Release build validation
- [ ] Performance profiling
- [ ] Final code review pass
