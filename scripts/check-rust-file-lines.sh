#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly new_file_limit=1500

if [[ -n "${OOMU_NEW_SOURCE_LINE_LIMIT:-}" ]]; then
  echo "source-line-ratchet: OOMU_NEW_SOURCE_LINE_LIMIT is not configurable" >&2
  exit 1
fi

# HEADROOM remains a hard failure in the parser-backed implementation.
exec node "$repo_root/scripts/check-source-quality.mjs" "$@"
