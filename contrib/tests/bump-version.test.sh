#!/usr/bin/env bash
# Exercises contrib/bump-version.sh (RELEASE-001) in a throwaway git
# repository holding copies of the four files it edits. Nothing touches this
# checkout: cargo and reconcile.py are replaced through BUMP_BUILD_CMD and
# BUMP_GATE_CMD, and the fake build rewrites Cargo.lock the way cargo would.
#
# Usage: contrib/tests/bump-version.test.sh [path to bump-version.sh]
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$(cd "$HERE/.." && pwd)/bump-version.sh"
SRC="${1:-$SRC}"
ROOT="$(cd "$HERE/../.." && pwd)"
[ -r "$SRC" ] || { echo "cannot read $SRC" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
export BUMP_GATE_CMD=true
# Stand-in for cargo build: set both lockfile entries to the manifest version.
export BUMP_BUILD_CMD='v=$(sed -n "3s/^version = \"\(.*\)\"/\1/p" Cargo.toml); sed -i "/^name = \"tetron\(-proto\)\{0,1\}\"$/{n;s/^version = .*/version = \"$v\"/}" Cargo.lock'

pass=0; fail=0
ok() { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; [ -n "${2:-}" ] && printf '%s\n' "$2" | tail -8 | sed 's/^/        /'; fail=$((fail + 1)); }

# fresh: a repo at version 0.14.6 on main with one Unreleased entry.
fresh() {
	rm -rf "$WORK/r"; mkdir -p "$WORK/r/tetron-proto"; cd "$WORK/r" || exit 1
	git init -q -b main
	printf '[package]\nname = "tetron"\nversion = "0.14.6"\n\n[dependencies]\ntetron-proto = { path = "tetron-proto", version = "0.14.6" }\n' > Cargo.toml
	printf '[package]\nname = "tetron-proto"\nversion = "0.14.6"\n' > tetron-proto/Cargo.toml
	printf '[[package]]\nname = "tetron"\nversion = "0.14.6"\n\n[[package]]\nname = "tetron-proto"\nversion = "0.14.6"\n' > Cargo.lock
	printf '# Changelog\n\n## [Unreleased]\n\n### Fixed\n- a fix\n\n## [0.14.6] - 2026-10-31\n' > CHANGELOG.md
	git add -A && git commit -q -m init && git tag v0.14.6
}
runb() { out="$(bash "$SRC" "$@" 2>&1)"; rc=$?; }
ver() { sed -n "$2p" "$1" | sed 's/.*version = "\([^"]*\)".*/\1/'; }

echo "bump-version.sh: $SRC"

fresh; runb -n
if [ $rc -eq 0 ] && grep -q '0.14.6 -> 0.14.7' <<<"$out" && [ -z "$(git status --porcelain)" ] && [ "$(git branch --show-current)" = main ]; then ok "dry run shows the plan and changes nothing"; else bad "dry run" "$out"; fi
grep -q 'Unreleased entries: 1' <<<"$out" && ok "state shows Unreleased entry count" || bad "state count" "$out"

fresh; runb -y
if [ $rc -eq 0 ] && [ "$(git branch --show-current)" = chore/release-0.14.7 ] \
	&& [ "$(ver Cargo.toml 3)" = 0.14.7 ] && [ "$(ver Cargo.toml 6)" = 0.14.7 ] && [ "$(ver tetron-proto/Cargo.toml 3)" = 0.14.7 ]; then
	ok "patch bump writes all three sites on a release branch"
else bad "patch bump" "$out"; fi
[ "$(git log -1 --format=%B | sed '/^$/d')" = "chore: bump version to 0.14.7" ] && ok "commit message is the subject only" || bad "commit message" "$(git log -1 --format=%B)"
grep -q "^## \[0.14.7\] - $(date +%F)$" CHANGELOG.md && grep -q '^## \[Unreleased\]$' CHANGELOG.md && ok "CHANGELOG heading rolled, Unreleased kept" || bad "CHANGELOG" "$(cat CHANGELOG.md)"
[ -z "$(git status --porcelain)" ] && git show --stat HEAD | grep -q Cargo.lock && ok "Cargo.lock committed, tree clean" || bad "lockfile in commit" "$(git status --porcelain)"
grep -q 'git tag v0.14.7' <<<"$out" && ok "prints the tag and push commands" || bad "next commands" "$out"
[ -z "$(git tag -l v0.14.7)" ] && ok "does not tag" || bad "tagged"

fresh; runb minor -y; [ $rc -eq 0 ] && [ "$(ver Cargo.toml 3)" = 0.15.0 ] && ok "minor -> 0.15.0" || bad "minor" "$out"
fresh; runb major -y; [ $rc -eq 0 ] && [ "$(ver Cargo.toml 3)" = 1.0.0 ] && ok "major -> 1.0.0" || bad "major" "$out"
fresh; runb 0.20.3 -y; [ $rc -eq 0 ] && [ "$(ver tetron-proto/Cargo.toml 3)" = 0.20.3 ] && ok "explicit version" || bad "explicit" "$out"

# refuse WANT_TEXT SETUP ARGS...: must exit non-zero, name the reason, change nothing.
refuse() {
	local want="$1" setup="$2"; shift 2
	fresh; eval "$setup"; local before; before="$(git status --porcelain; git rev-parse HEAD; git branch --show-current)"
	runb "$@" -y
	if [ $rc -ne 0 ] && grep -qi "$want" <<<"$out" && [ "$before" = "$(git status --porcelain; git rev-parse HEAD; git branch --show-current)" ]; then
		ok "refuses: $want"
	else bad "refuse: $want (rc=$rc)" "$out"; fi
}
refuse 'not clean' 'echo x >> CHANGELOG.md'
refuse 'not main' 'git switch -q -c other'
refuse 'tag v0.14.7 exists' 'git tag v0.14.7'
refuse 'branch chore/release-0.14.7 exists' 'git branch chore/release-0.14.7'
refuse 'not greater' ':' 0.14.6
refuse 'not greater' ':' 0.14.5
refuse 'not X.Y.Z' ':' 1.2
refuse 'Unreleased' 'sed -i "/^### Fixed/d; /^- a fix/d" CHANGELOG.md && git commit -qam empty'
refuse 'disagree' 'sed -i "3s/0.14.6/0.14.5/" tetron-proto/Cargo.toml && git commit -qam skew'
refuse 'disagree' 'sed -i "3s/0.14.6/0.14.5/" Cargo.lock && git commit -qam skew'

fresh; BUMP_BUILD_CMD=false runb -y
if [ $rc -ne 0 ] && grep -q 'git switch main' <<<"$out" && [ "$(git log -1 --format=%s)" = init ]; then ok "build failure: no commit, prints how to back out"; else bad "build failure" "$out"; fi
fresh; BUMP_BUILD_CMD=true runb -y
if [ $rc -ne 0 ] && grep -qi 'Cargo.lock' <<<"$out"; then ok "a build that leaves Cargo.lock stale is caught"; else bad "stale lock after build" "$out"; fi

fresh; runb -y </dev/null; grep -q 'push' "$SRC" && ! grep -nE '^[^#]*\bgit (push|tag [^-]|fetch|pull)' "$SRC" | grep -v 'echo\|printf\|say' >/dev/null && ok "never runs git push, tag, fetch or pull" || bad "remote or tag command in script" "$(grep -nE 'git (push|tag|fetch|pull)' "$SRC")"

# The real checkout's sites agree right now (same parser the script uses).
cd "$ROOT" && runb -n -y 2>/dev/null; grep -q 'Cargo.toml \[package\]' <<<"$out" && ok "reads this checkout" || bad "reads this checkout" "$out"

echo "passed $pass, failed $fail"
[ "$fail" -eq 0 ]
