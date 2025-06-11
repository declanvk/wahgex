#!/usr/bin/env bash
set -euxo pipefail

SCRIPT_FOLDER="$(dirname "$(readlink -f "$0")")"
DIST_FOLDER="${SCRIPT_FOLDER}/dist"

# Build the WASM module
wasm-pack build "${SCRIPT_FOLDER}/playground" --target web --out-dir "${DIST_FOLDER}/playground"

# Copy static files
rsync --recursive "${SCRIPT_FOLDER}/static/" "${DIST_FOLDER}"

# Copy rustdoc documentation to dist/rustdoc
rsync --recursive "${SCRIPT_FOLDER}/../target/doc/" "${DIST_FOLDER}/docs"
