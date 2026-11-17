#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
rgctl_source=$(cd rgctl-source && pwd -P)
rgctl_revision=$(git -C "$rgctl_source" rev-parse HEAD)
podman build -f analysis-server/Containerfile --target application-graph \
  --build-context "rgctl-source=$rgctl_source" \
  --build-arg "RGCTL_REVISION=$rgctl_revision" \
  -t localhost/application-graph:dev analysis-server
