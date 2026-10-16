#!/usr/bin/env bash
# Called by HiveOS before h-run.sh. The bridge reads config.yaml from its own folder;
# anything typed in the flight sheet's "Extra config arguments" field is passed as
# command line flags (e.g. --solo-mining true --karlsend-address 127.0.0.1:42110).
# If "Pool URL" is set to a node address (host:port), it is used as karlsend_address.

cd "$(dirname "$0")"
. h-manifest.conf

args=()
if [[ -n "$CUSTOM_URL" ]]; then
  node="${CUSTOM_URL#*://}"
  args+=(--karlsend-address "$node")
fi

printf '%s\n' "${args[@]}" > args.txt
[[ -n "$CUSTOM_USER_CONFIG" ]] && printf '%s\n' $CUSTOM_USER_CONFIG >> args.txt
exit 0
