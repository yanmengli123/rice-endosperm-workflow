#!/usr/bin/env bash
# Refresh the bundled models.dev catalog snapshot used as the offline build
# fallback. Run with network access (e.g. before a release), then commit the
# updated src-tauri/model_catalog.snapshot.json.
set -euo pipefail
cd "$(dirname "$0")/.."

# The snapshot is the build script's declared rerun-if-changed input, so
# touching it forces a re-fetch even when no source changed.
touch src-tauri/model_catalog.snapshot.json
cargo build -p wisp-tauri
latest=$(ls -t target/debug/build/wisp-tauri-*/out/model_catalog.json 2>/dev/null | head -1)
if [[ -z "$latest" ]]; then
    echo "no distilled catalog found under target/debug/build" >&2
    exit 1
fi
cp "$latest" src-tauri/model_catalog.snapshot.json
echo "snapshot updated from $latest"
