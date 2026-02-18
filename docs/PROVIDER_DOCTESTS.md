# Provider doctests

The following commands are executed by integration doctests. They are offline-safe and use repository-relative fixture paths.

```rustcode-doctest
$ RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models
$ RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json models openai
$ RUSTCODE_MODELS_PATH=docs/fixtures/models_min.json rustcode --json auth methods
$ rustcode --json mcp list
```
