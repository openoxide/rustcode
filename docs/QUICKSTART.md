# Rustcode Quickstart

## Developer Setup
1. Install toolchain from `rust-toolchain.toml`.
2. Build workspace:
```bash
cargo build --workspace
```
3. Run tests:
```bash
cargo test --workspace
```

## Core Commands
```bash
cargo run -q -p rustcode -- --help
cargo run -q -p rustcode -- run "hello"
cargo run -q -p rustcode -- agent "scan the repo and summarize key modules"
cargo run -q -p rustcode -- --json run "hello"
```

## Tool Runtime Commands
```bash
cargo run -q -p rustcode -- list .
cargo run -q -p rustcode -- read path/to/file
cargo run -q -p rustcode -- write path/to/file "contents"
cargo run -q -p rustcode -- edit path/to/file "from" "to"
cargo run -q -p rustcode -- exec echo hi
cargo run -q -p rustcode -- models
cargo run -q -p rustcode -- models openrouter
```

## Provider Setup
```bash
RUSTCODE_ALLOW_NETWORK=1 cargo run -q -p rustcode -- --llm-provider openrouter --llm-base-url https://openrouter.ai/api/v1 --llm-api-key-env OPENROUTER_API_KEY run "hello"
```

Tool-calling agent runs require an OpenAI-compatible provider that supports `tools`:

```bash
RUSTCODE_ALLOW_NETWORK=1 cargo run -q -p rustcode -- --llm-provider openrouter --llm-base-url https://openrouter.ai/api/v1 --llm-api-key-env OPENROUTER_API_KEY agent "list files then read Cargo.toml"
```

Safety defaults:
- `agent` runs are read-only by default. To allow mutation tools:
  - `--allow-write` for creating/overwriting files
  - `--allow-edit` for in-place edits
- Overwriting an existing file requires reading it first (agent refuses blind overwrites).

## Auth Commands
```bash
cargo run -q -p rustcode -- auth methods openrouter
cargo run -q -p rustcode -- auth set-key openrouter --from-env OPENROUTER_API_KEY
cargo run -q -p rustcode -- auth status openrouter
cargo run -q -p rustcode -- auth remove openrouter
```

## TUI Path
```bash
cargo run -q -p rustcode -- tui
```

## Benchmarking
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark prompt"
./scripts/benchmark.sh drift 20 5
./scripts/benchmark_record.sh benchmarks/latest.json
./scripts/provider_matrix.sh > /tmp/provider-matrix.tsv
```

## Operational Notes
- Project-local config (`rustcode.toml`) is trust-gated.
- Use `--trust-project-config` only for trusted repositories.
- JSON mode includes `schema_version` in every event envelope.
- Provider setup details: `docs/PROVIDERS.md`.
- Provider coverage audit details: `docs/PROVIDER_COVERAGE.md`.
