#!/usr/bin/env bash
# Exercises install-tetron-suite.sh's --method decision logic in isolation
# (PACKAGING-007): the argument parsing (run for real, it fails before any
# network access), and the extracted decision functions with a stub dpkg.
#
# Usage: contrib/tests/install-suite-method.test.sh [path to script]
#
# The functions are read straight out of the script at run time rather than
# copied here, so the test and the shipped code cannot drift apart.
SRC="${1:-$(dirname "$0")/../install-tetron-suite.sh}"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }

pass=0; fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
expect() { # desc want got
	if [ "$2" = "$3" ]; then ok "$1 -> '$3'"; else bad "$1: want '$2' got '$3'"; fi
}

# --- argument parsing: runs the real script; every case dies before the network ---
echo "argument parsing:"
out="$(bash "$SRC" --method bogus 2>&1)"; rc=$?
expect "--method bogus exits 1" 1 "$rc"
case "$out" in *"auto, binary or deb"*) ok "--method bogus names the valid values" ;; *) bad "--method bogus message: $out" ;; esac
out="$(bash "$SRC" --method 2>&1)"; rc=$?
expect "--method without a value exits 1" 1 "$rc"
case "$out" in *"needs a value"*) ok "--method without a value says so" ;; *) bad "--method no value message: $out" ;; esac
out="$(bash "$SRC" --method=bogus 2>&1)"; rc=$?
expect "--method=bogus exits 1" 1 "$rc"
out="$(bash "$SRC" --help 2>&1)"
case "$out" in *"--method"*) ok "--help documents --method" ;; *) bad "--help lacks --method" ;; esac

# --- stub dpkg: owns anything whose path ends in /owned ---
STUB="$(mktemp -d)"; trap 'rm -rf "$STUB"' EXIT
cat > "$STUB/dpkg" <<'STUBEOF'
#!/bin/sh
# dpkg -S -- PATH
path="$3"
case "$path" in
*/owned) echo "tetron: $path"; exit 0 ;;
*/owned-multi) echo "alpha, beta: $path"; exit 0 ;;
*) echo "dpkg-query: no path found matching pattern $path" >&2; exit 1 ;;
esac
STUBEOF
chmod +x "$STUB/dpkg"
PATH="$STUB:$PATH"

# Pull the shipped functions out of the script.
for fn in dpkg_owner component_origin choose_method deb_arch_for deb_asset_from_json; do
	body="$(sed -n "/^${fn}() {/,/^}/p" "$SRC")"
	[ -n "$body" ] || { bad "function $fn is missing from the script"; continue; }
	eval "$body"
done

echo
echo "dpkg_owner:"
expect "owned file" tetron "$(dpkg_owner /usr/bin/owned)"
expect "first of several owners" alpha "$(dpkg_owner /usr/bin/owned-multi)"
expect "unowned file" "" "$(dpkg_owner /usr/local/bin/free)"
expect "missing dpkg yields nothing" "" "$(PATH=/nonexistent dpkg_owner /usr/bin/owned)"

echo
echo "component_origin:"
mkdir -p "$STUB/bin"; : > "$STUB/bin/owned"
expect "package owned" "package:tetron" "$(component_origin "$STUB/bin/owned")"
expect "raw binary" binary "$(touch "$STUB/rawbin"; chmod +x "$STUB/rawbin"; component_origin "$STUB/rawbin")"
expect "absent" none "$(component_origin /nonexistent/tool)"

echo
echo "choose_method (METHOD, dest, have_deb):"
choose() { METHOD="$1" choose_method tetron "$2" "$3"; }
expect "auto, fresh install, deb available" binary "$(choose auto /usr/local/bin/free 1)"
expect "auto, raw install, deb available" binary "$(choose auto /usr/local/bin/free 1)"
expect "auto, package-owned, deb available" deb "$(choose auto /usr/bin/owned 1)"
expect "auto, package-owned, no deb asset" skip-owned "$(choose auto /usr/bin/owned 0)"
expect "binary, raw install" binary "$(choose binary /usr/local/bin/free 1)"
expect "binary, package-owned (never overwrite)" binary-default-dir "$(choose binary /usr/bin/owned 1)"
expect "deb, deb available" deb "$(choose deb /usr/local/bin/free 1)"
expect "deb, no deb asset" error-no-deb "$(choose deb /usr/local/bin/free 0)"

echo
echo "deb_arch_for:"
expect "x86_64" amd64 "$(deb_arch_for x86_64)"
expect "aarch64" arm64 "$(deb_arch_for aarch64)"
expect "other" "" "$(deb_arch_for riscv64)"

echo
echo "deb_asset_from_json:"
json='{"assets":[{"name":"tetron-linux-x86_64-musl"},{"name":"tetron_0.14.3-2_amd64.deb"},{"name":"tetron_0.14.3-2_arm64.deb"},{"name":"tetron_0.14.3-2_amd64.deb.sha256"},{"name":"tetron-tui_0.14.2-1_amd64.deb"}]}'
expect "amd64" tetron_0.14.3-2_amd64.deb "$(printf '%s' "$json" | deb_asset_from_json tetron amd64)"
expect "arm64" tetron_0.14.3-2_arm64.deb "$(printf '%s' "$json" | deb_asset_from_json tetron arm64)"
expect "does not confuse tetron with tetron-tui" tetron-tui_0.14.2-1_amd64.deb "$(printf '%s' "$json" | deb_asset_from_json tetron-tui amd64)"
expect "never returns the sha256 file" "" "$(printf '%s' '{"assets":[{"name":"tetron_1-1_amd64.deb.sha256"}]}' | deb_asset_from_json tetron amd64)"
expect "no deb published" "" "$(printf '%s' '{"assets":[{"name":"tetron-linux-x86_64"}]}' | deb_asset_from_json tetron amd64)"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
