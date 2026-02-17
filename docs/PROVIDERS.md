# Provider Configuration

`rustcode` now resolves LLM providers via config, env, or CLI overrides.

## Resolution Order

1. CLI flags
   - `--llm-provider`
   - `--llm-base-url`
   - `--llm-api-key-env`
2. Environment variables
   - `RUSTCODE_LLM_PROVIDER`
   - `RUSTCODE_LLM_BASE_URL`
   - `RUSTCODE_LLM_API_KEY_ENV`
3. Config files (`global` -> `user` -> trusted `project`)
   - `[llm] provider`
   - `[llm] base_url` (alias: `baseURL`)
   - `[llm] api_key_env` (alias: `apiKeyEnv`)

If `llm.provider` is `null`, `rustcode` derives provider from `model` when it is in `provider/model` format.

## Current Protocols

- `null` provider: offline deterministic response.
- OpenAI-compatible: `POST /v1/chat/completions`
- Anthropic: `POST /v1/messages`

## Provider Presets

Preset defaults include:

- OpenAI
- OpenRouter
- Groq
- xAI
- Mistral
- Together
- Perplexity
- DeepInfra
- Cerebras
- Ollama
- Azure (`azure`, `azure-cognitive-services`)
- GitHub Copilot (`github-copilot`, `github-copilot-enterprise`)
- Cloudflare (`cloudflare-workers-ai`, `cloudflare-ai-gateway`)

Additional IDs from opencode provider references are accepted through generic OpenAI-compatible mode when `base_url` is provided.

`rustcode` also reads provider metadata from `RUSTCODE_MODELS_PATH` (or `~/.cache/opencode/models.json`) to inherit provider env-key names and advertised endpoints where available.

## Examples

```toml
allow_network = true
model = "openrouter/openai/gpt-5-mini"

[llm]
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
```

```bash
OPENROUTER_API_KEY=... rustcode run "hello"
```

Or store once in the local auth store:

```bash
OPENROUTER_API_KEY=... rustcode auth set-key openrouter --from-env OPENROUTER_API_KEY
rustcode auth status openrouter
```

Ollama local example:

```toml
allow_network = true
model = "ollama/qwen2.5-coder:7b"

[llm]
provider = "ollama"
```

```bash
rustcode run "summarize this directory"
```

For OAuth-capable providers:

```bash
rustcode auth login openai
rustcode auth login openai --method oauth_browser --oauth-port 1455
rustcode auth login github-copilot
GITLAB_OAUTH_CLIENT_ID=... rustcode auth login gitlab --method oauth_browser
```

`auth login` supports method negotiation:

- `oauth_device_code`: `openai`, `github-copilot`, `github-copilot-enterprise`
- `oauth_browser`: `gitlab`
- `api_key`: all providers via `--from-env`
