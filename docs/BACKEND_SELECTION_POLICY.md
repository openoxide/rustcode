# Backend Selection Policy

`rustcode` now exposes a weighted backend-selection policy in config so teams can tune backend preference explicitly instead of relying on ad-hoc defaults.

## Why

We compare backend/tooling ecosystems across these factors:

- provider/model flexibility
- automation/workflow depth
- open-source extensibility
- LSP/code-intelligence integration
- privacy posture
- subscription friction

The policy lets you encode those tradeoffs in config and keep them versioned with the project.

## Config

```toml
[policy.backend_selection]
provider_agnostic_weight = 2
automation_skills_weight = 3
open_source_weight = 1
lsp_support_weight = 1
privacy_weight = 2
subscription_penalty = 1
```

## Semantics

- Higher positive weights increase preference for backends with that capability.
- `subscription_penalty` is a positive penalty term (must be non-negative).
- All fields are non-negative integers.
- If omitted, defaults are applied.

Current defaults:

- `provider_agnostic_weight = 2`
- `automation_skills_weight = 3`
- `open_source_weight = 1`
- `lsp_support_weight = 1`
- `privacy_weight = 2`
- `subscription_penalty = 1`

## Validation

Negative weights are rejected during config loading with a field-specific validation error.
