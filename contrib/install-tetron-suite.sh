#!/usr/bin/env bash
# install-tetron-suite.sh: install or upgrade tetron, tetron-webui,
# tetron-systray, and tetron-backup.sh to their latest GitHub releases in
# one pass. Idempotent -- a component already at the latest release version
# is left untouched.
# Fetch this file directly (raw.githubusercontent.com) and run it -- cloning
# this repo first is not required.
#
# Usage:
#   ./install-tetron-suite.sh [--check] [--yes-core] [core] [webui] [systray] [backup]
#
#   --check      report installed vs. latest versions only, change nothing
#   --yes-core   allow the core `tetron` component to be installed/upgraded
#                without an interactive confirmation prompt (needed for a
#                non-interactive/scripted run; installing/upgrading core
#                needs sudo and briefly disconnects every peer on this
#                host while the daemon restarts)
#   component names (core/webui/systray/backup) restrict which components
#   this run touches -- default is all four, freshly installed or
#   upgraded as needed. It is called "install", not "upgrade" -- a
#   component missing on this host is installed, not silently skipped.
#
#   backup is a script, not a versioned binary: it fetches
#   contrib/tetron-backup.sh from the tetron repo's main branch (no
#   release tag to compare against), installs to /usr/local/bin next to
#   the core binary, and is considered up to date whenever it is already
#   present -- delete it first to force a refresh.

set -uo pipefail

log_info()  { printf '[info]  %s\n' "$*" >&2; }
log_warn()  { printf '[warn]  %s\n' "$*" >&2; }
log_error() { printf '[error] %s\n' "$*" >&2; }
log_pass()  { printf '[pass]  %s\n' "$*" >&2; }
log_fail()  { printf '[fail]  %s\n' "$*" >&2; }

fatal() {
	log_error "$*"
	exit 1
}

require_cmd() {
	local cmd
	for cmd in "$@"; do
		command -v "$cmd" >/dev/null 2>&1 || fatal "required command not found: $cmd"
	done
}

require_cmd curl install

CHECK_ONLY=0
YES_CORE=0
COMPONENTS=()

for arg in "$@"; do
	case "$arg" in
	--check) CHECK_ONLY=1 ;;
	--yes-core) YES_CORE=1 ;;
	core | webui | systray | backup)
		COMPONENTS+=("$arg")
		;;
	-h | --help)
		sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) fatal "unrecognized argument: $arg (see --help)" ;;
	esac
done
[ "${#COMPONENTS[@]}" -eq 0 ] && COMPONENTS=(core webui systray backup)

# --- platform detection, same suffixes tetron-webui's addons.rs::asset_suffix() uses ---
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
Linux) plat_os=linux ;;
Darwin) plat_os=macos ;;
*) fatal "unsupported OS: $os" ;;
esac
case "$arch" in
x86_64 | amd64) plat_arch=x86_64 ;;
aarch64 | arm64) plat_arch=aarch64 ;;
*) fatal "unsupported architecture: $arch" ;;
esac
PLATFORM="${plat_os}-${plat_arch}"

# --- per-component config ---
component_binary() { case "$1" in core) echo tetron ;; webui) echo tetron-webui ;; systray) echo tetron-systray ;; backup) echo tetron-backup.sh ;; esac; }
component_repo() { case "$1" in core) echo ErikAllanKincaid/tetron ;; webui) echo ErikAllanKincaid/tetron-webui ;; systray) echo ErikAllanKincaid/tetron-systray ;; backup) echo ErikAllanKincaid/tetron ;; esac; }
component_default_dir() { case "$1" in core) echo /usr/local/bin ;; backup) echo /usr/local/bin ;; *) echo "$HOME/.local/bin" ;; esac; }
component_needs_sudo() { case "$1" in core) echo 1 ;; backup) echo 1 ;; *) echo 0 ;; esac; }

# Prefer wherever the binary is actually already installed (PATH) over the
# default dir, so an install in a nonstandard location is upgraded in
# place rather than a second copy appearing at the default path.
component_dest() {
	local comp="$1" bin
	bin="$(component_binary "$comp")"
	if command -v "$bin" >/dev/null 2>&1; then
		command -v "$bin"
	else
		echo "$(component_default_dir "$comp")/$bin"
	fi
}

latest_tag() {
	local repo="$1" json tag
	json="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest")" || return 1
	tag="$(printf '%s' "$json" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
	[ -n "$tag" ] || return 1
	printf '%s\n' "${tag#v}"
}

# Prints the installed semver (e.g. "0.8.2"), or nothing if the binary is
# missing or predates the `--version`/`-V`/`version` support all three
# gained 2026-07-25 -- an unparseable/missing version is treated as
# "needs installing/upgrading", not an error, so old installs still get
# picked up.
installed_version() {
	local dest="$1" out
	[ -x "$dest" ] || return 0
	out="$("$dest" --version 2>/dev/null)" || out="$("$dest" version 2>/dev/null)" || true
	# clap's default `--version` output is "<bin-name> <version> (<sha>)".
	printf '%s\n' "$out" | awk 'NF>=2{print $2}'
}

