# Architecture

## Runtime flow

1. `crates/rustcode/src/main.rs` parses CLI args and handles early-return commands (`version`, `auth`, `mcp`, `models`, `session`, `export`, `import`, `github`, `pr`, `tui`).
2. For engine-backed commands (`run`, `agent`, `exec`, `list`, `read`, `write`, `edit`, `serve`) it:
   - loads effective config from `rustcode-config`,
   - initializes session context from `rustcode-state`,
   - builds LLM client from `rustcode-llm` when required,
   - optionally builds MCP registry,
   - executes through `rustcode-engine`.
3. Events are rendered as human or JSON envelopes and written to stdout.

## Persistence

- Auth credentials: `rustcode-auth` store (path controlled by `RUSTCODE_AUTH_FILE`).
- Sessions/transcripts: `rustcode-state` store (path controlled by `RUSTCODE_SESSIONS_DIR`).

## Config layering

`rustcode-config` resolves config from environment, user/global config, and optional project config. Project config is gated by trust flags.

## Boundaries

- `rustcode-core` defines shared interfaces and wire contracts.
- `rustcode-engine` depends on ports/interfaces, not CLI.
- `rustcode` crate owns user-facing command contracts and formatting.
