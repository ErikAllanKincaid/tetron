#!/usr/bin/env bash
# Runs install-tetron-suite.sh end to end inside throwaway Docker containers
# and checks what it actually does.
#
# Usage: contrib/tests/install-suite-sandbox.test.sh [path to script]
#
# Nothing on the host is installed, upgraded, or restarted. Each case starts a
# fresh container from $IMAGE (default debian:trixie, which has no sudo and no
# curl). Inside it, `curl` and `systemctl` are test stand-ins: curl serves fake
# release binaries and checksums from a local fixture directory instead of
# GitHub, and records every URL requested. The fake binaries answer
# `--version` and log `install`/`restart-all` calls. No terminal is attached,
# so the script runs in its non-interactive mode.
set -uo pipefail

SRC="${1:-$(dirname "$0")/../install-tetron-suite.sh}"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }
IMAGE="${IMAGE:-debian:trixie}"
VER=9.9.9
command -v docker >/dev/null 2>&1 || { echo "docker is required" >&2; exit 1; }

FIX="$(mktemp -d)"
trap 'rm -rf "$FIX"' EXIT
mkdir -p "$FIX/bin" "$FIX/release" "$FIX/fail"
cp "$SRC" "$FIX/script"
echo "$VER" > "$FIX/version"

# Fake release binary: reports its version, logs service actions.
fake_bin() {
	printf '#!/bin/sh\ncase "$1" in\n--version | version) echo "%s %s (test)" ;;\ninstall | restart-all) echo "%s $1" >> /fix/actions.log ;;\nesac\n' "$1" "$2" "$1"
}
for bin in tetron tetron-webui tetron-tui tetron-systray tetron-hosts tetron-sync-receiver tetron-messageboard; do
	for asset in "$bin-linux-x86_64" "$bin-linux-x86_64-musl"; do
		fake_bin "$bin" "$VER" > "$FIX/release/$asset"
		(cd "$FIX/release" && sha256sum "$asset" > "$asset.sha256")
	done
done

cat > "$FIX/bin/curl" <<'EOF'
#!/bin/sh
# Test stand-in for curl: serves the fixture directory, logs each URL.
out=/dev/stdout url=""
while [ $# -gt 0 ]; do
	case "$1" in -o) out="$2"; shift 2 ;; -*) shift ;; *) url="$1"; shift ;; esac
done
echo "$url" >> /fix/curl.log
case "$url" in
https://api.github.com/repos/*/releases/latest)
	name="${url%/releases/latest}"; name="${name##*/}"
	[ -e "/fix/fail/$name" ] && exit 22
	printf '{"url": "x", "tag_name": "v%s", "name": "y"}\n' "$(cat /fix/version)" > "$out" ;;
https://github.com/*/releases/*)
	asset="${url##*/}"
	[ -e /fix/fail/interrupt ] && kill -TERM "$PPID"
	[ -e "/fix/fail/$asset" ] && exit 22
	cat "/fix/release/$asset" > "$out" || exit 22 ;;
*/contrib/install-tetron-suite.sh) cat /fix/script > "$out" ;;
*/contrib/tetron-backup.sh) printf '#!/bin/sh\necho backup\n' > "$out" ;;
*) exit 22 ;;
esac
EOF
printf '#!/bin/sh\nexit 0\n' > "$FIX/bin/systemctl"
chmod -R a+rwX "$FIX"
chmod a+x "$FIX/bin/"* "$FIX/release/"*

# run <docker args...> -- <shell command>: one fresh container per case.
OUT="" RC=0
run() {
	local args=()
	while [ "$1" != "--" ]; do args+=("$1"); shift; done
	shift
	rm -f "$FIX/curl.log" "$FIX/actions.log" "$FIX/fail/"*
	OUT="$(docker run --rm "${args[@]}" -v "$FIX:/fix" \
		-e PATH=/fix/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
		"$IMAGE" bash -c "$1" 2>&1)"
	RC=$?
}

