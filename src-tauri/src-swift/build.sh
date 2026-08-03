#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
TAURI_DIR="${SCRIPT_DIR:h}"
OUTPUT_DIR="${TAURI_DIR}/binaries"
OUTPUT="${OUTPUT_DIR}/oomu-speech-bridge-aarch64-apple-darwin"
MODULE_CACHE="${TAURI_DIR}/target/swift-module-cache"
export MACOSX_DEPLOYMENT_TARGET=14.0

mkdir -p "${OUTPUT_DIR}" "${MODULE_CACHE}"
xcrun swiftc \
  -O \
  -whole-module-optimization \
  -module-cache-path "${MODULE_CACHE}" \
  -target arm64-apple-macos14.0 \
  "${SCRIPT_DIR}/main.swift" \
  -framework AVFoundation \
  -framework Speech \
  -Xlinker -sectcreate \
  -Xlinker __TEXT \
  -Xlinker __info_plist \
  -Xlinker "${SCRIPT_DIR}/SpeechBridgeInfo.plist" \
  -o "${OUTPUT}"
chmod 755 "${OUTPUT}"

echo "Prepared ${OUTPUT}"
