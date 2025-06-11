#!/usr/bin/env bash
set -euxo pipefail

SCRIPT_FOLDER="$(dirname "$(readlink -f "$0")")"
WEB_FOLDER="${SCRIPT_FOLDER}/../web"
DIST_FOLDER="${WEB_FOLDER}/dist"

# Build the WASM module
wasm-pack build "${WEB_FOLDER}/playground" --target web --out-dir "${DIST_FOLDER}/playground"

# Copy static files
rsync --recursive "${WEB_FOLDER}/static/" "${DIST_FOLDER}"

# Build and copy rustdoc documentation to dist/rustdoc
cargo doc --workspace --no-deps --document-private-items
rsync --recursive "${SCRIPT_FOLDER}/../target/doc/" "${DIST_FOLDER}/docs"
