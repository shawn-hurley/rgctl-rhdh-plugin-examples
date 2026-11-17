#!/bin/sh
set -eu
export COREPACK_ENABLE_AUTO_PIN=0
cd "$(dirname "$0")/.."
sh local-dev/test-application-graph.sh
yarn tsc --strict --skipLibCheck --esModuleInterop --module commonjs --target es2022 \
  --outDir local-dev/.generated/catalog-import-tests \
  plugins/catalog-backend-module-application-graph/src/enrichment.ts \
  plugins/catalog-backend-module-application-graph/src/processor.ts
node --test plugins/catalog-backend-module-application-graph/test/*.test.cjs
