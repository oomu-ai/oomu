#!/usr/bin/env bash
set -euo pipefail

echo "OOMU RELEASE ERROR: Linux distributables are disabled because no supported WhatsApp sidecar and no signing/notarization evidence contract are available for Linux." >&2
echo "Use 'npm run build:prod' on the canonical macOS release runner." >&2
exit 1
