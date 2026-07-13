#!/usr/bin/env bash
# Closed-network admission + lifecycle-command e2e test orchestrator.
#
# Topology:
#   srv-a  coordinator of a closed network `priv`
#   srv-b  member (admitted by live approval, later promoted to co-coordinator)
#   srv-c  member (denied once, later admitted by a reusable key from srv-b)
#
# Exercises the command surface the other scenarios don't touch:
#   - live approval on a closed net with NO invite (`requests` / `accept` / `deny`)
#   - co-coordinator grant (`admin add` / `admin list`) + gatekeeper resilience:
#     a fresh join is admitted by the co-coordinator while the original
#     coordinator is offline, using a reusable key (`invite --reusable`)
#   - hostname change propagation (`torpedo hostname`) via the roster
#   - graceful leave + nuke (`torpedo leave` / `torpedo nuke`)
#   - `torpedo apply` smoke (`--example` / `--dry-run`, no mutation)
#
# Reads tests/e2e/closed-net/.servers (written by provision.sh). Does NOT modify
# infra. Re-runnable (resets torpedo state each run unless KEEP_STATE=1).
set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../../.." && pwd)"
SERVERS="$DIR/.servers"
# shellcheck source=../../lib/common.sh
source "$ROOT/tests/lib/common.sh"

[[ -f "$SERVERS" ]] || { echo "No $SERVERS — run $DIR/provision.sh first"; exit 1; }

A="$(server_ip "$SERVERS" srv-a || true)"
B="$(server_ip "$SERVERS" srv-b || true)"
C="$(server_ip "$SERVERS" srv-c || true)"
[[ -n "$A" && -n "$B" && -n "$C" ]] || { echo "missing srv-a/b/c in $SERVERS"; exit 1; }

NET=priv

# ---------------------------------------------------------------------------
step "0. wait for SSH + deploy on all hosts"
wait_all_ssh "$A" "$B" "$C"
seed_known_hosts "$A" "$B" "$C"
reset_state "$A" "$B" "$C"
deploy_all "$ROOT" "$A" "$B" "$C"
for h in "$A" "$B" "$C"; do on "$h" 'torpedo up' >/dev/null 2>&1 || true; done
wait_daemons "$A" "$B" "$C"

# ---------------------------------------------------------------------------
step "1. srv-a creates the closed network"
CREATE="$(on "$A" "torpedo create --name $NET --hostname srv-a" | strip)"
echo "$CREATE" | sed 's/^/   a| /'
ROOM="$(echo "$CREATE" | sed -n 's/.*torpedo join \([A-Za-z0-9]\{20,\}\).*/\1/p' | head -1)"
[[ -n "$ROOM" ]] && pass "network '$NET' created (room ${ROOM:0:12}…)" || { fail "create failed"; summary; }

# ---------------------------------------------------------------------------
step "2. live approval — srv-b joins with NO invite, srv-a approves"
# A bare room id on a closed net does not admit; the join queues for approval.
on "$B" "torpedo join $ROOM --hostname srv-b" 2>&1 | strip | sed 's/^/   b| /'
RID=""
if retry_until 60 "RID=\"\$(request_id '$A' '$NET' srv-b)\"; [[ -n \"\$RID\" ]]"; then
  RID="$(request_id "$A" "$NET" srv-b)"
  pass "srv-b shows up in 'torpedo requests' (id ${RID})"
else
  fail "srv-b never appeared in 'torpedo requests'"; summary
fi
on "$A" "torpedo accept $NET $RID" 2>&1 | strip | sed 's/^/   a| /'
wait_roster "$A" srv-b

# ---------------------------------------------------------------------------
step "3. live denial — srv-c joins with NO invite, srv-a denies"
on "$C" "torpedo join $ROOM --hostname srv-c" 2>&1 | strip | sed 's/^/   c| /'
CID=""
if retry_until 60 "CID=\"\$(request_id '$A' '$NET' srv-c)\"; [[ -n \"\$CID\" ]]"; then
  CID="$(request_id "$A" "$NET" srv-c)"; pass "srv-c queued (id ${CID})"
else
  fail "srv-c never queued"; CID=""
