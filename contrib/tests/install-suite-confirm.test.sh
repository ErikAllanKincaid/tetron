#!/usr/bin/env bash
# Exercises install-tetron-suite.sh's confirm_sudo_install() decision table
# in isolation (ADDONS-SUITE-004).
#
# Usage: contrib/tests/install-suite-confirm.test.sh [path to script]
#
# The function under test reads its prompts from /dev/tty, which a test
# cannot drive, so this extracts the function and points those reads at
# $TTYIN instead. Only the redirection changes; every branch under test is
# the shipped one, read straight out of the script at run time rather than
# copied here, so the two cannot drift apart.
#
# The two cases marked below are regressions, not hypotheticals: both
# failed against the script as it stood before ADDONS-SUITE-004, which is
# how a documented `curl | bash` upgrade refreshed the addons and left the
# core daemon stale.
SRC="${1:-$(dirname "$0")/../install-tetron-suite.sh}"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }

log_warn() { printf '[warn]  %s\n' "$*" >&2; }
TTYIN=/dev/null
HAVE_TTY=1
have_tty() { [ "$HAVE_TTY" -eq 1 ]; }

# Pull just the function out, redirecting its prompt reads at $TTYIN.
eval "$(sed -n '/^confirm_sudo_install() {/,/^}/p' "$SRC" | sed 's|< /dev/tty|< "$TTYIN"|')"

pass=0; fail=0
check() {
	local desc="$1" want="$2" comp="$3" is_upgrade="$4" tty="$5" reply="$6"
	HAVE_TTY="$tty"
	TTYIN=/dev/null
	if [ "$tty" -eq 1 ]; then
		TTYIN="$(mktemp)"
		printf '%s\n' "$reply" > "$TTYIN"
	fi
	confirm_sudo_install "$comp" "$is_upgrade" >/dev/null 2>&1
	local got=$?
	if [ "$got" -eq "$want" ]; then
		printf '  ok    %s (rc=%d)\n' "$desc" "$got"; pass=$((pass + 1))
	else
		printf '  FAIL  %s: want rc=%d got rc=%d\n' "$desc" "$want" "$got"; fail=$((fail + 1))
	fi
	[ "$tty" -eq 1 ] && rm -f "$TTYIN"
	return 0
}

echo "confirm_sudo_install: 0 = proceed, 1 = skip"
echo
echo "core upgrade -- the case that was silently declining:"
check "no tty, proceeds (was: skipped without --yes-core)" 0 core 1 0 ""
check "tty, bare Enter accepts (was: [y/N], Enter declined)" 0 core 1 1 ""
check "tty, explicit n declines"                            1 core 1 1 "n"
check "tty, explicit N declines"                            1 core 1 1 "N"
check "tty, explicit y accepts"                             0 core 1 1 "y"
echo
echo "core fresh install -- unchanged, never gated:"
check "no tty, proceeds" 0 core 0 0 ""
check "tty, Enter accepts" 0 core 0 1 ""
check "tty, n declines" 1 core 0 1 "n"
echo
echo "addons -- unchanged, [Y/n]:"
check "webui upgrade, tty, Enter accepts" 0 webui 1 1 ""
check "webui upgrade, tty, n declines"    1 webui 1 1 "n"
check "webui upgrade, no tty, proceeds"   0 webui 1 0 ""
check "hosts upgrade, tty, Enter accepts" 0 hosts 1 1 ""

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
