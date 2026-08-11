#!/usr/bin/env bash
# install-tetron-suite.sh: install or upgrade tetron, tetron-webui,
# tetron-systray, and tetron-backup.sh to their latest GitHub releases in
# one pass. Idempotent -- a component already at the latest release version
# is left untouched.
# Fetch this file directly (raw.githubusercontent.com) and run it -- cloning
# this repo first is not required.
#
# Usage:
#   ./install-tetron-suite.sh [--check] [--yes-core] [--musl]
#       [--core-only | --install-webui] [--install-systray] [--install-backup] [--install-all]
#
#   --check           report installed vs. latest versions only, change nothing
#   --yes-core        allow `core` (and `backup`) to be upgraded without an
#                      interactive confirmation prompt (needed for a
#                      non-interactive/scripted upgrade; upgrading core
#                      needs sudo and briefly disconnects every peer on
#                      this host while the daemon restarts). Not needed
#                      for a *fresh* install of core/backup -- nothing is
#                      running yet, so there is nothing to disrupt.
#   --musl            force the statically-linked musl `core` build regardless
#                      of host detection (core only -- webui/systray/backup have
#                      no musl variant). Auto-selected without this flag when
#                      the host is Alpine (`/etc/os-release`), or when the
#                      already-installed `tetron` binary is itself static, so an
#                      existing musl install upgrades in place instead of
#                      silently flipping to glibc.
#   --core-only        install/upgrade core only, no addons. Errors if
#                      combined with any --install-* flag below.
#   --install-webui    include webui (core is always included).
#   --install-systray  include systray (core is always included).
#   --install-backup   include backup (core is always included). Unlike
#                      webui/systray, backup is never in the default set --
#                      opt-in only, via this flag or the interactive picker.
#   --install-all      core + webui + systray + backup.
#
#   With none of the above selection flags, the script picks components
#   itself. Prompts (below) read from /dev/tty, not stdin, so the
#   README's `curl | bash` one-liner still prompts normally when a human
#   is running it at a real terminal -- stdin there is the piped script,
#   not the keyboard, but /dev/tty reaches the controlling terminal
#   directly regardless (same trick rustup/Homebrew's installers use):
#     - no controlling terminal at all (cron, CI, a container run without
#       -it): behaves as --core-only, since there is nothing to prompt.
#     - a controlling terminal is reachable: detects a display
#       ($DISPLAY/$WAYLAND_DISPLAY) and prints the resulting default
#       (core alone if headless; core+webui+systray with a display), then
#       asks "Use defaults? [Y/n]" -- enter accepts it, "n" drops into a
#       yes/no prompt per addon (each still pre-filled with the same
#       display-aware default).
#
#   It is called "install", not "upgrade" -- a selected component missing
#   on this host is installed, not silently skipped.
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
MUSL_OVERRIDE=0
CORE_ONLY=0
INSTALL_WEBUI=0
INSTALL_SYSTRAY=0
INSTALL_BACKUP=0
INSTALL_ALL=0
SELECTION_FLAG_GIVEN=0

for arg in "$@"; do
	case "$arg" in
	--check) CHECK_ONLY=1 ;;
	--yes-core) YES_CORE=1 ;;
	--musl) MUSL_OVERRIDE=1 ;;
	--core-only) CORE_ONLY=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-webui) INSTALL_WEBUI=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-systray) INSTALL_SYSTRAY=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-backup) INSTALL_BACKUP=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-all) INSTALL_ALL=1; SELECTION_FLAG_GIVEN=1 ;;
	-h | --help)
		sed -n '2,59p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) fatal "unrecognized argument: $arg (see --help)" ;;
	esac
done

[ "$CORE_ONLY" -eq 1 ] && [ "$((INSTALL_WEBUI + INSTALL_SYSTRAY + INSTALL_BACKUP + INSTALL_ALL))" -gt 0 ] \
	&& fatal "--core-only cannot be combined with --install-webui/--install-systray/--install-backup/--install-all"

# host_has_display: true if a display server is reachable -- systray can
# never function without one, and webui defaults follow it too (a remote
# port-forward to webui is possible but rare, not worth defaulting on).
host_has_display() {
	[ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]
}

# have_tty: true if a controlling terminal is reachable via /dev/tty --
# deliberately independent of stdin (`[ -t 0 ]`), which is always false
# under `curl ... | bash` even when a human is sitting at a real terminal
# running it (stdin there is the piped script itself). /dev/tty reaches
# the controlling terminal directly regardless of stdin redirection, the
# same trick rustup/Homebrew's installers use to stay interactive when
# piped. `:` reads nothing -- this only tests that the open succeeds.
# Genuinely fails (no controlling terminal at all) in cron/CI/containers
# without a pty, which is the correct signal for non-interactive.
have_tty() {
	{ : < /dev/tty; } 2>/dev/null
}

