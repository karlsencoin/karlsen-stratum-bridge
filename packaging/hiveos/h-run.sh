#!/usr/bin/env bash
# Started by HiveOS. Runs the bridge from its own folder so config.yaml and bridge.log
# are found next to the binary; console output also goes to the HiveOS miner log.

cd "$(dirname "$0")"
. h-manifest.conf

mkdir -p "$(dirname "$CUSTOM_LOG_BASENAME")"
mapfile -t args < <( [[ -f args.txt ]] && grep -v '^$' args.txt )

./karlsen-stratum-bridge --config ./config.yaml "${args[@]}" 2>&1 | tee --append "${CUSTOM_LOG_BASENAME}.log"
