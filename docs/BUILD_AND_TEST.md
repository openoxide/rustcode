# Build and test

## Requirements

- Rust toolchain from `rust-toolchain.toml`.
- Standard cargo workflow (`cargo` on PATH).

## Build

```bash
cargo check --workspace
```

## Test

```bash
cargo test --workspace
```

To run the CLI integration suite only:

```bash
cargo test -p rustcode --test integration_cli
```

## Binary

Run the CLI with cargo:

```bash
cargo run -p rustcode -- <command>
```

Example:

```bash
cargo run -p rustcode -- version
```
