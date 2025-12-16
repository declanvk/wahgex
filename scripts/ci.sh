#!/usr/bin/env bash

set -o errexit # make script exit when a command fails
set -o nounset # make script exit when using undeclared variables
set -o pipefail # make script exit when command fails in a pipe
set -o xtrace # print a trace of all commands executed by script

DEFAULT_TOOLCHAIN=$(rustup show active-toolchain | cut -f1 -d' ')
TOOLCHAIN="${1:-${DEFAULT_TOOLCHAIN}}"
TOOLCHAIN_ARG="+${TOOLCHAIN}"

# Build
# `cargo hack build --feature-powerset --print-command-list`
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features --features default,wasmi
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features --features compile
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features --features default
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features --features wasmi
cargo "${TOOLCHAIN_ARG}" build --manifest-path core/Cargo.toml --no-default-features --features compile,wasmi
cargo "${TOOLCHAIN_ARG}" build --manifest-path cli/Cargo.toml
cargo "${TOOLCHAIN_ARG}" build --manifest-path web/playground/Cargo.toml

# --all-targets does not include the doctests
# `cargo hack test --feature-powerset --print-command-list`
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features --features default,wasmi
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features --features compile
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features --features default
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features --features wasmi
cargo "${TOOLCHAIN_ARG}" test --manifest-path core/Cargo.toml --no-default-features --features compile,wasmi
cargo "${TOOLCHAIN_ARG}" test --manifest-path cli/Cargo.toml
cargo "${TOOLCHAIN_ARG}" test --manifest-path web/playground/Cargo.toml

cargo "${TOOLCHAIN_ARG}" test   --doc

# `cargo hack clippy --feature-powerset --print-command-list`
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features --features default,wasmi
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features --features compile
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features --features default
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features --features wasmi
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path core/Cargo.toml --no-default-features --features compile,wasmi
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path cli/Cargo.toml
cargo "${TOOLCHAIN_ARG}" clippy --manifest-path web/playground/Cargo.toml

cargo "${TOOLCHAIN_ARG}" doc    --no-deps --document-private-items

if [ "${TOOLCHAIN}" = "nightly" ]; then
    cargo "${TOOLCHAIN_ARG}" fmt -- --check
fi
