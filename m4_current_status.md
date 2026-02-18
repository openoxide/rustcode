# M4 Progress Summary

## Completed (from previous session)

### 1. Instructions System ✅
- Created `instructions.rs` (330 LOC)
- Discovers and loads RUSTCODE.md or AGENTS.md from workspace/global config
- Priority order: workspace RUSTCODE.md → workspace AGENTS.md → global AGENTS.md
- 17 unit tests covering file discovery, precedence, and formatting

### 2. System Prompt Builder ✅
- Created `system_prompt.rs` (285 LOC)
- Builds full system prompt with:
  - Base identity and behavior guidelines
  - Model-specific hints (Claude, GPT, Gemini)
  - Environment context (working dir, platform, date, git status)
  - Loaded instruction files
- Integrated into `agent_runtime.rs`
- Tests cover all components

### 3. Retry Logic ✅
- Created `retry.rs` (157 LOC)
- Exponential backoff: 2s → 4s → 8s (max 30s)
- Default 3 retries for transient failures
- Integrated into agent loop LLM calls
- 5 unit tests for delay calculation and retry execution

### 4. Session Summary ✅
- Created `session_summary.rs` (145 LOC)
- Computes git diff stats (files changed, additions, deletions)
- Formats human-readable output
- 3 unit tests

## Status
- All 4 modules created and tested
- 310 tests passing (22 new from M4)
- Build clean
- **Next step**: Wire session_summary into engine_commands.rs to emit summary after agent runs

## Remaining M4 Items
- [ ] Message v2 format
- [ ] Prompt construction improvements
- [ ] Revert logic
- [ ] Status tracking
- [ ] Snapshot/share support
- [ ] Wire session summary into engine

## Notes
- All branding uses "rustcode" (never "opencode")
- Following dual-reference rule (opencode + codex)
- All files under 600 LOC limit
