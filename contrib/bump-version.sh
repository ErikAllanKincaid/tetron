#!/usr/bin/env bash
# Prepare a tetron release commit (RELEASE-001).
#
# Usage: contrib/bump-version.sh [X.Y.Z|patch|minor|major] [-n] [-y]
#   patch (default)  0.14.6 -> 0.14.7
#   minor, major     0.14.6 -> 0.15.0, 1.0.0
#   X.Y.Z            any version greater than the current one
#   -n               show the state and the plan, change nothing
#   -y               do not ask before changing anything
#
# The version lives at three sites that must stay equal: [package] version in
# Cargo.toml, the tetron-proto dependency requirement in Cargo.toml, and
# [package] version in tetron-proto/Cargo.toml. Cargo.lock follows them
# through a real cargo build, and the CHANGELOG gets the release heading.
#
# It works on a new local branch chore/release-X.Y.Z and makes one commit.
# It never pushes, tags or talks to a remote: it prints those commands for
# you to run when you are ready.
set -euo pipefail

BUILD_CMD="${BUMP_BUILD_CMD:-cargo -q build && cargo -q build --locked}"
GATE_CMD="${BUMP_GATE_CMD:-python3 reconcile.py >/dev/null}"
want=patch dry=0 yes=0
for a in "$@"; do
	case "$a" in
	-n) dry=1 ;;
	-y) yes=1 ;;
	-h|--help) sed -n '2,19s/^# \{0,1\}//p' "$0"; exit 0 ;;
	-*) echo "unknown option: $a" >&2; exit 2 ;;
	*) want="$a" ;;
	esac
done

cd "$(git rev-parse --show-toplevel)"

# Readers, one per site.
pkg_version() { sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)".*/\1/p' "$1" | head -1; }
dep_version() { sed -n 's/^tetron-proto = {.*version = "\([^"]*\)".*/\1/p' Cargo.toml; }
lock_version() { sed -n "/^name = \"$1\"\$/{n;s/^version = \"\([^\"]*\)\"/\1/p}" Cargo.lock; }
unreleased_entries() { sed -n '/^## \[Unreleased\]/,/^## \[/p' CHANGELOG.md | grep -c '^- ' || true; }

v_pkg="$(pkg_version Cargo.toml)"
v_dep="$(dep_version)"
v_proto="$(pkg_version tetron-proto/Cargo.toml)"
v_lock="$(lock_version tetron)"
v_lock_proto="$(lock_version tetron-proto)"
branch="$(git branch --show-current)"
dirty="$(git status --porcelain)"
last_tag="$(git tag -l 'v*' --sort=-v:refname | head -1)"
entries="$(unreleased_entries)"

echo "Current state"
printf '  %-36s %s\n' "Cargo.toml [package]" "$v_pkg" \
	"Cargo.toml tetron-proto dependency" "$v_dep" \
	"tetron-proto/Cargo.toml [package]" "$v_proto" \
	"Cargo.lock tetron" "$v_lock" \
	"Cargo.lock tetron-proto" "$v_lock_proto" \
	"branch" "${branch:-(detached)}" \
	"working tree" "$([ -z "$dirty" ] && echo clean || echo 'not clean')" \
	"newest tag" "${last_tag:-(none)}"
echo "  Unreleased entries: $entries"

# The new version.
IFS=. read -r ma mi pa <<<"$v_pkg"
case "$want" in
patch) new="$ma.$mi.$((pa + 1))" ;;
minor) new="$ma.$((mi + 1)).0" ;;
major) new="$((ma + 1)).0.0" ;;
*) new="$want" ;;
esac
rel="chore/release-$new"

problems=()
[ "$v_pkg" = "$v_dep" ] && [ "$v_pkg" = "$v_proto" ] && [ "$v_pkg" = "$v_lock" ] && [ "$v_pkg" = "$v_lock_proto" ] \
	|| problems+=("the version sites disagree (see above); fix them by hand first")
if ! [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	problems+=("'$new' is not X.Y.Z")
elif [ "$new" = "$v_pkg" ] || [ "$(printf '%s\n%s\n' "$v_pkg" "$new" | sort -V | tail -1)" != "$new" ]; then
	problems+=("$new is not greater than $v_pkg")
fi
[ -z "$dirty" ] || problems+=("working tree is not clean; commit or stash first")
[ "$branch" = main ] || problems+=("branch is '${branch:-(detached)}', not main")
git rev-parse -q --verify "refs/tags/v$new" >/dev/null && problems+=("tag v$new exists")
git rev-parse -q --verify "refs/heads/$rel" >/dev/null && problems+=("branch $rel exists")
[ "$entries" -gt 0 ] || problems+=("CHANGELOG ## [Unreleased] has no entries; describe the release first")

echo
echo "Plan: $v_pkg -> $new on branch $rel"
if [ ${#problems[@]} -gt 0 ]; then
	echo "Refusing:"
	printf '  - %s\n' "${problems[@]}"
	exit 1
fi
[ "$dry" -eq 1 ] && { echo "Dry run: nothing changed."; exit 0; }
if [ "$yes" -eq 0 ]; then
	printf 'Proceed? [y/N] '
	read -r reply </dev/tty || reply=
	[[ "$reply" =~ ^[Yy] ]] || { echo "Nothing changed."; exit 1; }
fi

backout() {
	echo
	echo "FAILED: $1"
	echo "Nothing was committed. To return to where you were:"
	echo "  git checkout -- . && git switch main && git branch -D $rel"
	exit 1
}

git switch -q -c "$rel"
sed -i '/^\[package\]/,/^\[/s/^version = "[^"]*"/version = "'"$new"'"/' Cargo.toml tetron-proto/Cargo.toml
sed -i 's/^\(tetron-proto = {.*version = "\)[^"]*\(".*\)/\1'"$new"'\2/' Cargo.toml
sed -i "0,/^## \[Unreleased\]\$/s//## [Unreleased]\n\n## [$new] - $(date +%F)/" CHANGELOG.md

[ "$(pkg_version Cargo.toml)" = "$new" ] && [ "$(dep_version)" = "$new" ] && [ "$(pkg_version tetron-proto/Cargo.toml)" = "$new" ] \
	|| backout "a version site did not take the new value"
echo "Wrote $new to the three sites and the CHANGELOG."

echo "Building (refreshes Cargo.lock, then checks it with --locked)..."
sh -c "$BUILD_CMD" || backout "build failed"
[ "$(lock_version tetron)" = "$new" ] && [ "$(lock_version tetron-proto)" = "$new" ] \
	|| backout "Cargo.lock still holds the old version after the build"
echo "Running reconcile.py..."
sh -c "$GATE_CMD" || backout "reconcile.py failed (run python3 reconcile.py to see why)"

git add Cargo.toml tetron-proto/Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "chore: bump version to $new"
echo
echo "Committed on $rel: $(git log -1 --format='%h %s')"
echo "When you are ready to release:"
echo "  git switch main && git merge --ff-only $rel"
echo "  git tag v$new && git push origin main && git push origin v$new"
echo "  git branch -d $rel"
