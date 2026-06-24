#!/usr/bin/env bash
# Destroy the 2 benchmark instances listed in tests/bench/.servers and remove
# the file. Manual — run only when you're done collecting numbers.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SERVERS="$DIR/.servers"

[[ -f "$SERVERS" ]] || { echo "No $SERVERS — nothing to tear down."; exit 0; }

while read -r id ip label zone; do
  [[ -n "$id" ]] || continue
  echo ">> terminating $label  id=$id  ip=$ip  zone=$zone"
  # `terminate` deletes the server and frees its attached local volume + IP.
  scw instance server terminate "$id" zone="$zone" with-ip=true with-block=true || \
    echo "   (terminate failed for $id — check 'scw instance server list')"
done < "$SERVERS"

rm -f "$SERVERS"
echo
echo "Removed $SERVERS. Verify with: scw instance server list"
