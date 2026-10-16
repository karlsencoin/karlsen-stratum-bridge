#!/usr/bin/env bash
# HiveOS stats hook. The bridge does not mine itself, so it reports no hashrate;
# HiveOS only shows it as running with its version and uptime.

# HiveOS sources this file, so locate the manifest via BASH_SOURCE and do not change directory.
. "$(dirname "${BASH_SOURCE[0]}")/h-manifest.conf"

pid=$(pgrep -f "karlsen-stratum-bridge --config" | head -n1)
uptime=0
if [[ -n "$pid" ]]; then
  uptime=$(ps -o etimes= -p "$pid" | tr -d ' ')
fi

khs=0
stats=$(jq -nc --arg ver "$CUSTOM_VERSION" --argjson uptime "${uptime:-0}" \
  '{hs: [], hs_units: "khs", temp: [], fan: [], uptime: $uptime, ver: $ver, algo: "karlsenhashv2", ar: [0, 0]}')
