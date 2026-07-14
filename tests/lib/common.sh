# Shared helpers for the tetron e2e / benchmark test orchestrators.
# Sourced (not executed) by each scenario's run.sh after it sets DIR/ROOT/SERVERS.
# Provides SSH plumbing, PASS/FAIL accounting, and host-lifecycle helpers
# (wait-for-ssh, state reset, deploy, daemon-up) so the run.sh scripts contain
# only their scenario-specific steps.

KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null \
          -o ConnectTimeout=10 -o LogLevel=ERROR -o BatchMode=yes)

# PASS/FAIL accounting. FAILS is read by summary().
FAILS=0
pass(){ printf '  \033[32mPASS\033[0m %s\n' "$*"; }
fail(){ printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAILS=$((FAILS+1)); }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

# summary : print the final tally and exit non-zero if any check failed.
summary(){
  step "summary"
  if [[ "$FAILS" -eq 0 ]]; then
    printf '\033[32mALL CHECKS PASSED\033[0m\n'; exit 0
  else
    printf '\033[31m%d CHECK(S) FAILED\033[0m\n' "$FAILS"; exit 1
  fi
}

# on <ip> <command-string> : run a shell command on a host as root.
# -n: never read stdin, so calling `on` inside a `while read` loop can't eat it.
on(){ local ip="$1"; shift; ssh -n "${SSH_OPTS[@]}" -i "$KEY" "root@$ip" "$*"; }

# strip : remove ANSI colour codes from tetron CLI output (stdin -> stdout).
strip(){ sed -r 's/\x1B\[[0-9;]*[mGKH]//g'; }

# own_ip <status-text> : extract a node's own VPN IPv4 (10.88.0.0/16 CGNAT range).
own_ip(){ echo "$1" | grep -oE '10\.88\.[0-9]+\.[0-9]+' | head -1; }


# ping_loss <from-ip> <target-ip> : echo the packet-loss percentage (number only).
ping_loss(){ on "$1" "ping -c 3 -W 2 $2" 2>&1 | grep -oE '[0-9]+% packet loss' | grep -oE '^[0-9]+'; }

# png <from-ip> <target-ip> <label> : PASS if 0% loss, FAIL otherwise.
png(){
  local loss; loss="$(ping_loss "$1" "$2")"
  if [[ "${loss:-100}" == "0" ]]; then pass "ping $3"; else fail "ping $3 (loss=${loss:-?}%)"; fi
}

# server_ip <servers-file> <label> : echo the public ip for a label in a
# `id ip label zone` .servers file. Avoids bash-3.2 associative arrays.
server_ip(){
  local f="$1" want="$2" id ip label zone
  while read -r id ip label zone; do
    [[ "${label:-}" == "$want" ]] && { echo "$ip"; return 0; }
  done < "$f"
  return 1
}

# wait_all_ssh <ip...> : block until every host accepts SSH; abort on timeout.
wait_all_ssh(){
  local ip
  for ip in "$@"; do
    local ok=0 _
    for _ in $(seq 1 60); do on "$ip" true 2>/dev/null && { ok=1; break; }; sleep 5; done
    if [[ "$ok" == 1 ]]; then pass "ssh reachable ($ip)"; else fail "ssh ($ip) unreachable"; echo "aborting"; exit 1; fi
  done
}

# seed_known_hosts <ip...> : pre-seed ~/.ssh/known_hosts so `just deploy` (which
# uses the default known_hosts) doesn't block on an interactive host-key prompt.
seed_known_hosts(){
  local h
  for h in "$@"; do ssh-keyscan -T 10 "$h" >> ~/.ssh/known_hosts 2>/dev/null || true; done
}

# reset_state <ip...> : clean-slate the daemon (stop + wipe the config tree) so
# runs are reproducible on already-used servers. Set KEEP_STATE=1 to skip.
# Linux config lives in /etc/tetron; /root/.config/tetron is the pre-migration
# location (wiped too so an upgraded VM doesn't migrate stale state back in).
reset_state(){
  [[ "${KEEP_STATE:-0}" == "1" ]] && return 0
  step "reset tetron state on all hosts (KEEP_STATE=1 to skip)"
  local h
  for h in "$@"; do
    on "$h" 'systemctl stop tetron 2>/dev/null; rm -rf /etc/tetron /root/.config/tetron' && echo "   reset $h"
  done
}

# deploy_all <root> <ip...> : cross-build + rsync + tetron up on each host; abort on failure.
deploy_all(){
  local root="$1"; shift
  step "deploy tetron to all hosts (cross build + rsync + tetron up)"
  local ip
  for ip in "$@"; do
    echo ">> just deploy $ip"
    if ( cd "$root" && just deploy "$ip" ); then pass "deploy $ip"; else fail "deploy $ip"; echo "aborting"; exit 1; fi
  done
}

# wait_daemons <ip...> : give daemons a moment to settle, then confirm `tetron status` responds.
wait_daemons(){
  sleep 5
  local ip
  for ip in "$@"; do
    if on "$ip" 'tetron status' >/dev/null 2>&1; then pass "daemon up on $ip"; else fail "daemon not responding on $ip"; fi
  done
}

