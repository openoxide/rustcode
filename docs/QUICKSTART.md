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
cargo run -q -p rustcode-cli -- --help
cargo run -q -p rustcode-cli -- run "hello"
cargo run -q -p rustcode-cli -- --json run "hello"
```

## Tool Runtime Commands
```bash
cargo run -q -p rustcode-cli -- list .
cargo run -q -p rustcode-cli -- read path/to/file
cargo run -q -p rustcode-cli -- write path/to/file "contents"
cargo run -q -p rustcode-cli -- edit path/to/file "from" "to"
cargo run -q -p rustcode-cli -- exec echo hi
```

## TUI Path
```bash
cargo run -q -p rustcode-cli -- tui
```

## Benchmarking
```bash
./scripts/benchmark.sh startup 5
./scripts/benchmark.sh memory "benchmark prompt"
./scripts/benchmark.sh drift 20 5
./scripts/benchmark_record.sh benchmarks/latest.json
```

## Operational Notes
- Project-local config (`rustcode.toml`) is trust-gated.
- Use `--trust-project-config` only for trusted repositories.
- JSON mode includes `schema_version` in every event envelope.
