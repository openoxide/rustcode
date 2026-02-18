#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  scripts/build_release_bundle.sh <version> [output_dir]

Examples:
  scripts/build_release_bundle.sh 0.1.0 dist
  scripts/build_release_bundle.sh v0.1.0 /tmp/rustcode-dist
USAGE
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

version_raw="$1"
output_dir="${2:-dist}"
version="${version_raw#v}"
tag="v$version"

cd "$(dirname "$0")/.."

if [[ ! -f benchmarks/latest.json ]]; then
  echo "missing benchmark artifact: benchmarks/latest.json" >&2
  exit 1
fi

if [[ ! -f docs/RELEASE_NOTES_DRAFT.md ]]; then
  echo "missing release notes draft: docs/RELEASE_NOTES_DRAFT.md" >&2
  exit 1
fi

cargo build --release -p rustcode

target_triple="$(rustc -vV | awk '/^host:/ { print $2 }')"
if [[ -z "$target_triple" ]]; then
  echo "failed to determine rustc host triple" >&2
  exit 1
fi

bundle_name="rustcode-${tag}-${target_triple}"
bundle_root="$output_dir/$bundle_name"
mkdir -p "$bundle_root"

cp "target/release/rustcode" "$bundle_root/"
cp "benchmarks/latest.json" "$bundle_root/"
cp "docs/RELEASE_NOTES_DRAFT.md" "$bundle_root/"

archive="$output_dir/${bundle_name}.tar.gz"
tar -C "$output_dir" -czf "$archive" "$bundle_name"

checksum_file="${archive}.sha256"
rm -f "${checksum_file}.tmp"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$archive" > "$checksum_file"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$archive" > "$checksum_file"
else
  echo "no checksum tool available (sha256sum/shasum)" >&2
  exit 1
fi

echo "bundle_archive=$archive"
echo "bundle_checksum=$checksum_file"
