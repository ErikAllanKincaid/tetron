#!/usr/bin/env bash
# Provision 2 Scaleway DEV1-S instances for the `ray connect` (direct 2-peer)
# e2e test. Creates servers, waits for boot, resolves public IPs, and writes
# `id ip label zone` lines to tests/e2e/connect/.servers. Re-running is a no-op
# while .servers exists (delete it to re-provision). Servers are LEFT RUNNING;
# use teardown.sh to destroy them.
set -euo pipefail

ZONE="${ZONE:-fr-par-1}"
TYPE="${TYPE:-DEV1-S}"
IMAGE="${IMAGE:-ubuntu_jammy}"

DIR="$(cd "$(dirname "$0")" && pwd)"
SERVERS="$DIR/.servers"

NAMES=(rayfish-connect-a rayfish-connect-b)
LABELS=(srv-a srv-b)

if [[ -f "$SERVERS" ]]; then
  echo "Found existing $SERVERS — skipping provisioning."
  echo "(delete it to provision a fresh set)"
  echo
  cat "$SERVERS"
  exit 0
fi

command -v scw >/dev/null || { echo "scw not found"; exit 1; }
command -v jq  >/dev/null || { echo "jq not found";  exit 1; }

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

for i in 0 1; do
  name="${NAMES[$i]}"
  label="${LABELS[$i]}"
  echo ">> creating $name ($label)  [$TYPE $IMAGE $ZONE]"
  json="$(scw instance server create \
            type="$TYPE" zone="$ZONE" image="$IMAGE" \
            name="$name" ip=new -w -o json)"
  id="$(echo "$json"  | jq -r '.id')"
  ip="$(echo "$json"  | jq -r '(.public_ip.address // (.public_ips[0].address) // empty)')"
  if [[ -z "$ip" || "$ip" == "null" ]]; then
    ip="$(scw instance server get "$id" zone="$ZONE" -o json \
            | jq -r '(.public_ip.address // (.public_ips[0].address))')"
  fi
  echo "   id=$id  ip=$ip"
  echo "$id $ip $label $ZONE" >> "$tmp"
done

mv "$tmp" "$SERVERS"
trap - EXIT
echo
echo "Wrote $SERVERS:"
cat "$SERVERS"
echo
echo "Next:  tests/e2e/connect/run.sh"
