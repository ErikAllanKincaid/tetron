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
mkdir -p "$W/stubs" "$W/home" "$W/tmp"
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
# sudo: log, and execute only what stays inside the test directory: apt-get
# (the stub below) and rm/mv/mkdir whose path arguments are all under
# $TESTROOT. Everything else is logged only, so nothing on the host changes.
cat > "$W/stubs/sudo" <<'STUB'
#!/bin/bash
echo "sudo $*" >> "$CMDLOG"
if [ "$1" = env ]; then shift; while [[ "$1" == *=* ]]; do shift; done; fi
[ -n "$TESTROOT" ] || exit 0
case "$1" in
apt-get) "$@"; exit $? ;;
rm | mv | mkdir | install)
	for a in "${@:2}"; do
		case "$a" in /*) case "$a" in "$TESTROOT"/*) ;; *) exit 0 ;; esac ;; esac
	done
	"$@"; exit $? ;;
esac
exit 0
STUB
# systemctl: `show -p ExecStart --value tetron.service` reports the ExecStart of
# $TETRON_SUITE_UNIT_DIR/tetron.service while it exists, else $PKG_UNIT_EXEC
# (the packaged unit). Nothing else is ever active.
cat > "$W/stubs/systemctl" <<'STUB'
#!/bin/bash
case "$*" in
*"show -p ExecStart --value"*)
	u="${TETRON_SUITE_UNIT_DIR:-/nonexistent}/tetron.service"
	if [ -f "$u" ]; then p="$(sed -n 's/^ExecStart=\([^ ]*\).*/\1/p' "$u")"; else p="${PKG_UNIT_EXEC:-}"; fi
	[ -n "$p" ] && echo "{ path=$p ; argv[]=$p daemon ; }"
	exit 0 ;;
esac
exit 1
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
# With $FAKE_PKG_DIR set it "installs" the package: writes $FAKE_PKG_DIR/tetron
# at $PKG_VERSION. $FAKE_APT_FAIL=1 makes it fail.
cat > "$W/stubs/apt-get" <<'STUB'
#!/bin/bash
echo "apt-get $*" >> "$CMDLOG"
if [ "$1" = -s ] && [ "$2" = remove ]; then
	echo "NOTE: This is only a simulation!"
	echo "Remv $3 [0.14.3-1]"
	for x in ${FAKE_APT_REMOVE_EXTRA:-}; do echo "Remv $x [1]"; done
	exit 0
fi
if [ "$1" = remove ]; then
	[ "${FAKE_APT_FAIL:-0}" -eq 1 ] && exit 100
	rm -f "${FAKE_PKG_DIR:-/nonexistent}/tetron"; exit 0
fi
[ "${FAKE_APT_FAIL:-0}" -eq 1 ] && exit 100
if [ -n "${FAKE_PKG_DIR:-}" ]; then
	mkdir -p "$FAKE_PKG_DIR"
	printf '#!/bin/sh\necho "tetron %s (pkg)"\n' "$PKG_VERSION" > "$FAKE_PKG_DIR/tetron"
	chmod +x "$FAKE_PKG_DIR/tetron"
fi
exit 0
STUB
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

echo "--method binary, package-owned, no terminal -> refuses, prints the manual commands (PACKAGING-012):"
fake_installed 0.14.2; release_json 1; run 1 --method binary
[ "$RC" -eq 1 ] && ok "exits 1" || bad "exit code $RC"
case "$OUT" in *"sudo apt-get remove tetron"*) ok "prints the apt-get remove command" ;; *) bad "command missing: $OUT" ;; esac
hasnt "install -m 0755" "no raw copy placed"
hasnt "apt-get remove" "package not removed"

echo "--method deb, raw install at the latest version -> converts to the package:"
fake_installed 0.14.3; release_json 1; run 0 --method deb
has "apt-get install -y" "installs the .deb even though versions match"
case "$OUT" in *"(same version; converting to the package)"*) ok "status says converting, not update available" ;; *) bad "status wording: $OUT" ;; esac
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

