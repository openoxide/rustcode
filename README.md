# rustcode

`rustcode` is a Rust CLI/TUI workspace for local command execution, sessioned run/agent workflows, provider diagnostics, auth credential management, and MCP server integration.

## Workspace crates

- `crates/rustcode`: CLI entrypoint and command routing.
- `crates/rustcode-core`: shared command/event/config/session types and traits.
- `crates/rustcode-engine`: command execution engine, agent tool execution, run/serve flow.
- `crates/rustcode-llm`: LLM provider resolution, diagnostics, and HTTP clients.
- `crates/rustcode-auth`: auth store and OAuth/device-code/browser flows.
- `crates/rustcode-config`: layered config loading and config editing for MCP servers.
- `crates/rustcode-mcp`: MCP transport clients.
- `crates/rustcode-state`: session and transcript persistence.
- `crates/rustcode-io`: filesystem/process adapters.
- `crates/rustcode-plugins`: plugin registry.
- `crates/rustcode-tui`: terminal UI runtime.

## Build and test

See `docs/BUILD_AND_TEST.md`.

## Command behavior and architecture

- CLI contracts: `docs/CLI_BEHAVIOR.md`
- System architecture: `docs/ARCHITECTURE.md`
- Provider doctests: `docs/PROVIDER_DOCTESTS.md`
