#!/bin/sh
# tetron-invite-serve.sh - create one invite and serve it on the LAN
# (INVITE-SERVE-001).
#
# Usage: ./tetron-invite-serve.sh NETWORK [PORT]
#
# Prints a URL. On the joiner, enter it in the tetron-tui join field (J), or:
#   curl --proto =http -fsS URL | tetron join -
# Serves until Ctrl+C, then deletes the invite. Needs tetron and python3.
set -eu

net=${1:?usage: $0 NETWORK [PORT]}
port=${2:-7979}

ip=$(ip -4 route get 1.1.1.1 2>/dev/null |
	awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}')
[ -n "$ip" ] || { echo "no default route, can not pick a LAN address" >&2; exit 1; }

dir=$(mktemp -d)
pid=
trap '[ -z "$pid" ] || kill "$pid" 2>/dev/null; rm -rf "$dir"' EXIT
trap 'exit 130' INT TERM
chmod 700 "$dir"
: > "$dir/index.html"
name=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')

json=$(tetron invite "$net" create --json) || { echo "tetron invite $net create failed" >&2; exit 1; }
(umask 077 && printf '%s\n' "$json" |
	python3 -c 'import json, sys; print(json.load(sys.stdin)["invite_key"])' > "$dir/$name") ||
	{ echo "could not read invite_key from tetron output" >&2; exit 1; }

echo "http://$ip:$port/$name"
echo "serving until Ctrl+C" >&2
python3 -m http.server "$port" --bind "$ip" --directory "$dir" &
pid=$!
wait "$pid"