echo "shadowing: a raw copy first on PATH hides a package copy (PACKAGING-008):"
mkdir -p "$W/pkgbin"
printf '#!/bin/sh\necho "tetron 0.14.0 (pkg)"\n' > "$W/pkgbin/tetron"; chmod +x "$W/pkgbin/tetron"
fake_installed 0.14.2; release_json 1
: > "$LOG"
OUT="$(env HOME="$W/home" PATH="$W/stubs:$W/bin:$W/pkgbin:$PATH" CMDLOG="$LOG" OWNED_PATHS="$W/pkgbin/tetron" \
	RELEASE_JSON="$W/release.json" bash "$SRC" --core-only --check 2>&1 </dev/null)"
case "$OUT" in *"shadows $W/pkgbin/tetron from package tetron"*) ok "--check reports the shadowing" ;; *) bad "shadow report missing: $OUT" ;; esac
hasnt "rm -f" "--check removes nothing"
: > "$LOG"
OUT="$(env HOME="$W/home" PATH="$W/stubs:$W/bin:$W/pkgbin:$PATH" CMDLOG="$LOG" OWNED_PATHS="$W/pkgbin/tetron" \
	RELEASE_JSON="$W/release.json" bash "$SRC" --core-only 2>&1 </dev/null)"
case "$OUT" in *"shadows $W/pkgbin/tetron from package tetron"*) ok "install run reports the shadowing too" ;; *) bad "shadow report missing on install: $OUT" ;; esac
hasnt "rm -f" "no terminal: neither copy is deleted"

# --- PACKAGING-009 / PACKAGING-010 -------------------------------------------
# Conversion runs: the raw copy is $W/bin/tetron, the package installs to
# $W/pkg (owned), the /etc unit directory is $W/unit, backups go to $W/backup.
line_of() { grep -nF -- "$1" "$LOG" | head -1 | cut -d: -f1; }
before() { # a b desc: log line with a comes before log line with b
	local x y; x="$(line_of "$1")"; y="$(line_of "$2")"
	if [ -n "$x" ] && [ -n "$y" ] && [ "$x" -lt "$y" ]; then ok "$3"; else bad "$3 ($1 at ${x:-none}, $2 at ${y:-none})"; sed 's/^/        log: /' "$LOG"; fi
}
conv_setup() { # raw_version unit_exec(path or empty)
	rm -rf "$W/pkg" "$W/unit" "$W/backup"
	mkdir -p "$W/unit"
	fake_installed "$1"
	[ -n "$2" ] && printf '[Service]\nExecStart=%s daemon\n' "$2" > "$W/unit/tetron.service"
	release_json 1
}
run_conv() { # args...
	: > "$LOG"
	OUT="$(env HOME="$W/home" PATH="$W/stubs:$W/bin:$W/pkg:$PATH" CMDLOG="$LOG" TESTROOT="$W" TMPDIR="$W/tmp" \
		OWNED_PATHS="$W/pkg/tetron" FAKE_PKG_DIR="$W/pkg" PKG_VERSION=0.14.3 PKG_UNIT_EXEC="$W/pkg/tetron" \
		FAKE_APT_FAIL="${FAKE_APT_FAIL:-0}" \
		TETRON_SUITE_UNIT_DIR="$W/unit" TETRON_SUITE_BACKUP_DIR="$W/backup" TETRON_SUITE_PKG_BIN_DIR="$W/pkg" \
		RELEASE_JSON="$W/release.json" bash "$SRC" --core-only "$@" 2>&1 </dev/null)"
	RC=$?
}