verify_checksum() {
	local file="$1" sumfile="$2" dir sumfile_name
	dir="$(dirname "$file")"
	sumfile_name="$(basename "$sumfile")"
	if command -v sha256sum >/dev/null 2>&1; then
		(cd "$dir" && sha256sum -c "$sumfile_name") >/dev/null
	elif command -v shasum >/dev/null 2>&1; then
		(cd "$dir" && shasum -a 256 -c "$sumfile_name") >/dev/null
	else
		fatal "neither sha256sum nor shasum is available to verify the download"
	fi
}

confirm_core() {
	[ "$YES_CORE" -eq 1 ] && return 0
	if [ ! -t 0 ]; then
		log_warn "core needs --yes-core to install/upgrade non-interactively -- skipping core"
		return 1
	fi
	local reply
	read -r -p "Install/upgrade core tetron now? This needs sudo and briefly disconnects every peer on this host. [y/N] " reply
	[[ "$reply" =~ ^[Yy]$ ]]
}

install_component() {
	local comp="$1" dest="$2" bin repo asset tmpdir sudo_prefix
	bin="$(component_binary "$comp")"
	repo="$(component_repo "$comp")"
	asset="${bin}-${PLATFORM}"
	sudo_prefix=""
	[ "$(component_needs_sudo "$comp")" -eq 1 ] && sudo_prefix="sudo"

	tmpdir="$(mktemp -d)"
	trap 'rm -rf "$tmpdir"' RETURN

	log_info "$comp: downloading $asset from $repo release..."
	curl -fsSL -o "$tmpdir/$asset" "https://github.com/$repo/releases/latest/download/$asset" \
		|| fatal "$comp: failed to download $asset"
	curl -fsSL -o "$tmpdir/$asset.sha256" "https://github.com/$repo/releases/latest/download/$asset.sha256" \
		|| fatal "$comp: failed to download $asset.sha256"
	verify_checksum "$tmpdir/$asset" "$tmpdir/$asset.sha256" \
		|| fatal "$comp: checksum verification failed"

	log_info "$comp: installing to $dest..."
	$sudo_prefix mkdir -p "$(dirname "$dest")"
	$sudo_prefix install -m 0755 "$tmpdir/$asset" "$dest" \
		|| fatal "$comp: failed to install binary to $dest"

	log_info "$comp: running '$dest install' to register/restart its service..."
	$sudo_prefix "$dest" install \
		|| fatal "$comp: '$dest install' failed -- binary is updated but its service was not restarted, check logs"

	log_pass "$comp: installed/upgraded successfully"
	ver="$(installed_version "$dest")"
	[ -n "$ver" ] && log_pass "$comp: version $ver"
}

# The backup component is a raw script, not a release binary: no checksum
# sidecar convention and no `install` subcommand to register a service.
# Fetches contrib/tetron-backup.sh from the repo's main branch (floating,
# same as the webui's own Config Backup proxy -- there is no release tag
# for a script) and drops it next to the core binary.
install_backup() {
	local comp="$1" dest="$2" bin repo url tmpdir sudo_prefix
	bin="$(component_binary "$comp")"
	repo="$(component_repo "$comp")"
	url="https://raw.githubusercontent.com/$repo/main/contrib/$bin"
	sudo_prefix=""
	[ "$(component_needs_sudo "$comp")" -eq 1 ] && sudo_prefix="sudo"

	tmpdir="$(mktemp -d)"
	trap 'rm -rf "$tmpdir"' RETURN

	log_info "$comp: downloading $url ..."
	curl -fsSL -o "$tmpdir/$bin" "$url" \
		|| fatal "$comp: failed to download $bin"
	log_info "$comp: installing to $dest..."
	$sudo_prefix mkdir -p "$(dirname "$dest")"
	$sudo_prefix install -m 0755 "$tmpdir/$bin" "$dest" \
		|| fatal "$comp: failed to install $bin to $dest"

	log_pass "$comp: installed/upgraded successfully"
}

any_failed=0
for comp in "${COMPONENTS[@]}"; do
	dest="$(component_dest "$comp")"

	# backup is a script with no version: skip the release-tag compare
	# entirely. Present = up to date; missing = install (same confirm as
	# core -- root-owned /usr/local/bin) when passed explicitly.
	if [ "$comp" = "backup" ]; then
		if [ -x "$dest" ]; then
			log_pass "$comp: installed ($dest)"
		else
			log_info "$comp: not installed"
			[ "$CHECK_ONLY" -eq 1 ] && continue
			confirm_core || continue
			install_backup "$comp" "$dest" || any_failed=1
		fi
		continue
	fi

	current="$(installed_version "$dest")"
	latest="$(latest_tag "$(component_repo "$comp")")" || {
		log_error "$comp: failed to query latest release -- skipping"
		any_failed=1
		continue
	}

	status="$comp: installed=${current:-none} latest=$latest"
	if [ -n "$current" ] && [ "$current" = "$latest" ]; then
		log_pass "$status (up to date)"
		continue
	fi
	log_info "$status ($( [ -z "$current" ] && echo "not installed or version unknown" || echo "update available" ))"

	[ "$CHECK_ONLY" -eq 1 ] && continue

	if [ "$(component_needs_sudo "$comp")" -eq 1 ]; then
		confirm_core || continue
	fi

	install_component "$comp" "$dest" || any_failed=1
done

exit "$any_failed"
