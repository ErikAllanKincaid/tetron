#!/usr/bin/env bash
# End-to-end flow test for install-tetron-suite.sh --method (PACKAGING-007).
# Runs the real script, core only, against stub curl/sudo/dpkg/apt-get and a
# fake installed `tetron`, and checks which commands it would have run. The
# sudo stub only logs, so nothing on the host is touched.
#
# Usage: contrib/tests/install-suite-method-flow.test.sh [path to script]
SRC="$(cd "$(dirname "${1:-$(dirname "$0")/../install-tetron-suite.sh}")" && pwd)/$(basename "${1:-install-tetron-suite.sh}")"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }

W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
mkdir -p "$W/stubs" "$W/home"
LOG="$W/cmds.log"

# curl: serves the release JSON and fabricated assets with matching .sha256 files.
cat > "$W/stubs/curl" <<'STUB'
#!/bin/bash
out=""; url=""
while [ $# -gt 0 ]; do
	case "$1" in
	-o) out="$2"; shift 2 ;;
	-*) shift ;;
	*) url="$1"; shift ;;
	esac
done
case "$url" in
*/releases/latest) cat "$RELEASE_JSON" ;;
*/releases/latest/download/* | */releases/download/*)
	name="${url##*/}"
	case "$name" in
	*.sha256)
		base="${name%.sha256}"
		printf 'fixture-%s' "$base" > "$(dirname "$out")/$base"
		( cd "$(dirname "$out")" && sha256sum "$base" > "$name" )
		;;
	*) printf 'fixture-%s' "$name" > "$out" ;;
	esac ;;
*) echo "curl stub: unexpected url $url" >&2; exit 22 ;;
esac
STUB
# sudo: log only, never execute.
cat > "$W/stubs/sudo" <<'STUB'
#!/bin/bash
echo "sudo $*" >> "$CMDLOG"
exit 0
STUB
# dpkg: -S -- PATH owns PATH only if it is listed in $OWNED_PATHS.
cat > "$W/stubs/dpkg" <<'STUB'
#!/bin/bash
p="$3"
for o in $OWNED_PATHS; do
	[ "$o" = "$p" ] && { echo "tetron: $p"; exit 0; }
done
echo "dpkg-query: no path found matching pattern $p" >&2; exit 1
STUB
# apt-get must exist for deb_supported; it is only ever run through the sudo stub.
printf '#!/bin/bash\necho "apt-get $*" >> "$CMDLOG"\n' > "$W/stubs/apt-get"
chmod +x "$W/stubs/"*

# The fake installed tetron.
mkdir -p "$W/bin"
fake_installed() { # version
	printf '#!/bin/sh\necho "tetron %s (abc12345)"\n' "$1" > "$W/bin/tetron"
	chmod +x "$W/bin/tetron"
}

pass=0; fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
has()   { grep -qF -- "$1" "$LOG" && ok "$2" || { bad "$2 (log lacks: $1)"; sed 's/^/        log: /' "$LOG"; }; }
hasnt() { grep -qF -- "$1" "$LOG" && { bad "$2 (log has: $1)"; sed 's/^/        log: /' "$LOG"; } || ok "$2"; }

release_json() { # with_deb 0|1
	local deb=""
	[ "$1" -eq 1 ] && deb=',{"name":"tetron_0.14.3-1_amd64.deb"},{"name":"tetron_0.14.3-1_arm64.deb"},{"name":"tetron_0.14.3-1_amd64.deb.sha256"}'
	printf '{"tag_name":"v0.14.3","assets":[{"name":"tetron-linux-x86_64"},{"name":"tetron-linux-aarch64"}%s]}' "$deb" > "$W/release.json"
}

run() { # owned(0|1) args...
	local owned="$1"; shift
	: > "$LOG"
	OWNED_PATHS=""
	[ "$owned" -eq 1 ] && OWNED_PATHS="$W/bin/tetron"
	OUT="$(env HOME="$W/home" PATH="$W/stubs:$W/bin:$PATH" CMDLOG="$LOG" OWNED_PATHS="$OWNED_PATHS" \
		RELEASE_JSON="$W/release.json" bash "$SRC" --core-only "$@" 2>&1 </dev/null)"
	RC=$?
}

echo "auto, raw install -> raw binary (no behaviour change):"
fake_installed 0.14.2; release_json 1; run 0
has "install -m 0755" "places the raw binary"
hasnt "apt-get" "does not use apt"

echo "auto, package-owned, deb published -> upgrade with the deb:"
fake_installed 0.14.2; release_json 1; run 1
has "apt-get install -y" "installs the .deb with apt-get"
has "tetron_0.14.3-1_" "uses the discovered asset name"
has "/usr/bin/tetron install" "runs install on the packaged path"
hasnt "install -m 0755" "does not copy a raw binary over the package"

echo "auto, package-owned, no deb in the release -> skip, never overwrite:"
fake_installed 0.14.2; release_json 0; run 1
hasnt "install -m 0755" "does not overwrite the packaged file"
hasnt "apt-get" "no apt call"
case "$OUT" in *"owned by package tetron"*"not overwriting"*) ok "warns with the package name" ;; *) bad "warning missing: $OUT" ;; esac
case "$OUT" in *"not installed/upgraded: core"*) ok "listed as skipped in the summary" ;; *) bad "summary missing: $OUT" ;; esac

echo "--method binary, package-owned -> raw copy goes to /usr/local/bin:"
fake_installed 0.14.2; release_json 1; run 1 --method binary
has "install -m 0755" "installs a raw binary"
has "/usr/local/bin/tetron" "into the default directory"
hasnt "$W/bin/tetron" "not over the packaged copy"

echo "--method deb, raw install at the latest version -> converts to the package:"
fake_installed 0.14.3; release_json 1; run 0 --method deb
has "apt-get install -y" "installs the .deb even though versions match"
hasnt "install -m 0755" "no raw copy"

echo "--method deb, no deb published -> error:"
fake_installed 0.14.2; release_json 0; run 0 --method deb
[ "$RC" -eq 1 ] && ok "exits 1" || bad "exit code $RC"
case "$OUT" in *"has no .deb"*) ok "says there is no .deb" ;; *) bad "message missing: $OUT" ;; esac
hasnt "apt-get" "no apt call"

echo "--check reports origin and changes nothing:"
fake_installed 0.14.2; release_json 1; run 1 --check
case "$OUT" in *"origin=package:tetron"*) ok "package origin shown" ;; *) bad "origin missing: $OUT" ;; esac
hasnt "apt-get" "no apt call"
hasnt "install -m 0755" "no install"
fake_installed 0.14.2; run 0 --check
case "$OUT" in *"origin=binary"*) ok "binary origin shown" ;; *) bad "origin missing: $OUT" ;; esac

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
