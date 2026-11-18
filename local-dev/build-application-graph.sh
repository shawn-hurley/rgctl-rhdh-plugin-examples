#!/bin/sh
# Compatibility alias: both workflows use the same image.
set -eu
exec sh "$(dirname "$0")/build-service.sh" "$@"
