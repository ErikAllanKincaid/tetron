#!/usr/bin/env bash
# Provision 2 Scaleway instances for the rayfish throughput/latency benchmark.
# Creates servers, waits for them to boot, resolves public IPs, and writes
# `id ip label zone` pairs to tests/bench/.servers. Re-running is a no-op while
# .servers exists (delete it to re-provision). Servers are LEFT RUNNING; use
# teardown.sh to destroy them.
#
# Both servers are placed in the SAME zone so the direct (public-IP) path is
# fast and low-latency — the benchmark then isolates the overhead rayfish adds
# on top of the raw network rather than measuring inter-region distance.
set -euo pipefail

ZONE="${ZONE:-fr-par-1}"
TYPE="${TYPE:-DEV1-S}"
IMAGE="${IMAGE:-ubuntu_jammy}"

DIR="$(cd "$(dirname "$0")" && pwd)"
SERVERS="$DIR/.servers"

NAMES=(rayfish-bench-a rayfish-bench-b)
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
echo "Next:  tests/bench/run.sh"
