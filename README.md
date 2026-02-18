# rustcode

`rustcode` is a Rust workspace that builds a CLI coding agent with a small set of deterministic core tools, multi-provider LLM support, and a basic TUI.

This repo is designed to be:
- buildable with a single Rust toolchain (`rust-toolchain.toml`)
- usable offline by default (network is opt-in)
- safe-by-default for local config and file mutation

## Build And Test

```bash
cargo build --workspace
cargo test --workspace
```

## Run The CLI

```bash
cargo run -q -p rustcode -- --help
cargo run -q -p rustcode -- --version

# Offline deterministic run (null provider)
cargo run -q -p rustcode -- run "hello"

# Tool-calling agent (requires a tools-capable provider)
cargo run -q -p rustcode -- agent "list files then read Cargo.toml"
```

## Provider Setup (Example)

Network use is gated. For one-off live runs, set `RUSTCODE_ALLOW_NETWORK=1`.

```bash
RUSTCODE_ALLOW_NETWORK=1 \
  cargo run -q -p rustcode -- \
  --llm-provider openrouter \
  --llm-base-url https://openrouter.ai/api/v1 \
  --llm-api-key-env OPENROUTER_API_KEY \
  run "hello"
```

See provider details and auth flows in `docs/PROVIDERS.md` and `docs/CREDENTIAL_REQUIREMENTS.md`.

## Models And Diagnostics

```bash
cargo run -q -p rustcode -- models
cargo run -q -p rustcode -- models openrouter

cargo run -q -p rustcode -- --json models
cargo run -q -p rustcode -- --json models openai
```

## Auth

```bash
cargo run -q -p rustcode -- auth methods openrouter
cargo run -q -p rustcode -- auth login openrouter --from-env OPENROUTER_API_KEY
cargo run -q -p rustcode -- auth status openrouter
cargo run -q -p rustcode -- auth remove openrouter
```

## Trust Model

Project-local config (`rustcode.toml`) is trust-gated. For trusted repos:

```bash
cargo run -q -p rustcode -- --trust-project-config models
```

## Docs

- Getting started: `docs/QUICKSTART.md`
- Providers: `docs/PROVIDERS.md`
- Credentials + env var matrix: `docs/CREDENTIAL_REQUIREMENTS.md`
- Provider coverage audit: `docs/PROVIDER_COVERAGE.md`
- Provider examples executed in CI (no network): `docs/PROVIDER_DOCTESTS.md`
