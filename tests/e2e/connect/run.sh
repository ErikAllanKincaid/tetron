#!/usr/bin/env bash
# `ray connect` (direct 2-peer connection) e2e test orchestrator.
#
# Topology:
#   srv-a  identity U   the initiator (`ray connect`)
#   srv-b  identity V   the recipient (`ray connections approve`)
#
# Proves the full friend-request flow over real hosts + the public pkarr DHT:
#   B publishes a contact id  ->  A `ray connect <id>`  ->  B sees + approves
#   ->  a 2-peer `[direct]` network forms  ->  A<->B reach each other (ping +
#   ray send) and the network is tagged direct with its room id hidden.
# Plus a negative case: connecting to an offline contact fails cleanly.
#
# Reads tests/e2e/connect/.servers (written by provision.sh). Does NOT modify
# infra. Re-runnable (resets rayfish state on each run unless KEEP_STATE=1).
set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../../.." && pwd)"
SERVERS="$DIR/.servers"
KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null \
          -o ConnectTimeout=10 -o LogLevel=ERROR -o BatchMode=yes)

[[ -f "$SERVERS" ]] || { echo "No $SERVERS — run tests/e2e/connect/provision.sh first"; exit 1; }

A=""; B=""
while read -r id ip label zone; do
  case "${label:-}" in
    srv-a) A="$ip" ;;
    srv-b) B="$ip" ;;
  esac
done < "$SERVERS"
[[ -n "$A" && -n "$B" ]] || { echo "missing srv-a/srv-b in $SERVERS"; exit 1; }

FAILS=0
pass(){ printf '  \033[32mPASS\033[0m %s\n' "$*"; }
fail(){ printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAILS=$((FAILS+1)); }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

# on <ip> <command-string> : run a shell command on a host as root.
# -n: never read stdin, so calling `on` inside a `while read` loop can't eat it.
on(){ local ip="$1"; shift; ssh -n "${SSH_OPTS[@]}" -i "$KEY" "root@$ip" "$*"; }
strip(){ sed -r 's/\x1B\[[0-9;]*[mGKH]//g'; }
own_ip(){ echo "$1" | grep -oE '100\.[0-9]+\.[0-9]+\.[0-9]+' | head -1; }

# ---------------------------------------------------------------------------
step "0. wait for SSH on both hosts"
wait_ssh(){ local ip="$1"; for _ in $(seq 1 60); do on "$ip" true 2>/dev/null && return 0; sleep 5; done; return 1; }
for pair in "srv-a $A" "srv-b $B"; do
  set -- $pair
  if wait_ssh "$2"; then pass "ssh $1 ($2)"; else fail "ssh $1 ($2) unreachable"; echo "aborting"; exit 1; fi
done
for h in "$A" "$B"; do ssh-keyscan -T 10 "$h" >> ~/.ssh/known_hosts 2>/dev/null || true; done

# ---------------------------------------------------------------------------
if [[ "${KEEP_STATE:-0}" != "1" ]]; then
  step "0b. reset rayfish state on both hosts (KEEP_STATE=1 to skip)"
  for h in "$A" "$B"; do
    on "$h" 'systemctl stop rayfish 2>/dev/null; rm -rf /root/.config/rayfish' && echo "   reset $h"
  done
fi

# ---------------------------------------------------------------------------
step "1. deploy ray to both hosts (cross build + rsync + ray up)"
for pair in "srv-a $A" "srv-b $B"; do
  set -- $pair
  echo ">> just deploy $2 ($1)"
  if ( cd "$ROOT" && just deploy "$2" ); then pass "deploy $1"; else fail "deploy $1"; echo "aborting"; exit 1; fi
done
# Ensure the VPN is active on both (TUN up + contact publisher running). After a
# `systemctl restart` the daemon boots inactive, so activate explicitly.
for h in "$A" "$B"; do on "$h" 'ray up' >/dev/null 2>&1 || true; done
sleep 5
for pair in "srv-a $A" "srv-b $B"; do
  set -- $pair
  if on "$2" 'ray status' >/dev/null 2>&1; then pass "daemon up on $1"; else fail "daemon not responding on $1"; fi
done

# ---------------------------------------------------------------------------
step "2. read contact ids"
A_CID="$(on "$A" 'ray contact id' | strip | head -1 | tr -d ' ')"
B_CID="$(on "$B" 'ray contact id' | strip | head -1 | tr -d ' ')"
echo "   A contact id: ${A_CID:0:16}…"
echo "   B contact id: ${B_CID:0:16}…"
[[ -n "$A_CID" && "${#A_CID}" -ge 20 ]] && pass "srv-a has a contact id" || fail "srv-a contact id missing/short"
[[ -n "$B_CID" && "${#B_CID}" -ge 20 ]] && pass "srv-b has a contact id" || fail "srv-b contact id missing/short"
# The contact id must also surface in `ray status`.
on "$A" 'ray status' | strip | grep -qi "${A_CID:0:16}" && pass "contact id shown in ray status" \
  || fail "contact id not shown in ray status"