# ---------------------------------------------------------------------------
# JSON-backed status helpers. Every `ray` subcommand takes a global `--json`
# flag (color/spinners off, machine-readable). We run it on the remote host and
# parse the JSON *locally* with jq, so assertions don't scrape coloured tables.
# jq is already a provisioning prerequisite (see tests/e2e/README.md).
# ---------------------------------------------------------------------------

# status_json <ip> : echo `tetron status --json` from a host (raw JSON).
status_json(){ on "$1" 'tetron status --json' 2>/dev/null; }

# my_ip4 <ip> [net] : this node's own VPN IPv4 — for the named network, or the
# first network if omitted. Empty if none.
my_ip4(){
  status_json "$1" | jq -r --arg n "${2:-}" '
    (.networks // [])
    | (if $n == "" then .[0] else (map(select(.name == $n)) | .[0]) end)
    | .my_ip // empty'
}

# peer_ip4 <ip> <peer-hostname> [net] : a specific peer's VPN IPv4 as seen by
# <ip>. Searches the named network, or all networks if net omitted. Empty if the
# peer isn't present.
peer_ip4(){
  status_json "$1" | jq -r --arg h "$2" --arg n "${3:-}" '
    (.networks // [])
    | (if $n == "" then . else map(select(.name == $n)) end)
    | [ .[].peers[] | select((.hostname // "") == $h) ] | .[0].ip // empty'
}

# peer_online <ip> <peer-hostname> [net] : echo 1 if that peer has a live
# connection (.connection != null), else 0.
peer_online(){
  local r
  r="$(status_json "$1" | jq -r --arg h "$2" --arg n "${3:-}" '
    (.networks // [])
    | (if $n == "" then . else map(select(.name == $n)) end)
    | [ .[].peers[] | select((.hostname // "") == $h) ] | .[0]
    | if . != null and .connection != null then "1" else "0" end')"
  echo "${r:-0}"
}

# net_role <ip> <net> : the node's role on a network (lowercased:
# coordinator/member/direct). Empty if the node isn't on that network.
net_role(){
  status_json "$1" | jq -r --arg n "$2" '
    (.networks // []) | map(select(.name == $n)) | .[0].role // empty' \
    | tr 'A-Z' 'a-z'
}

# has_net <ip> <net> : exit 0 if the node has a network by that name.
has_net(){
  [[ -n "$(status_json "$1" | jq -r --arg n "$2" \
    '(.networks // []) | map(select(.name == $n)) | .[0].name // empty')" ]]
}

# ---------------------------------------------------------------------------
# Polling / convergence
# ---------------------------------------------------------------------------

# retry_until <secs> <shell-cond...> : eval the condition every 3s until it
# succeeds or <secs> elapse. Returns the condition's last exit status.
retry_until(){
  local secs="$1"; shift
  local end=$((SECONDS + secs))
  while (( SECONDS < end )); do
    if eval "$*"; then return 0; fi
    sleep 3
  done
  return 1
}

# _roster_has <ip> <host...> : exit 0 iff every named host is online from <ip>.
_roster_has(){
  local ip="$1"; shift
  local h
  for h in "$@"; do [[ "$(peer_online "$ip" "$h")" == "1" ]] || return 1; done
}

# wait_roster <ip> <host...> : block (≤120s) until all named peers are online
# from <ip>'s view, then PASS/FAIL.
wait_roster(){
  local ip="$1"; shift
  if retry_until 120 "_roster_has '$ip' $*"; then
    pass "roster converged on $ip (sees: $*)"
  else
    fail "roster did not converge on $ip (want: $*)"
  fi
}


# ---------------------------------------------------------------------------
# Admission (approval-only — tetron mints no invites, MINIMAL-013)
# ---------------------------------------------------------------------------

# join_approve <joiner-ip> <coord-ip> <net> <room> <hostname> : the joiner dials
# the closed network with the bare room id (which queues it for approval), then
# the coordinator (or any co-coordinator) waits for the request and accepts it.
# This is the only admission path in tetron. Returns non-zero if the request
# never appears.
join_approve(){
  local joiner="$1" coord="$2" net="$3" room="$4" host="$5"
  on "$joiner" "tetron join $room --hostname $host" 2>&1 | strip | sed "s/^/   $host| /"
  local rid=""
  retry_until 60 "rid=\"\$(request_id '$coord' '$net' '$host')\"; [[ -n \"\$rid\" ]]" || return 1
  rid="$(request_id "$coord" "$net" "$host")"
  on "$coord" "tetron accept $net $rid" 2>&1 | strip | sed "s/^/   acc| /"
}

# request_id <coord-ip> <net> <hostname> : the short id of a queued join request
# matching <hostname> (from `tetron requests <net> --json`). Empty if none.
request_id(){
  on "$1" "tetron requests $2 --json" 2>/dev/null \
    | jq -r --arg h "$3" 'map(select((.hostname // "") == $h)) | .[0].id // empty'
}

# peer_endpoint <ip> <peer-hostname> [net] : a peer's full endpoint id as seen by
# <ip> (for `tetron admin add`, which prefix-matches). Empty if absent.
peer_endpoint(){
  status_json "$1" | jq -r --arg h "$2" --arg n "${3:-}" '
    (.networks // [])
    | (if $n == "" then . else map(select(.name == $n)) end)
    | [ .[].peers[] | select((.hostname // "") == $h) ] | .[0].endpoint_id // empty'
}