echo "PACKAGING-009: --method deb converts a raw install with an /etc unit in one step:"
conv_setup 0.14.2 "$W/bin/tetron"; run_conv --method deb
[ "$RC" -eq 0 ] && ok "exits 0" || bad "exit code $RC: $OUT"
case "$OUT" in *"converting the raw install at $W/bin/tetron"*) ok "prints the plan" ;; *) bad "plan missing: $OUT" ;; esac
has "DEBIAN_FRONTEND=noninteractive apt-get install -y" "apt-get runs non-interactive"
has "systemctl disable tetron.service" "disables the /etc unit"
has "systemctl daemon-reload" "reloads systemd"
[ ! -e "$W/unit/tetron.service" ] && ok "/etc unit is gone" || bad "/etc unit still present"
ls "$W/backup"/tetron.service.* >/dev/null 2>&1 && ok "/etc unit kept in the backup dir" || bad "no backup of the unit"
[ ! -e "$W/bin/tetron" ] && ok "raw copy removed" || bad "raw copy still present"
has "$W/pkg/tetron install" "runs install on the packaged binary"
before "apt-get install" "systemctl disable" "package installed before the unit is touched"
before "systemctl disable" "mv -- $W/unit/tetron.service" "unit disabled before it is moved"
before "mv -- $W/unit/tetron.service" "rm -f -- $W/bin/tetron" "unit moved before the raw copy is removed"
before "rm -f -- $W/bin/tetron" "$W/pkg/tetron install" "raw copy removed before the service restart"
case "$OUT" in *"[pass]  core: version 0.14.3"*) ok "reports the version that runs" ;; *) bad "version line missing: $OUT" ;; esac
case "$OUT" in *"Remove the raw copy"*) bad "asked a second question" ;; *) ok "no separate shadow question" ;; esac

echo "PACKAGING-009: a half-converted host (package installed, raw still first) is completed:"
conv_setup 0.14.3 "$W/bin/tetron"
mkdir -p "$W/pkg"; printf '#!/bin/sh\necho "tetron 0.14.3 (pkg)"\n' > "$W/pkg/tetron"; chmod +x "$W/pkg/tetron"
run_conv --method deb
[ "$RC" -eq 0 ] && ok "exits 0" || bad "exit code $RC: $OUT"
[ ! -e "$W/bin/tetron" ] && [ ! -e "$W/unit/tetron.service" ] && ok "raw copy and /etc unit gone" || bad "conversion not completed"

echo "PACKAGING-009: apt-get fails -> nothing else is touched:"
conv_setup 0.14.2 "$W/bin/tetron"; FAKE_APT_FAIL=1 run_conv --method deb
[ "$RC" -eq 1 ] && ok "exits 1" || bad "exit code $RC"
[ -e "$W/bin/tetron" ] && [ -e "$W/unit/tetron.service" ] && ok "raw copy and unit untouched" || bad "something was removed"
hasnt "systemctl disable" "unit not disabled"

echo "PACKAGING-009/010: an /etc unit that runs another path is left alone and reported:"
conv_setup 0.14.2 "/opt/other/tetron"; run_conv --method deb
[ -e "$W/unit/tetron.service" ] && ok "unit left in place" || bad "foreign unit was moved"
hasnt "systemctl disable" "foreign unit not disabled"
[ "$RC" -eq 1 ] && ok "exits 1" || bad "exit code $RC"
case "$OUT" in *"[fail]"*"runs /opt/other/tetron"*) ok "names the binary the service runs" ;; *) bad "fail line missing: $OUT" ;; esac

echo "PACKAGING-010: a copy first on PATH that is not the installed one is a failure:"
mkdir -p "$W/home/bin"
printf '#!/bin/sh\necho "tetron 0.13.0 (home)"\n' > "$W/home/bin/tetron"; chmod +x "$W/home/bin/tetron"
# Tag 9.9.9: whatever the host has in /usr/local/bin, it is never "up to date".
printf '{"tag_name":"v9.9.9","assets":[{"name":"tetron-linux-x86_64"},{"name":"tetron-linux-aarch64"}]}' > "$W/release.json"
: > "$LOG"
OUT="$(env HOME="$W/home" PATH="$W/stubs:$W/home/bin:$PATH" CMDLOG="$LOG" \
	RELEASE_JSON="$W/release.json" bash "$SRC" --core-only 2>&1 </dev/null)"
