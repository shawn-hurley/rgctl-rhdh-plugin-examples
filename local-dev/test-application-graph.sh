#!/bin/sh
set -eu
export COREPACK_ENABLE_AUTO_PIN=0
cd "$(dirname "$0")/.."
mkdir -p local-dev/.generated/application-graph-tests
yarn tsc --strict --skipLibCheck --esModuleInterop --module commonjs --target es2022 \
  --outDir local-dev/.generated/application-graph-tests \
  plugins/application-graph-backend/src/catalog.ts \
  plugins/application-graph-backend/src/client.ts \
  plugins/application-graph-backend/src/router.ts
node --test plugins/application-graph-backend/test/router.test.cjs
