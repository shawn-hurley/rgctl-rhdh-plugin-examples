#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
# Refuse concurrent legacy writers; do not stop unrelated containers or the VM.
for volume in dependency-impact-dev_analysis-data application-graph-local_application-graphs; do
  writers=$(podman ps --filter "volume=$volume" --format '{{.Names}}')
  for writer in $writers; do
    case "$writer" in
      rgctl-services-rgctl-service-1|rgctl-services_rgctl-service_1) ;;
      *) echo "Stop the existing store writer before starting the shared service: $writer" >&2; exit 1 ;;
    esac
  done
done
# Idempotent on existing stores.
for volume in dependency-impact-dev_analysis-data application-graph-local_application-graphs; do
  if ! podman volume exists "$volume"; then
    podman volume create "$volume"
  fi
done
podman compose -f local-dev/service-compose.yaml "$@" up -d --no-build
