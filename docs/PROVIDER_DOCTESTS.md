# Provider Doctests

This document contains provider-oriented CLI examples that are executed as part of the test suite.

Constraints:
- No network access and no secrets.
- Commands must use the `rustcode` binary directly (not `cargo run`).
- Commands in doctest blocks must be `rustcode ...` only (no pipes, no other shell commands).

Run locally:

```bash
cargo test -p rustcode --test docs_provider_doctest
```

## Models And Diagnostics (Deterministic Fixture)

```bash rustcode-doctest
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --help
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --version

RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models openai
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models openrouter
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models github-copilot
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models google

RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models github-copilot-enterprise

RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode models
RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode models openai
```

## Models Index Missing (Builtin Fallback)

```bash rustcode-doctest
RUSTCODE_MODELS_PATH=/this/path/does/not/exist/models.json rustcode --json models
RUSTCODE_MODELS_PATH=/this/path/does/not/exist/models.json rustcode --json models openai
RUSTCODE_MODELS_PATH=/this/path/does/not/exist/models.json rustcode --json models anthropic
RUSTCODE_MODELS_PATH=/this/path/does/not/exist/models.json rustcode --json models vercel
```