RC=$?
[ "$RC" -eq 1 ] && ok "exits 1" || bad "exit code $RC: $OUT"
case "$OUT" in *"[fail]"*"$W/home/bin/tetron comes first on PATH and runs instead of /usr/local/bin/tetron (it is version 0.13.0)"*) ok "names the copy that runs and its version" ;; *) bad "fail line missing: $OUT" ;; esac
case "$OUT" in *"[pass]  core: version"*) bad "still reports a version pass" ;; *) ok "no version pass" ;; esac
rm -rf "$W/home/bin"

# --- PACKAGING-011 / PACKAGING-012 --------------------------------------------
# A sealed PATH (stubs plus links to the host tools the script needs) so a
# tetron installed on the host can never be found.
mkdir -p "$W/tools" "$W/tmp"
for t in bash sh env awk sed grep sort head tail tr cut cat mkdir rm mv mktemp id uname readlink \
	dirname basename install sha256sum date setsid timeout chmod ls touch sleep file ldd; do
	p="$(command -v "$t" 2>/dev/null)" && ln -sf "$p" "$W/tools/$t"
done
run_sealed() { # pathdirs(colon list, before tools) args...
	local dirs="$1"; shift
	: > "$LOG"
	OUT="$(env HOME="$W/home" PATH="$W/stubs:$dirs:$W/tools" CMDLOG="$LOG" TESTROOT="$W" TMPDIR="$W/tmp" \
		OWNED_PATHS="$W/pkg/tetron" FAKE_PKG_DIR="$W/pkg" PKG_VERSION=0.14.3 PKG_UNIT_EXEC="" \
		FAKE_APT_REMOVE_EXTRA="${FAKE_APT_REMOVE_EXTRA:-}" FAKE_APT_FAIL="${FAKE_APT_FAIL:-0}" \
		TETRON_SUITE_UNIT_DIR="$W/unit" TETRON_SUITE_BACKUP_DIR="$W/backup" \
		TETRON_SUITE_PKG_BIN_DIR="$W/pkg" TETRON_SUITE_RAW_BIN_DIR="$W/raw" \
		RELEASE_JSON="$W/release.json" bash "$SRC" --core-only "$@" 2>&1 </dev/null)"
	RC=$?
}
# run_tty: same, but at a terminal (script(1) gives a pty) answering $ANSWER.
SCRIPT_PTY="$(command -v script)"
run_tty() { # pathdirs args...
	local dirs="$1"; shift
	: > "$LOG"
	OUT="$(printf '%s\n' "${ANSWER:-y}" | env HOME="$W/home" PATH="$W/stubs:$dirs:$W/tools" CMDLOG="$LOG" TESTROOT="$W" TMPDIR="$W/tmp" \
		OWNED_PATHS="$W/pkg/tetron" FAKE_PKG_DIR="$W/pkg" PKG_VERSION=0.14.3 PKG_UNIT_EXEC="" \
		FAKE_APT_REMOVE_EXTRA="${FAKE_APT_REMOVE_EXTRA:-}" FAKE_APT_FAIL="${FAKE_APT_FAIL:-0}" \
		TETRON_SUITE_UNIT_DIR="$W/unit" TETRON_SUITE_BACKUP_DIR="$W/backup" \
		TETRON_SUITE_PKG_BIN_DIR="$W/pkg" TETRON_SUITE_RAW_BIN_DIR="$W/raw" \
		RELEASE_JSON="$W/release.json" "$SCRIPT_PTY" -qec "bash '$SRC' --core-only $*; echo rc=\$?" /dev/null 2>&1)"
	RC="$(printf '%s' "$OUT" | tr -d '\r' | sed -n 's/^rc=//p' | tail -1)"
}
pkg_installed() { mkdir -p "$W/pkg"; printf '#!/bin/sh\necho "tetron 0.14.3 (pkg)"\n' > "$W/pkg/tetron"; chmod +x "$W/pkg/tetron"; }
reset_dirs() { rm -rf "$W/pkg" "$W/raw" "$W/unit" "$W/backup" "$W/bin/tetron"; mkdir -p "$W/raw" "$W/unit"; }