pass=0 fail=0
check() {
	local desc="$1"; shift
	if "$@"; then
		printf '  ok    %s\n' "$desc"; pass=$((pass + 1))
	else
		printf '  FAIL  %s\n' "$desc"; fail=$((fail + 1))
		printf '%s\n' "$OUT" | sed 's/^/        | /' | tail -15
	fi
}
has() { printf '%s\n' "$OUT" | grep -q -- "$1"; }
rc_is() { [ "$RC" -eq "$1" ]; }
log_has() { grep -q -- "$2" "$FIX/$1" 2>/dev/null; }
log_lacks() { ! log_has "$@"; }

HEADER="$(awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$SRC")"

echo "--help"
run -- 'bash /fix/script --help'
check "prints the whole header comment" [ "$OUT" = "$HEADER" ]
run -- 'bash -s -- --help < /fix/script'
check "prints the same text when piped into bash" [ "$OUT" = "$HEADER" ]

echo "argument checks"
run -- 'bash /fix/script --core-only --install-tui'
check "--core-only with --install-tui is refused" rc_is 1
check "  with a usage error" has "cannot be combined"
check "  and nothing is fetched" log_lacks curl.log http

echo "fresh install as root, no sudo on the system"
run -- 'bash /fix/script --core-only && /usr/local/bin/tetron --version'
check "succeeds" rc_is 0
check "  core binary placed and runs" has "tetron $VER (test)"
check "  core service registered" log_has actions.log "tetron install"
check "  download pinned to the queried release tag" log_has curl.log "/releases/download/v$VER/tetron-linux-x86_64"
check "  never uses the floating latest/download URL" log_lacks curl.log /latest/download/

echo "non-root without sudo"
run --user 1000:1000 -e HOME=/tmp -- 'bash /fix/script --core-only'
check "refused" rc_is 1
check "  says sudo is needed" has "sudo"
check "  before downloading anything" log_lacks curl.log /releases/download/
run --user 1000:1000 -e HOME=/tmp -- 'bash /fix/script --check'
check "--check still works without sudo" rc_is 0

echo "one component fails"
run -- 'touch /fix/fail/tetron-webui-linux-x86_64; bash /fix/script --install-webui --install-tui; echo "rc=$?"; ls /usr/local/bin'
check "run exits non-zero" has "rc=1"
check "  later components still installed" has "^tetron-tui$"
check "  failure named in the closing summary" has "failed: webui"

echo "interrupted during a download"
run -- 'touch /fix/fail/interrupt; bash /fix/script --core-only; echo "tmp=$(ls -A /tmp | wc -l)"'
check "temporary files removed" has "tmp=0"

echo "older copy in a home directory ahead on PATH"
run -- 'mkdir -p /root/.local/bin && cp /fix/release/tetron-tui-linux-x86_64 /root/.local/bin/tetron-tui && sed -i "s/9.9.9/0.0.1/" /root/.local/bin/tetron-tui
	export PATH=/root/.local/bin:$PATH; bash /fix/script --no-core --install-tui; ls /usr/local/bin'
check "new copy installed to /usr/local/bin" has "^tetron-tui$"
check "  warns that the old copy shadows it" has "/root/.local/bin/tetron-tui comes first on PATH"

echo "upgrade of an existing install, defaults, no terminal"
run -- 'cp /fix/release/tetron-linux-x86_64 /usr/local/bin/tetron && sed -i "s/9.9.9/0.0.1/" /usr/local/bin/tetron
	bash /fix/script; echo "rc=$?"; tetron --version; ls /usr/local/bin'
check "succeeds" has "rc=0"
check "  core upgraded" has "tetron $VER (test)"
check "  tui added (headless default)" has "^tetron-tui$"

echo "already up to date"
run -- 'cp /fix/release/tetron-linux-x86_64 /usr/local/bin/tetron; bash /fix/script --core-only'
check "reports up to date" has "up to date"
check "  downloads nothing" log_lacks curl.log /releases/download/

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
