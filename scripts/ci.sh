#!/usr/bin/env bash

set -o errexit # make script exit when a command fails
set -o nounset # make script exit when using undeclared variables
set -o pipefail # make script exit when command fails in a pipe
set -o xtrace # print a trace of all commands executed by script

if [ "$#" -ne 1 ]; then
    >&2 echo "Illegal number of parameters [$#]"
    >&2 echo "usage: ci.sh <toolchain>"
    exit 1
fi

TOOLCHAIN="${1}"
TOOLCHAIN_ARG="+${TOOLCHAIN}"

cargo "${TOOLCHAIN_ARG}" build  --all-targets

# --all-targets does not include the doctests
cargo "${TOOLCHAIN_ARG}" test   --all-targets
cargo "${TOOLCHAIN_ARG}" test   --doc

cargo "${TOOLCHAIN_ARG}" clippy --all-targets
cargo "${TOOLCHAIN_ARG}" doc    --no-deps --document-private-items

if [ "${TOOLCHAIN}" = "nightly" ]; then
    cargo "${TOOLCHAIN_ARG}" fmt -- --check
fi