echo "PACKAGING-011: auto on a fresh dpkg host with a deb -> installs the deb:"
reset_dirs; release_json 1; run_sealed "$W/raw:$W/pkg"
[ "$RC" -eq 0 ] && ok "exits 0" || bad "exit code $RC: $OUT"
has "apt-get install -y" "installs the .deb"
has "$W/pkg/tetron install" "runs install on the packaged binary"
hasnt "install -m 0755" "no raw copy"

echo "PACKAGING-011: auto on a fresh host, no deb in the release -> raw binary:"
reset_dirs; release_json 0; run_sealed "$W/raw:$W/pkg"
has "install -m 0755" "places the raw binary"
hasnt "apt-get" "no apt call"

echo "PACKAGING-011: auto on a raw install -> stays raw, hints at --method deb:"
reset_dirs; fake_installed 0.14.2; release_json 1; run_sealed "$W/bin:$W/pkg"
has "install -m 0755" "upgrades the raw binary"
hasnt "apt-get install" "no conversion"
case "$OUT" in *"--method deb converts"*) ok "prints the hint" ;; *) bad "hint missing: $OUT" ;; esac
rm -f "$W/bin/tetron"

echo "PACKAGING-012: --method binary on a package host, at a terminal, answer y -> replaces the package:"
reset_dirs; pkg_installed; release_json 1; ANSWER=y run_tty "$W/raw:$W/pkg" --method binary
[ "$RC" = 0 ] && ok "exits 0" || bad "exit code $RC: $OUT"
case "$OUT" in *"replacing package tetron"*) ok "prints the plan" ;; *) bad "plan missing: $OUT" ;; esac
has "install -m 0755" "places the raw binary"
has "apt-get remove -y tetron" "removes the package"
before "systemctl disable tetron.service" "apt-get remove -y tetron" "packaged unit disabled before removal (enable link goes too)"
case "$OUT" in *"(same version; replacing the package)"*) ok "status says replacing, not update available" ;; *) bad "status wording: $OUT" ;; esac
hasnt "purge" "never purges"
has "$W/raw/tetron install" "runs install on the raw binary"
before "install -m 0755" "apt-get remove -y tetron" "raw copy placed before the package is removed"
before "apt-get remove -y tetron" "$W/raw/tetron install" "package removed before the service restart"
[ ! -e "$W/pkg/tetron" ] && [ -e "$W/raw/tetron" ] && ok "package copy gone, raw copy in place" || bad "files wrong"

echo "PACKAGING-012: answer n -> nothing changes:"
reset_dirs; pkg_installed; release_json 1; ANSWER=n run_tty "$W/raw:$W/pkg" --method binary
hasnt "install -m 0755" "no raw copy"
hasnt "apt-get remove" "package kept"
[ -e "$W/pkg/tetron" ] && ok "package copy still there" || bad "package copy gone"

echo "PACKAGING-012: other packages would be removed -> refuses and names them:"
reset_dirs; pkg_installed; release_json 1; FAKE_APT_REMOVE_EXTRA="tetronos-core" ANSWER=y run_tty "$W/raw:$W/pkg" --method binary
[ "$RC" = 1 ] && ok "exits 1" || bad "exit code $RC"
case "$OUT" in *"tetronos-core"*) ok "names the other package" ;; *) bad "name missing: $OUT" ;; esac
hasnt "install -m 0755" "no raw copy"
hasnt "apt-get remove -y" "package kept"

echo "PACKAGING-012: apt-get remove fails -> failed, raw copy named:"
reset_dirs; pkg_installed; release_json 1; FAKE_APT_FAIL=1 ANSWER=y run_tty "$W/raw:$W/pkg" --method binary
[ "$RC" = 1 ] && ok "exits 1" || bad "exit code $RC"
case "$OUT" in *"$W/raw/tetron"*) ok "names the raw copy left in place" ;; *) bad "raw copy not named: $OUT" ;; esac
hasnt "$W/raw/tetron install" "service not switched"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