# ---------------------------------------------------------------------------
step "3. srv-a requests a direct connection to srv-b"
# Give B's contact record time to propagate on the public pkarr DHT.
sleep 8
CONNECT_OUT=""
for _ in $(seq 1 6); do
  CONNECT_OUT="$(on "$A" "ray connect $B_CID --hostname dario" 2>&1 | strip)"
  echo "$CONNECT_OUT" | grep -qiE 'waiting for approval|connected' && break
  sleep 8
done
echo "$CONNECT_OUT" | sed 's/^/   a| /'
if echo "$CONNECT_OUT" | grep -qiE 'waiting for approval|connected'; then
  pass "srv-a connect request accepted (pending)"
else
  fail "srv-a connect request did not reach srv-b"
fi

# ---------------------------------------------------------------------------
step "4. srv-b sees the pending request and approves it"
REQ=""
for _ in $(seq 1 8); do
  REQ="$(on "$B" 'ray connections' 2>/dev/null | strip)"
  echo "$REQ" | grep -qiE "${A_CID:0:8}" && break
  sleep 3
done
echo "$REQ" | sed 's/^/   b| /'
if echo "$REQ" | grep -qiE "${A_CID:0:8}"; then
  pass "srv-b sees srv-a's request"
else
  fail "srv-b never saw srv-a's request"
fi
# Approve by srv-a's full contact id (the daemon matches it as a prefix), so we
# don't have to parse the short id out of the table.
APPROVE="$(on "$B" "ray connections approve $A_CID" 2>&1 | strip)"
echo "$APPROVE" | sed 's/^/   b| /'
echo "$APPROVE" | grep -qiE 'approved|already connected' && pass "srv-b approved the request" \
  || fail "srv-b approve failed"

# ---------------------------------------------------------------------------
step "5. wait for the 2-peer direct network to form on both sides"
converged=0
for _ in $(seq 1 18); do  # up to ~90s
  SA="$(on "$A" 'ray status' | strip)"; SB="$(on "$B" 'ray status' | strip)"
  if echo "$SA" | grep -qi 'direct' && echo "$SB" | grep -qi 'direct'; then converged=1; break; fi
  sleep 5
done
SA="$(on "$A" 'ray status' | strip)"; SB="$(on "$B" 'ray status' | strip)"
echo "---- srv-a status ----"; echo "$SA" | sed 's/^/   a| /'
echo "---- srv-b status ----"; echo "$SB" | sed 's/^/   b| /'
[[ "$converged" == 1 ]] && pass "both sides show a [direct] network" || fail "direct network did not form within timeout"

# A direct network must NOT print a shareable join/room id.
if echo "$SB" | grep -qiE 'join [A-Za-z0-9]{20,}'; then
  fail "direct network leaked a room id in status"
else
  pass "direct network hides its room id"
fi

# ---------------------------------------------------------------------------
step "6. reachability — ping over the TUN (both directions)"
A_IP="$(own_ip "$SA")"; B_IP="$(own_ip "$SB")"
echo "   A_IP=$A_IP  B_IP=$B_IP"
if [[ -n "$A_IP" && -n "$B_IP" && "$A_IP" != "$B_IP" ]]; then
  pass "two distinct VPN IPs (srv-a=$A_IP srv-b=$B_IP)"
else
  fail "expected two distinct VPN IPs (srv-a=$A_IP srv-b=$B_IP)"
fi
ping_loss(){ on "$1" "ping -c 3 -W 2 $2" 2>&1 | grep -oE '[0-9]+% packet loss' | grep -oE '^[0-9]+'; }
png(){ # png <from-ip> <target-ip> <label>
  local loss; loss="$(ping_loss "$1" "$2")"
  if [[ "${loss:-100}" == "0" ]]; then pass "ping $3"; else fail "ping $3 (loss=${loss:-?}%)"; fi
}
[[ -n "$A_IP" && -n "$B_IP" ]] && png "$A" "$B_IP" "srv-a -> srv-b ($B_IP)"
[[ -n "$A_IP" && -n "$B_IP" ]] && png "$B" "$A_IP" "srv-b -> srv-a ($A_IP)"

