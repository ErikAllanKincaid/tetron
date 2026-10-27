#!/usr/bin/env bash
# Exercises install-tetron-suite.sh's shadow detection (PACKAGING-008): a raw
# binary that comes first on PATH hides a package-owned copy of the same tool.
#
# Usage: contrib/tests/install-suite-shadow.test.sh [path to script]
#
# Functions are extracted from the shipped script at run time. dpkg and sudo
# are stubs; nothing on the host is touched.
SRC="${1:-$(dirname "$0")/../install-tetron-suite.sh}"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }

pass=0; fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
expect() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: want '$2' got '$3'"; fi; }

W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
mkdir -p "$W/stubs" "$W/d1" "$W/d2" "$W/d3" "$W/tools"
# A private PATH tail with only the tools the functions need, so the host's own
# tetron binaries can never leak into a result.
for t in readlink awk; do ln -s "$(command -v $t)" "$W/tools/$t"; done
LOG="$W/cmds.log"; : > "$LOG"

# dpkg stub: owns exactly the (resolved) paths listed in $OWNED_PATHS.
cat > "$W/stubs/dpkg" <<'STUB'
#!/bin/bash
p="$3"
for o in $OWNED_PATHS; do
	[ "$o" = "$p" ] && { echo "tetron: $p"; exit 0; }
done
echo "dpkg-query: no path found matching pattern $p" >&2; exit 1
STUB
cat > "$W/stubs/sudo" <<'STUB'
#!/bin/bash
echo "sudo $*" >> "$CMDLOG"
STUB
chmod +x "$W/stubs/"*
export CMDLOG="$LOG"

log_warn() { printf '[warn]  %s\n' "$*" >&2; }
log_pass() { printf '[pass]  %s\n' "$*" >&2; }
HAVE_TTY=0
have_tty() { [ "$HAVE_TTY" -eq 1 ]; }
TTYIN=/dev/null
# The script sets SUDO=sudo when not run as root; the stub sudo only logs.
SUDO=sudo

for fn in dpkg_owner path_copies shadowed_package_copy; do
	body="$(sed -n "/^${fn}() {/,/^}/p" "$SRC")"
	[ -n "$body" ] || { bad "function $fn is missing from the script"; continue; }
	eval "$body"
done
body="$(sed -n '/^offer_remove_shadow() {/,/^}/p' "$SRC" | sed 's|< /dev/tty|< "$TTYIN"|')"
[ -n "$body" ] || bad "function offer_remove_shadow is missing from the script"
eval "$body"

mk() { : > "$1"; chmod +x "$1"; }
mk "$W/d1/tetron"; mk "$W/d2/tetron"
ln -sf "$W/d2/tetron" "$W/d3/tetron"

detect() { # PATH owned_paths
	PATH="$1:$W/stubs:$W/tools" OWNED_PATHS="$2" shadowed_package_copy tetron
}

echo "shadowed_package_copy:"
expect "raw first, package later -> reported" "$W/d1/tetron|$W/d2/tetron|tetron" "$(detect "$W/d1:$W/d2" "$W/d2/tetron")"
expect "package first, raw later -> not a shadow" "" "$(detect "$W/d2:$W/d1" "$W/d2/tetron")"
expect "only a raw copy" "" "$(detect "$W/d1" "")"
expect "only a package copy" "" "$(detect "$W/d2" "$W/d2/tetron")"
expect "two raw copies, none packaged" "" "$(detect "$W/d1:$W/d2" "")"
expect "symlink to the packaged file counts once and is packaged" "$W/d1/tetron|$W/d3/tetron|tetron" "$(detect "$W/d1:$W/d3:$W/d2" "$W/d2/tetron")"
expect "no dpkg on the host" "" "$(PATH="$W/d1:$W/d2:$W/tools" OWNED_PATHS="$W/d2/tetron" shadowed_package_copy tetron)"

echo
echo "path_copies lists each file once:"
expect "symlink duplicate collapsed" "2" "$(PATH="$W/d1:$W/d3:$W/d2:$W/tools" path_copies tetron | wc -l | tr -d ' ')"

echo
echo "offer_remove_shadow (default is no; never without a yes):"
ask() { # have_tty reply
	HAVE_TTY="$1"; TTYIN=/dev/null
	if [ "$1" -eq 1 ]; then TTYIN="$(mktemp)"; printf '%s\n' "$2" > "$TTYIN"; fi
	: > "$LOG"
	PATH="$W/stubs:$PATH" offer_remove_shadow tetron "$W/d1/tetron" >/dev/null 2>&1
	local rc=$?
	[ "$1" -eq 1 ] && rm -f "$TTYIN"
	return $rc
}
ask 0 ""; rc=$?;    expect "no terminal: declines" 1 "$rc"; expect "no terminal: nothing run" 0 "$(wc -l < "$LOG" | tr -d ' ')"
ask 1 "";  rc=$?;   expect "bare Enter declines" 1 "$rc";   expect "bare Enter: nothing run" 0 "$(wc -l < "$LOG" | tr -d ' ')"
ask 1 "n"; rc=$?;  expect "n declines" 1 "$rc"
ask 1 "y"; rc=$?;  expect "y accepts" 0 "$rc"; expect "y removes only the raw copy" "sudo rm -f -- $W/d1/tetron" "$(cat "$LOG")"
ask 1 "Y"; rc=$?;  expect "Y accepts" 0 "$rc"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