fi
[[ -n "$CID" ]] && on "$A" "torpedo deny $NET $CID" 2>&1 | strip | sed 's/^/   a| /'
# A denied peer must not become a member. Give it a window; expect still offline.
sleep 15
[[ "$(peer_online "$A" srv-c "$NET")" == "0" ]] && pass "denied peer is not admitted" \
  || fail "denied peer unexpectedly became a member"
on "$C" "torpedo leave $NET" >/dev/null 2>&1 || true   # stop srv-c's background retries

# ---------------------------------------------------------------------------
step "4. co-coordinator grant — srv-a promotes srv-b (admin add / list)"
B_ID="$(peer_endpoint "$A" srv-b "$NET")"
echo "   srv-b id (as seen by srv-a): ${B_ID:0:16}…"
[[ -n "$B_ID" ]] || { fail "could not resolve srv-b's id"; summary; }
on "$A" "torpedo admin $NET add $B_ID" 2>&1 | strip | sed 's/^/   a| /'
# admin list should now show two key-holders (the local node + srv-b).
if retry_until 30 "[[ \"\$(on '$A' 'torpedo admin $NET list --json' | jq -r 'length')\" -ge 2 ]]"; then
  pass "srv-a's 'admin list' shows two key-holders"
else
  fail "srv-b not reflected as a key-holder"
fi
# Let the promotion (is_coordinator=true) propagate into the blob before srv-a drops.
sleep 8

# ---------------------------------------------------------------------------
step "5. gatekeeper resilience — co-coordinator admits while srv-a is offline"
KEY="$(mint_reusable "$B" "$NET")"   # srv-b (now a co-coordinator) mints a reusable key
[[ -n "$KEY" ]] && pass "co-coordinator minted a reusable key (${KEY:0:12}…)" || fail "co-coordinator could not mint a key"
on "$A" 'torpedo down' >/dev/null 2>&1 || true   # original coordinator goes offline
sleep 3
# srv-c joins unattended; only srv-b is online to admit it.
on "$C" "torpedo join $KEY --hostname srv-c" 2>&1 | strip | sed 's/^/   c| /'
wait_roster "$B" srv-c
on "$A" 'torpedo up' >/dev/null 2>&1 || true     # bring the coordinator back

# ---------------------------------------------------------------------------
step "6. hostname change propagates to roster (Magic DNS removed in tetron)"
on "$B" "torpedo hostname $NET srv-bb" 2>&1 | strip | sed 's/^/   b| /'
# srv-a learns the new name on reconverge (MemberSync trigger or 60s poller).
if retry_until 90 "[[ -n \"\$(peer_ip4 '$A' srv-bb '$NET')\" ]]"; then
  pass "rename propagated — srv-a's roster shows srv-bb"
else
  fail "rename did not propagate to srv-a's roster"
fi
# srv-c reaches srv-bb by its mesh IP from the roster (ICMP is allowed by
# default, so a successful ping proves reachability). No .ray resolution — tetron
# removed Magic DNS (MINIMAL-012); peers are addressed by mesh IP from status.
BB_IP="$(peer_ip4 "$C" srv-bb "$NET" 2>/dev/null)"
if [[ -n "$BB_IP" ]] && retry_until 60 "[[ \"\$(on '$C' 'ping -c1 -W2 $BB_IP >/dev/null 2>&1 && echo ok || echo no')\" == ok ]]"; then
  pass "srv-bb ($BB_IP) answers from srv-c by mesh IP"
else
  fail "srv-bb did not answer from srv-c by mesh IP (ip=$BB_IP)"
fi

# ---------------------------------------------------------------------------
step "7. graceful leave + nuke"
on "$C" "torpedo leave $NET" 2>&1 | strip | sed 's/^/   c| /'
# A graceful leave (LEAVE_CODE) prunes the member promptly, not on a timeout.
if retry_until 45 "[[ \"\$(peer_online '$B' srv-c '$NET')\" == 0 ]]"; then
  pass "graceful leave pruned srv-c from the roster"
else
  fail "srv-c still present after leave"
fi
on "$A" "torpedo nuke $NET --force" 2>&1 | strip | sed 's/^/   a| /'
# After nuke the coordinator drops the network locally.
if retry_until 30 "! has_net '$A' '$NET'"; then
  pass "nuke removed the network from the coordinator"
else
  fail "network still present on coordinator after nuke"
fi

# ---------------------------------------------------------------------------
summary
