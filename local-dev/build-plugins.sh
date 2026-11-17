#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
# Use the developer's Yarn without Corepack adding a toolchain pin to package.json.
export COREPACK_ENABLE_AUTO_PIN=0
yarn install
yarn export:plugins
