# CLI behavior contracts

This file documents behavior implemented in `crates/rustcode/src` and verified by tests.

## Global flags

- `--json`: JSON envelopes for commands that provide structured output.
- `--event-debug`: prints event envelopes for run/serve flows in human mode.
- `--trust-project-config`: enables project config loading.
- `--allow-network` / `--deny-network`: network policy overrides.

## Core commands

- `run <prompt>`: executes a run request; writes assistant output; stores run transcript in session state.
- `run --attach <url> <prompt>`: streams events from remote `/v1/run` SSE endpoint.
- `agent <prompt>`: executes agent flow with optional tool approval flags.
- `serve --listen <addr>`: serves health and run/session endpoints.

## Session commands

- `session new|list|show|fork`: session metadata and transcript management.
- `export [session_id]`: emits JSON export containing `session` and `messages`.
- `import <file>`: creates a new session and imports messages.

## Auth commands

- `auth set-key`, `auth set-oauth`, `auth status`, `auth list`, `auth methods`, `auth login`, `auth remove|logout`.
- `auth login` supports `api_key`, `oauth_device_code`, and `oauth_browser` based on provider capabilities.

## MCP commands

- `mcp list|status|get|add|remove|login|logout`.
- MCP login supports token import (`--from-env`) and browser OAuth (`--method oauth_browser`).
- MCP add/remove updates user or project config via `--scope`.

## GitHub/PR commands

- `github repo|status`.
- `pr checkout|create` wrappers around `gh` CLI.
