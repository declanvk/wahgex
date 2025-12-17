#!/usr/bin/env bash

set -o errexit # make script exit when a command fails
set -o nounset # make script exit when using undeclared variables
set -o pipefail # make script exit when command fails in a pipe

if [ "$#" -ne 2 ]; then
    >&2 echo "Illegal number of parameters [$#]"
    >&2 echo "usage: bench-against.sh <previous-commit> <benchmark name>"
    exit 1
fi

PREVIOUS_REF="${1}"
BENCHMARK_NAME="${2}"

git switch --quiet --detach "${PREVIOUS_REF}"

BASELINE_NAME="$(git rev-parse --short HEAD)"

# Create the baseline benchmark and don't output the summary
cargo bench --quiet --bench "${BENCHMARK_NAME}" -- --save-baseline="${BASELINE_NAME}" > /dev/null

# Using '-' will switch back to the previous branch or git checkout
git switch --quiet -

# Run the benchmark again with comparison to baseline
cargo bench --quiet --bench "${BENCHMARK_NAME}" -- --baseline="${BASELINE_NAME}"