COMPONENTS=(core)
if [ "$INSTALL_ALL" -eq 1 ]; then
	COMPONENTS=(core webui systray backup)
elif [ "$SELECTION_FLAG_GIVEN" -eq 1 ]; then
	[ "$CORE_ONLY" -eq 1 ] || {
		[ "$INSTALL_WEBUI" -eq 1 ] && COMPONENTS+=(webui)
		[ "$INSTALL_SYSTRAY" -eq 1 ] && COMPONENTS+=(systray)
		[ "$INSTALL_BACKUP" -eq 1 ] && COMPONENTS+=(backup)
	}
elif [ "$CHECK_ONLY" -eq 1 ]; then
	# Read-only status check: use the display-aware default set with no
	# prompt at all -- there is nothing to confirm before just reporting.
	host_has_display && COMPONENTS=(core webui systray)
elif ! have_tty; then
	# No controlling terminal at all (cron, CI, a container run without
	# -it) -- nothing to prompt on. Default to core-only and tell the
	# user how to get addons instead of guessing.
	log_info "no controlling terminal detected -- defaulting to core-only (pass --install-webui/--install-systray/--install-backup/--install-all, or run this script with a terminal attached, to include addons)"
else
	if host_has_display; then
		DEFAULT_COMPONENTS=(core webui systray)
		log_info "display detected"
	else
		DEFAULT_COMPONENTS=(core)
		log_info "no display detected (headless)"
	fi
	log_info "default install: ${DEFAULT_COMPONENTS[*]}"
	reply=""
	read -r -p "Use defaults? [Y/n] " reply < /dev/tty
	if [ -z "$reply" ] || [[ "$reply" =~ ^[Yy]$ ]]; then
		COMPONENTS=("${DEFAULT_COMPONENTS[@]}")
	else
		ask_addon() {
			local name="$1" default_yes="$2" reply prompt
			prompt="[y/N]"
			[ "$default_yes" -eq 1 ] && prompt="[Y/n]"
			read -r -p "Install $name? $prompt " reply < /dev/tty
			if [ "$default_yes" -eq 1 ]; then
				[[ "$reply" =~ ^[Nn]$ ]] && return 1 || return 0
			else
				[[ "$reply" =~ ^[Yy]$ ]]
			fi
		}
		webui_default=0; systray_default=0
		host_has_display && { webui_default=1; systray_default=1; }
		ask_addon webui "$webui_default" && COMPONENTS+=(webui)
		ask_addon systray "$systray_default" && COMPONENTS+=(systray)
		ask_addon backup 0 && COMPONENTS+=(backup)
	fi
fi

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

# Alpine ships musl only -- no glibc at all, so the plain asset can't even
# run there. `/etc/os-release` is the standard, reliable source for this
# (avoids guessing from `ldd` output, which varies across musl distros and
# is sometimes absent entirely).
host_is_alpine() {
	[ -r /etc/os-release ] || return 1
	(
		. /etc/os-release
		case "${ID:-} ${ID_LIKE:-}" in
		*alpine*) exit 0 ;;
		*) exit 1 ;;
		esac
	)
}

# True if the binary already at $1 is statically linked -- lets an
# existing musl install upgrade in place instead of silently flipping to
# glibc (found live 2026-08-08: `rpi5-test` had a manually-installed
# static musl `core`, and a plain upgrade through this script replaced it
# with a dynamic glibc binary with no warning that linkage changed).
dest_is_static() {
	local dest="$1"
	[ -x "$dest" ] || return 1
	if command -v ldd >/dev/null 2>&1; then
		ldd "$dest" 2>&1 | grep -q "not a dynamic executable"
	else
		command -v file >/dev/null 2>&1 && file "$dest" 2>/dev/null | grep -q "statically linked"
	fi
}

# Non-empty (the reason, for logging) when `core` should fetch the musl
# asset instead of the plain glibc one. Only `core` has a musl variant --
# webui/systray/backup do not, so callers must gate this to comp=core.
musl_reason() {
	local dest="$1"
	if [ "$MUSL_OVERRIDE" -eq 1 ]; then
		echo "--musl requested"
	elif host_is_alpine; then
		echo "Alpine host"
	elif dest_is_static "$dest"; then
		echo "existing install is statically linked"
	fi
}