# ---------------------------------------------------------------------------
step "7. data transfer — ray send / ray files accept (both directions)"
# `ray send` resolves the destination by hostname (or short id), not by IP.
# Each side's peer row (● / ○) carries the *other* node's `<host>.<net>.ray`
# name; take its first label as the peer hostname.
peer_host(){ echo "$1" | grep -E '●|○' | grep -oE '[a-z0-9-]+\.[a-z0-9-]+\.ray' | head -1 | cut -d. -f1; }
PEER_OF_A="$(peer_host "$SA")"   # srv-b's hostname, as seen from srv-a
PEER_OF_B="$(peer_host "$SB")"   # srv-a's hostname, as seen from srv-b
echo "   peer-of-a=$PEER_OF_A  peer-of-b=$PEER_OF_B"
send_recv(){ # send_recv <from-ip> <to-ip> <to-peer-hostname> <label>
  local from="$1" to="$2" peer="$3" label="$4"
  on "$from" "head -c 1048576 /dev/urandom > /tmp/c_src.bin; sha256sum /tmp/c_src.bin | cut -d' ' -f1 > /tmp/c_src.sha"
  local src_sha; src_sha="$(on "$from" 'cat /tmp/c_src.sha')"
  on "$from" "ray send /tmp/c_src.bin $peer" 2>&1 | strip | sed 's/^/      send| /'
  # `ray files` rows are `<id> <from> <size> <file> …` with a numeric id; the
  # header row's first column is the literal "id", so match a numeric id.
  local fid=""
  for _ in $(seq 1 12); do
    fid="$(on "$to" 'ray files' 2>/dev/null | strip | awk '$1 ~ /^[0-9]+$/ {print $1; exit}')"
    [[ -n "$fid" ]] && break
    sleep 3
  done
  if [[ -z "$fid" ]]; then fail "$label: no incoming file offer on receiver"; return; fi
  on "$to" "rm -rf /tmp/c_recv && mkdir -p /tmp/c_recv && ray files accept $fid --output /tmp/c_recv" 2>&1 | strip | sed 's/^/      recv| /'
  local dst_sha=""
  for _ in $(seq 1 10); do
    dst_sha="$(on "$to" 'f=$(find /tmp/c_recv -type f | head -1); [ -n "$f" ] && sha256sum "$f" | cut -d" " -f1')"
    [[ -n "$dst_sha" ]] && break
    sleep 2
  done
  if [[ -n "$dst_sha" && "$dst_sha" == "$src_sha" ]]; then
    pass "$label (sha ${src_sha:0:12}… verified)"
  else
    fail "$label (sent ${src_sha:0:12}… got ${dst_sha:0:12}…)"
  fi
}
[[ -n "$PEER_OF_A" ]] && send_recv "$A" "$B" "$PEER_OF_A" "ray send srv-a -> srv-b" || fail "could not resolve srv-b hostname"
[[ -n "$PEER_OF_B" ]] && send_recv "$B" "$A" "$PEER_OF_B" "ray send srv-b -> srv-a (reverse)" || fail "could not resolve srv-a hostname"

# ---------------------------------------------------------------------------
step "8. firewall — network-scoped rule on the direct connection is enforced"
# A direct connection is a real network, so the per-device firewall applies and
# can be scoped to it with --network. Deny inbound ICMP on srv-b for this net,
# confirm srv-a -> srv-b ping breaks, then remove it and confirm it recovers.
NET="$(echo "$SB" | grep -oE '[a-z0-9-]+\.[a-z0-9-]+\.ray' | head -1 | sed -E 's/^[a-z0-9-]+\.([a-z0-9-]+)\.ray/\1/')"
echo "   direct net: $NET"
if [[ -n "$NET" && -n "$A_IP" && -n "$B_IP" ]]; then
  on "$B" "ray firewall add in deny -p icmp --network $NET" 2>&1 | strip | sed 's/^/   b| /'
  BLOCKED="$(ping_loss "$A" "$B_IP")"
  if [[ "${BLOCKED:-0}" == "100" ]]; then pass "network-scoped deny blocks ICMP on the direct net (100% loss)"; else fail "firewall rule did not block ICMP (loss=${BLOCKED:-?}%)"; fi
  on "$B" 'ray firewall remove 0' 2>&1 | strip | sed 's/^/   b| /'
  RECOVERED="$(ping_loss "$A" "$B_IP")"
  if [[ "${RECOVERED:-100}" == "0" ]]; then pass "removing the rule restores ICMP (0% loss)"; else fail "ICMP did not recover after removing rule (loss=${RECOVERED:-?}%)"; fi
else
  fail "could not determine direct net / IPs for firewall test"
fi

# ---------------------------------------------------------------------------
step "9. negative — connecting to an offline contact fails cleanly"
# Put srv-b on standby so its contact record stops being published / endpoint
# is unreachable. A fresh connect from A to B's (now stale) contact id should
# error, not hang.
on "$B" 'ray down' >/dev/null 2>&1 || true
sleep 3
# Rotate B's contact id so A's lookup of the NEW id can't resolve at all
# (deterministic "offline/unknown" rather than racing the TTL).
NEW_B_CID="$(on "$B" 'ray contact rotate' 2>/dev/null | strip | grep -oE '[A-Za-z0-9]{20,}' | head -1)"
sleep 3
OFFLINE_OUT="$(on "$A" "ray connect ${NEW_B_CID:-$B_CID}" 2>&1 | strip)"
echo "$OFFLINE_OUT" | sed 's/^/   a| /'
if echo "$OFFLINE_OUT" | grep -qiE 'offline|unknown|could not resolve|failed'; then
  pass "connect to offline/unknown contact errors cleanly"
else
  fail "connect to offline contact did not produce a clean error"
fi
on "$B" 'ray up' >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
step "summary"
if [[ "$FAILS" -eq 0 ]]; then
  printf '\033[32mALL CHECKS PASSED\033[0m\n'; exit 0
else
  printf '\033[31m%d CHECK(S) FAILED\033[0m\n' "$FAILS"; exit 1
fi
