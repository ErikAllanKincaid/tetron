#!/usr/bin/env bash
# Destroy the 2 instances in tests/e2e/connect/.servers and remove the file.
# Manual — run only when you're done inspecting the servers.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SERVERS="$DIR/.servers"

[[ -f "$SERVERS" ]] || { echo "No $SERVERS — nothing to tear down."; exit 0; }

while read -r id ip label zone; do
  [[ -n "$id" ]] || continue
  echo ">> terminating $label  id=$id  ip=$ip  zone=$zone"
  scw instance server terminate "$id" zone="$zone" with-ip=true with-block=true || \
    echo "   (terminate failed for $id — check 'scw instance server list')"
done < "$SERVERS"

rm -f "$SERVERS"
echo
echo "Removed $SERVERS. Verify with: scw instance server list"
