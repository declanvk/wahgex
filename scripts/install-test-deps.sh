#!/usr/bin/env bash

set -o errexit # make script exit when a command fails
set -o nounset # make script exit when using undeclared variables
set -o pipefail # make script exit when command fails in a pipe
set -o xtrace # print a trace of all commands executed by script

version=$(cargo metadata --format-version=1 |\
    jq '.packages[] | select(.name == "gungraun").version' |\
    tr -d '"'
)

cargo install gungraun-runner --version $version

sudo apt install -y valgrind