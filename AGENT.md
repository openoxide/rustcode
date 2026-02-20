# AGENT.md — Rustcode System Rules

## Core Rules

1. **All tests must pass at all times** — `cargo test --workspace` is the gate
2. **`cargo fmt` must always be run** before committing
3. **Clippy warnings are not ignored** — `cargo clippy --workspace -- -D warnings`
4. **No `unwrap()` in production paths** — use `?`, `.ok_or()`, or proper error handling
5. **Proper typed error handling** — use `thiserror` for library crates, `anyhow` for binary
6. **Public APIs documented** — all `pub` items must have `///` doc comments
7. **No blocking in async paths** — use `tokio::task::spawn_blocking` if needed
8. **Streaming must support cancellation** — via `CancellationToken`
9. **All new features require dual reference check** (opencode + codex)
10. **No regressions allowed** — test count must never decrease
11. **Cross-platform compatibility enforced** — macOS + Linux
12. **No file may exceed 600 lines of code** — split into modules if approaching limit
13. **Fix before building** — all existing failing tests and clippy warnings must be resolved before starting any new milestone
14. **Manual testing required** — run `/manual-testing` workflow after every milestone or feature completion

## Development Discipline

- Every milestone must leave the system buildable
- **Commit work after every milestone completion** (`git add -A -S && git commit`) even if you are doing in phases so commit each phase of milestone.
- No incomplete feature merges
- Tests written before or alongside implementation
- Update docs after each milestone
- **Always sync `docs/TASKS.md`** from the artifact `task.md` after every task update
- Maintain milestone tracking during compaction
- `unsafe_code = "forbid"` (enforced in `Cargo.toml`)
- Always commit after you complete a big feature no co authors should be there [always use signed commits -S]

## Architecture

```
rustcode (binary)
├── rustcode-core      — Domain types, traits, events, sessions, permissions
├── rustcode-engine    — Agent runtime, tool execution, command handling
├── rustcode-llm       — Provider abstraction, streaming, transforms
├── rustcode-config    — Multi-layer config with trust model
├── rustcode-auth      — OAuth + API key credential store
├── rustcode-mcp       — Model Context Protocol HTTP + stdio sessions
├── rustcode-state     — File-based session persistence
├── rustcode-io        — Filesystem + process port abstractions
├── rustcode-plugins   — Plugin trait + registry
└── rustcode-tui       — Terminal UI with ratatui
```

## Reference Projects

- **opencode** (`../opencode`) — TypeScript CLI + Zig OpenTUI — feature parity target
- **codex** (`../codex/codex-rs`) — Rust CLI — architectural reference

## Testing & Credentials

- User is available to provide API keys, OAuth tokens, or other credentials for end-to-end testing
- Ask when needed for: LLM provider auth, MCP server testing, OAuth flow verification
- Do not hardcode credentials — use env vars or the auth store

## Dual Reference Rule

For **every** feature, bug fix, or refactor:
1. Inspect opencode implementation
2. Inspect codex implementation
3. Compare design tradeoffs
4. Implement idiomatic, production-grade Rust version
