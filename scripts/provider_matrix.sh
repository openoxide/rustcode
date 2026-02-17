#!/usr/bin/env bash
set -euo pipefail

MODELS_PATH="${RUSTCODE_MODELS_PATH:-$HOME/.cache/opencode/models.json}"
if [[ ! -f "$MODELS_PATH" ]]; then
  echo "models file not found: $MODELS_PATH" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "node is required for provider_matrix.sh" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

cat >"$tmp_dir/rustcode.toml" <<'EOF'
allow_network = true
EOF

providers="$(node -e 'const fs=require("fs"); const p=process.argv[1]; const data=JSON.parse(fs.readFileSync(p,"utf8")); console.log(Object.keys(data).sort().join("\n"));' "$MODELS_PATH")"

printf "provider\tstatus\tdetail\n"
while IFS= read -r provider; do
  [[ -z "$provider" ]] && continue
  output="$(
    RUSTCODE_USER_CONFIG="$tmp_dir/rustcode.toml" \
    RUSTCODE_TRUST_PROJECT=1 \
    cargo run -q -p rustcode-cli -- --llm-provider "$provider" --model "$provider/smoke-model" version 2>&1 || true
  )"

  if [[ "$output" == *"rustcode 0.1.0"* ]]; then
    printf "%s\tready\tinit-ok\n" "$provider"
    continue
  fi
  if [[ "$output" == *"requires an LLM base URL"* ]]; then
    printf "%s\tneeds_base_url\tmissing-default-endpoint\n" "$provider"
    continue
  fi
  if [[ "$output" == *"requires an API key"* || "$output" == *"requires ANTHROPIC_API_KEY"* ]]; then
    printf "%s\tneeds_api_key\tmissing-secret\n" "$provider"
    continue
  fi
  if [[ "$output" == *"network access is disabled"* ]]; then
    printf "%s\terror\tnetwork-gate\n" "$provider"
    continue
  fi

  detail="$(printf "%s" "$output" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g' | cut -c1-180)"
  printf "%s\terror\t%s\n" "$provider" "$detail"
done <<<"$providers"
