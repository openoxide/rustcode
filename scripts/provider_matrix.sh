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

summary_json="$tmp_dir/models_summary.json"
summary_err="$tmp_dir/models_summary.err"

RUSTCODE_USER_CONFIG="$tmp_dir/rustcode.toml" \
RUSTCODE_TRUST_PROJECT=1 \
RUSTCODE_MODELS_PATH="$MODELS_PATH" \
cargo run -q -p rustcode -- --json models >"$summary_json" 2>"$summary_err" || true

printf "provider\tstatus\tdetail\n"
node - "$MODELS_PATH" "$summary_json" "$summary_err" <<'NODE'
const fs = require('fs');

const modelsPath = process.argv[2];
const summaryPath = process.argv[3];
const summaryErrPath = process.argv[4];

function safeRead(path) {
  try {
    return fs.readFileSync(path, 'utf8');
  } catch {
    return '';
  }
}

const modelsRaw = safeRead(modelsPath);
if (!modelsRaw.trim()) {
  console.error(`models file is empty: ${modelsPath}`);
  process.exit(1);
}

let providers;
try {
  const index = JSON.parse(modelsRaw);
  providers = Object.keys(index).sort();
} catch (err) {
  console.error(`failed to parse models file ${modelsPath}: ${err}`);
  process.exit(1);
}

const summaryRaw = safeRead(summaryPath);
if (!summaryRaw.trim()) {
  const err = safeRead(summaryErrPath).replace(/\s+/g, ' ').trim();
  const detail = err ? err.slice(0, 180) : 'missing-summary-json';
  for (const provider of providers) {
    console.log(`${provider}\terror\t${detail}`);
  }
  process.exit(0);
}

let summary;
try {
  summary = JSON.parse(summaryRaw);
} catch (err) {
  const detail = String(summaryRaw).replace(/\s+/g, ' ').trim().slice(0, 180);
  for (const provider of providers) {
    console.log(`${provider}\terror\tinvalid-json:${detail}`);
  }
  process.exit(0);
}

const rows = Array.isArray(summary.providers) ? summary.providers : [];
const rowMap = new Map(rows.map((row) => [row.id, row]));

for (const provider of providers) {
  const row = rowMap.get(provider);
  if (!row) {
    console.log(`${provider}\terror\tmissing-from-summary`);
    continue;
  }

  const missing = Array.isArray(row.missing) ? row.missing : [];
  if (missing.includes('base_url')) {
    console.log(`${provider}\tneeds_base_url\tmissing-default-endpoint`);
    continue;
  }
  if (missing.includes('api_key')) {
    console.log(`${provider}\tneeds_api_key\tmissing-secret`);
    continue;
  }

  console.log(`${provider}\tready\tinit-ok`);
}
NODE
