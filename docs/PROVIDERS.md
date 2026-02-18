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
   - `enabled_providers` (optional allow-list)
   - `disabled_providers` (optional deny-list; wins over allow-list)

If `llm.provider` is `null`, `rustcode` derives provider from `model` when it is in `provider/model` format.

## Current Protocols

- `null` provider: offline deterministic response.
- OpenAI-compatible: `POST /v1/chat/completions`
- Anthropic: `POST /v1/messages`
- Vercel AI Gateway: `POST /language-model` (AI SDK v2 protocol headers + SSE deltas)
- Google Gemini: `POST /v1beta/models/<model>:generateContent` and `:streamGenerateContent?alt=sse`

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
  - Vercel AI Gateway (`vercel`)
  - Google Gemini (`google`)
  - Vercel v0 (`v0`)
  - OpenCode Zen (`opencode`)

Additional IDs from opencode provider references are accepted through generic OpenAI-compatible mode when `base_url` is provided.

`rustcode` also reads provider metadata from `RUSTCODE_MODELS_PATH` (or `~/.cache/opencode/models.json`) to inherit provider env-key names and advertised endpoints where available.

## Provider Filtering

`rustcode` supports OpenCode-style provider filtering:

- `enabled_providers = ["openrouter", "openai"]`: only these providers are considered available.
- `disabled_providers = ["openai"]`: always disables a provider (even if it is in `enabled_providers`).

## Examples

```toml
allow_network = true
model = "openrouter/openai/gpt-5-mini"

[llm]
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
```

You can also enable network access for a single run via env (no config file edit):

```bash
RUSTCODE_ALLOW_NETWORK=1 rustcode models
```

For any non-`null` provider, you must either:

- set `allow_network = true` in config, or
- set `RUSTCODE_ALLOW_NETWORK=1` for the command.

```bash
RUSTCODE_ALLOW_NETWORK=1 OPENROUTER_API_KEY=... rustcode run "hello"
```

Or store once in the local auth store:

```bash
OPENROUTER_API_KEY=... rustcode auth set-key openrouter --from-env OPENROUTER_API_KEY
rustcode auth status openrouter
```

Vercel AI Gateway:

```bash
AI_GATEWAY_API_KEY=... rustcode auth set-key vercel --from-env AI_GATEWAY_API_KEY
RUSTCODE_ALLOW_NETWORK=1 rustcode run "hello via vercel gateway"
```

Vercel v0:

```bash
V0_API_KEY=... rustcode auth set-key v0 --from-env V0_API_KEY
RUSTCODE_ALLOW_NETWORK=1 rustcode run "hello via v0"
```

OpenCode Zen:

```bash
OPENCODE_API_KEY=... rustcode auth set-key opencode --from-env OPENCODE_API_KEY
RUSTCODE_ALLOW_NETWORK=1 rustcode run "hello via opencode zen"
```

Google Gemini:

```bash
GEMINI_API_KEY=... rustcode auth set-key google --from-env GEMINI_API_KEY
RUSTCODE_ALLOW_NETWORK=1 rustcode --llm-provider google --model google/gemini-2.5-flash run "hello via gemini"
```

GitHub Copilot enterprise base URL:

```bash
# Option A: explicit base URL
RUSTCODE_ALLOW_NETWORK=1 rustcode --llm-provider github-copilot-enterprise --llm-base-url "https://copilot-api.github.example.com" --model github-copilot/gpt-4o run "hello"

# Option B: env-derived base URL
export GITHUB_COPILOT_ENTERPRISE_DOMAIN=github.example.com
RUSTCODE_ALLOW_NETWORK=1 rustcode --llm-provider github-copilot-enterprise --model github-copilot/gpt-4o run "hello"
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
rustcode auth login gitlab --method oauth_browser
# self-hosted GitLab:
GITLAB_OAUTH_CLIENT_ID=... rustcode auth login gitlab --method oauth_browser --domain gitlab.example.com
```

`auth login` supports method negotiation:

- `oauth_device_code`: `openai`, `github-copilot`, `github-copilot-enterprise`
- `oauth_browser`: `gitlab`
- `api_key`: all providers via `--from-env`
