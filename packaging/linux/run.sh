#!/usr/bin/env bash
# Start the bridge from its own folder (config.yaml and bridge.log live next to the binary).
cd "$(dirname "$0")"
exec ./karlsen-stratum-bridge --config ./config.yaml "$@"
