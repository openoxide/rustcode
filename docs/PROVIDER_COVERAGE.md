# Provider Coverage Audit

Date: 2026-02-17

## Command

```bash
./scripts/provider_matrix.sh > /tmp/provider-matrix.tsv
```

## Source of Provider IDs

- `~/.cache/opencode/models.json`
- 91 provider IDs discovered

## Result Summary

- `needs_api_key`: 81
- `needs_base_url`: 10
- `error`: 0

This means every provider ID in the opencode models index is recognized by `rustcode` provider diagnostics. None fail with unknown/unsupported IDs during diagnostics.

## Interpretation

- `needs_api_key` means protocol + endpoint shape are resolved and runtime is blocked only on credential availability.
- `needs_base_url` means an endpoint cannot be inferred and must be configured via `[llm].base_url` (or `--llm-base-url`).

## Next Gaps

1. Add first-class default endpoints for more providers currently in `needs_base_url`.
2. Add provider login/OAuth adapters for providers that do not rely on static API keys.
3. Add live integration checks (opt-in) for at least one provider per protocol family.