# --- per-component config ---
component_binary() { case "$1" in core) echo tetron ;; webui) echo tetron-webui ;; systray) echo tetron-systray ;; backup) echo tetron-backup.sh ;; esac; }
component_repo() { case "$1" in core) echo ErikAllanKincaid/tetron ;; webui) echo ErikAllanKincaid/tetron-webui ;; systray) echo ErikAllanKincaid/tetron-systray ;; backup) echo ErikAllanKincaid/tetron ;; esac; }
# All four components install to the same root-owned /usr/local/bin and
# need sudo -- webui/systray run as per-user services (systemd --user /
# launchd LaunchAgent) with no elevated runtime privilege of their own,
# but /usr/local/bin is the one location reliably on $PATH out of the box
# on every supported OS/shell, and the sudo prompt is already unavoidable
# for core, so splitting addons off into a separate unprivileged directory
# only trades a $PATH problem for a sudo prompt that costs nothing extra.
component_default_dir() { echo /usr/local/bin; }
component_needs_sudo() { echo 1; }

# Placing the binary in root-owned /usr/local/bin always needs sudo, but
# registering the service is a different privilege question: `core`
# registers a real root-owned systemd *system* service, but webui/systray
# register a `systemd --user` unit (or launchd LaunchAgent) that must run
# as the plain invoking user -- running it under sudo points `systemctl
# --user` at root's own (nonexistent, session-less) user manager instead
# of the invoking user's, which fails with a D-Bus/XDG_RUNTIME_DIR error.
component_service_needs_sudo() { case "$1" in core) echo 1 ;; *) echo 0 ;; esac; }

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

# $1 = component name (for messaging), $2 = 1 if this is an upgrade of an
# already-installed copy, 0 for a fresh install. Only an upgrade of `core`
# actually disrupts anything (drops live peers while the daemon restarts)
# -- a fresh install has nothing running yet, so it proceeds without
# --yes-core even when piped (no TTY). webui/systray/backup upgrades are
# not disruptive either (restarting a per-user service has no VPN/peer
# impact), so they get the same no-drama treatment as a fresh install
# rather than core's --yes-core gate.
confirm_sudo_install() {
	local comp="$1" is_upgrade="$2" reply
	[ "$YES_CORE" -eq 1 ] && return 0
	if [ "$is_upgrade" -eq 0 ] || [ "$comp" != "core" ]; then
		have_tty || return 0
		read -r -p "$([ "$is_upgrade" -eq 1 ] && echo Upgrade || echo Install) $comp now? This needs sudo. [Y/n] " reply < /dev/tty
		[[ "$reply" =~ ^[Nn]$ ]] && return 1
		return 0
	fi
	if ! have_tty; then
		log_warn "$comp needs --yes-core to upgrade non-interactively -- skipping $comp"
		return 1
	fi
	read -r -p "Upgrade $comp now? This needs sudo and briefly disconnects every peer on this host. [y/N] " reply < /dev/tty
	[[ "$reply" =~ ^[Yy]$ ]]
}

install_component() {
	local comp="$1" dest="$2" asset="$3" repo tmpdir sudo_prefix service_sudo_prefix
	repo="$(component_repo "$comp")"
	sudo_prefix=""
	[ "$(component_needs_sudo "$comp")" -eq 1 ] && sudo_prefix="sudo"
	service_sudo_prefix=""
	[ "$(component_service_needs_sudo "$comp")" -eq 1 ] && service_sudo_prefix="sudo"

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
	$service_sudo_prefix "$dest" install \
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
			confirm_sudo_install "$comp" 0 || continue
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

	asset="$(component_binary "$comp")-${PLATFORM}"
	if [ "$comp" = "core" ]; then
		reason="$(musl_reason "$dest")"
		if [ -n "$reason" ]; then
			asset="${asset}-musl"
			log_info "$comp: selecting musl asset ($reason)"
		fi
	fi

	status="$comp: installed=${current:-none} latest=$latest"
	if [ -n "$current" ] && [ "$current" = "$latest" ]; then
		log_pass "$status (up to date)"
		continue
	fi
	log_info "$status ($( [ -z "$current" ] && echo "not installed or version unknown" || echo "update available" ))"

	[ "$CHECK_ONLY" -eq 1 ] && continue

	if [ "$(component_needs_sudo "$comp")" -eq 1 ]; then
		is_upgrade=0
		[ -n "$current" ] && is_upgrade=1
		confirm_sudo_install "$comp" "$is_upgrade" || continue
	fi

	install_component "$comp" "$dest" "$asset" || any_failed=1
done

exit "$any_failed"
