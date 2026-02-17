# Credential Requirements

This document is the single source of truth for runtime credentials required by `rustcode` auth flows.

## Update Rule

- Any auth/login feature change must update this file in the same commit.
- Before attempting live OAuth/API validation that requires user-specific secrets, ask the user for the missing values explicitly.
- Never commit user-provided secrets, tokens, or API keys.

## Provider Matrix

| Provider | Method | Required | Optional | Notes |
| --- | --- | --- | --- | --- |
| `openai` | `oauth_device_code` | none | none | Uses built-in client ID from opencode/codex-compatible flow. |
| `openai` | `oauth_browser` | none | none | Uses built-in client ID; callback port set by `--oauth-port`. |
| `openai` | `api_key` | `--from-env <ENV_VAR>` value | none | Stored as API key credential. |
| `github-copilot` | `oauth_device_code` | none | `--domain` for enterprise variants | Uses built-in Copilot client ID `Ov23li8tweQw6odWQebz`. |
| `github-copilot-enterprise` | `oauth_device_code` | none | `--domain` recommended | Domain should target enterprise host. |
| `github-copilot` / `github-copilot-enterprise` | `api_key` | `--from-env <ENV_VAR>` value | none | Manual token mode. |
| `gitlab` | `oauth_browser` | none for `gitlab.com`; `GITLAB_OAUTH_CLIENT_ID` for self-hosted instances | `GITLAB_OAUTH_CLIENT_SECRET`, `GITLAB_INSTANCE_URL` | Uses PKCE + localhost callback + `/oauth/token`; defaults to bundled OpenCode-compatible client ID on `gitlab.com`. |
| `gitlab` | `api_key` | `--from-env <ENV_VAR>` value | none | Personal Access Token mode. |
| `mcp:<name>` | `oauth_browser` | one of: `--client-id`, configured `oauth.client_id`, or `RUSTCODE_MCP_OAUTH_CLIENT_ID`; OAuth-capable MCP metadata | `--client-secret-env <ENV_VAR>`, configured `oauth.client_secret_env`, `RUSTCODE_MCP_OAUTH_CLIENT_SECRET`, `--oauth-port` | Uses MCP-discovered OAuth endpoints + PKCE + localhost callback; stores OAuth token in auth store under `mcp:<name>`. |
| `mcp:<name>` | `api_key` import | `--from-env <ENV_VAR>` value | none | Manual token import fallback for MCP servers without browser OAuth support. |
| any provider | `api_key` | `--from-env <ENV_VAR>` value | none | Generic fallback. |

## Live Validation Inputs (Ask User)

Ask the user for these before running live external auth validation:

- `GITLAB_OAUTH_CLIENT_ID` for self-hosted GitLab browser OAuth.
- `GITLAB_OAUTH_CLIENT_SECRET` when their app configuration requires it.
- MCP OAuth app registration values when the server requires explicit client credentials:
  - client ID (`--client-id` or `oauth.client_id`)
  - client secret env var name (`--client-secret-env` or `oauth.client_secret_env`)
- Provider-specific API keys when validating `--from-env` auth paths against real endpoints.

## References

- `https://opencode.ai/docs`
- `https://developers.openai.com/codex/`
- `../opencode/packages/opencode/src/plugin/codex.ts`
- `../opencode/packages/opencode/src/plugin/copilot.ts`
- `../opencode/packages/web/src/content/docs/providers.mdx`
