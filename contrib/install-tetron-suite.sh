#!/usr/bin/env bash
# install-tetron-suite.sh: install or upgrade tetron and its add-ons from their
# GitHub releases in one pass. It is safe to run again at any time: anything
# already at the latest release is left as it is.
#
# Quick start (asks what to install when run at a terminal):
#   curl -fsSL https://raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/install-tetron-suite.sh | bash
# Show this help without installing anything:
#   curl -fsSL https://raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/install-tetron-suite.sh | bash -s -- --help
# Options go after `bash -s --` in the same way.
#
# Components:
#   core           tetron, the mesh VPN daemon and command line tool
#   webui          tetron-webui, a browser dashboard for this host
#   tui            tetron-tui, a terminal dashboard
#   systray        tetron-systray, a desktop tray icon
#   hosts          tetron-hosts, keeps /etc/hosts names for mesh peers
#   sync-receiver  tetron-sync-receiver, receives phone photo backups
#   messageboard   tetron-messageboard, a message board served on the mesh
#   backup         tetron-backup.sh, a script that backs up tetron's settings
#
# What this script does for each selected component, in order:
#   1. Asks GitHub (api.github.com) for the latest release of the component's
#      repository. All repositories are under github.com/ErikAllanKincaid/.
#   2. If that version is installed and its service is running, moves on.
#      Otherwise downloads the binary built for this OS and CPU, plus its
#      .sha256 checksum file, from that exact release over HTTPS. The binary
#      is not installed unless the checksum matches.
#   3. Copies the binary into /usr/local/bin. An existing copy elsewhere on
#      PATH, such as /usr/bin, is replaced where it is, unless a Debian
#      package owns it (see --method). sudo never writes into a home
#      directory. On Debian-family Linux the release .deb is installed with
#      apt-get instead when the release has one and the component is not
#      yet installed, is package-owned, or --method deb is given; the .deb
#      puts the binary in /usr/bin and ships the packaged service unit.
#   4. Runs the new binary's own `install` subcommand, which registers and
#      (re)starts its background service:
#        core, hosts                    system services, registered with sudo
#        webui, systray, sync-receiver  per-user services, registered as you
#      tui has no service. messageboard runs one board per tetron network, so
#      only the binary is placed; start boards from tetron-webui's Add-ons
#      panel or with `tetron-messageboard install --network <name>`.
#   5. Checks that the copy it installed is the one that runs: the first
#      copy on PATH and, for core, the binary its service starts. If another
#      copy runs instead, the component is reported as failed.
#   backup is a plain script rather than a release: it is fetched from the
#   tetron repository's main branch, with no checksum file, and copied to
#   /usr/local/bin. If it is already there it is left alone; delete it to
#   fetch a fresh copy.
#
# What to expect:
#   - sudo is used only to write to /usr/local/bin and to register the core
#     and hosts system services. Run as root, sudo is not used at all.
#   - Upgrading core restarts the tetron daemon, which briefly disconnects
#     this host from its mesh peers.
#   - Downloads go to a private temporary directory that is removed when the
#     script exits, including on Ctrl-C.
#   - If one component fails, the others still run. Failures, and anything
#     you declined, are listed at the end. The exit status is 1 on any
#     failure, 0 otherwise.
#   - Every component that is already installed is upgraded with the rest,
#     unless you decline it or use --no-core or --core-only. The add-ons talk
#     to the core daemon and need matching releases.
#
# Limits worth knowing:
#   - The checksum proves the download is intact and matches the release. It
#     does not protect against a compromised GitHub account or release.
#   - This script and backup come from the main branch, and binaries from the
#     latest release. Both change over time, so read the copy you are about
#     to run.
#   - GitHub allows 60 API requests per hour per IP address without login. If
#     the limit is reached, the affected components are reported as failed;
#     run the script again later.
#
# Choosing components:
#   With no --install-* or --core-only option, the script chooses:
#     - At a terminal, it shows a default set and asks "Use defaults? [Y/n]".
#       The default is core and tui, plus webui and systray when a graphical
#       display is found (always on macOS). Answering "n" asks about each
#       add-on in turn; hosts, sync-receiver, messageboard and backup default
#       to "no" unless they are already installed.
#     - With no terminal (cron, CI, a container started without -t), it
#       installs core and tui without asking.
#   Questions are read from the terminal (/dev/tty), so they work when the
#   script is piped into bash. `tetron-hosts install` asks its own questions
#   (which networks to sync, whether to schedule runs) the same way.
#
# Options:
#   --check                  show installed and latest versions; change
#                            nothing. Exits 1 if an up-to-date component's
#                            service is not running.
#   --core-only              install or upgrade core only, leaving any other
#                            installed components as they are. Cannot be
#                            combined with --install-* options; those already
#                            include core, so `--install-tui` alone installs
#                            core and tui.
#   --no-core                leave core as it is; install or upgrade add-ons
#                            only.
#   --install-webui          add webui
#   --install-tui            add tui
#   --install-systray        add systray
#   --install-hosts          add hosts
#   --install-sync-receiver  add sync-receiver
#   --install-messageboard   add messageboard
#   --install-backup         add backup
#   --install-all            add all of the above
#                            The --install-* options replace the "Use
#                            defaults?" question. core is included unless
#                            --no-core is given.
#   --musl                   use the statically linked (musl) build of core.
#                            Chosen automatically on Alpine Linux, and when
#                            the installed tetron is already a static build.
#   --method auto|binary|deb how to install each component. binary copies
#                            the raw release binary as described above.
#                            deb installs the release .deb with apt-get
#                            (Debian-family Linux only) and also converts a
#                            raw install to the package in one confirmed
#                            step: the old unit in /etc/systemd/system is
#                            disabled and moved to /var/backups/tetron/, the
#                            raw copy is removed, and the service restarts
#                            from the packaged binary. binary on a host where
#                            a package owns the component replaces the
#                            package in one confirmed step: the raw binary
#                            goes to /usr/local/bin, the package is removed
#                            (not purged), and the service restarts from the
#                            raw binary. It needs a terminal and refuses if
#                            removing the package would remove other
#                            packages. auto, the default, uses the .deb for a
#                            new install and for a package-owned component
#                            when the release has one, and keeps an existing
#                            raw install raw. A file a package owns is never
#                            overwritten: with no .deb in the release, auto
#                            skips that component with a warning. --check shows each
#                            component's origin (package:<name> or binary).
#                            A raw copy that comes first on PATH and hides a
#                            package-owned copy is reported on every run; at
#                            a terminal the script offers to remove the raw
#                            copy (default no).
#   --yes-core               accepted for compatibility; has no effect.
#   -h, --help               show this text.

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

SELF_URL=https://raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/install-tetron-suite.sh

# --help prints the comment block at the top of this file. When the script is
# piped into bash there is no file to read, so it is fetched again from
# SELF_URL, the same address the documented one-liner uses.
print_help() {
	local src="${BASH_SOURCE[0]:-}"
	{ if [ -r "$src" ]; then cat "$src"; else curl -fsSL "$SELF_URL"; fi; } \
		| awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }'
}

CHECK_ONLY=0
NO_CORE=0
MUSL_OVERRIDE=0
CORE_ONLY=0
INSTALL_WEBUI=0
INSTALL_TUI=0
INSTALL_SYSTRAY=0
INSTALL_HOSTS=0
INSTALL_SYNC_RECEIVER=0
INSTALL_MESSAGEBOARD=0
INSTALL_BACKUP=0
INSTALL_ALL=0
SELECTION_FLAG_GIVEN=0
METHOD=auto
EXPECT_METHOD=0

for arg in "$@"; do
	# `--method VALUE`: the value is the next argument.
	if [ "$EXPECT_METHOD" -eq 1 ]; then
		METHOD="$arg"
		EXPECT_METHOD=0
		continue
	fi
	case "$arg" in
	--method) EXPECT_METHOD=1 ;;
	--method=*) METHOD="${arg#--method=}" ;;
	--check) CHECK_ONLY=1 ;;
	# Accepted and ignored: core is upgraded by default.
	--yes-core) ;;
	--no-core) NO_CORE=1 ;;
	--musl) MUSL_OVERRIDE=1 ;;
	--core-only) CORE_ONLY=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-webui) INSTALL_WEBUI=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-tui) INSTALL_TUI=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-systray) INSTALL_SYSTRAY=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-hosts) INSTALL_HOSTS=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-sync-receiver) INSTALL_SYNC_RECEIVER=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-messageboard) INSTALL_MESSAGEBOARD=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-backup) INSTALL_BACKUP=1; SELECTION_FLAG_GIVEN=1 ;;
	--install-all) INSTALL_ALL=1; SELECTION_FLAG_GIVEN=1 ;;
	-h | --help)
		print_help
		exit 0
		;;
	*) fatal "unrecognized argument: $arg (see --help)" ;;
	esac
done

[ "$EXPECT_METHOD" -eq 1 ] && fatal "--method needs a value (auto, binary or deb)"
case "$METHOD" in
auto | binary | deb) ;;
*) fatal "--method must be auto, binary or deb (got: $METHOD)" ;;
esac

[ "$CORE_ONLY" -eq 1 ] && [ "$((INSTALL_WEBUI + INSTALL_TUI + INSTALL_SYSTRAY + INSTALL_HOSTS + INSTALL_SYNC_RECEIVER + INSTALL_MESSAGEBOARD + INSTALL_BACKUP + INSTALL_ALL))" -gt 0 ] \
	&& fatal "--core-only installs core alone and cannot be combined with --install-* options; those already include core, so run again without --core-only"

# --core-only and --no-core together would select nothing.
[ "$CORE_ONLY" -eq 1 ] && [ "$NO_CORE" -eq 1 ] \
	&& fatal "--core-only and --no-core are contradictory -- they select nothing"

# Writing to /usr/local/bin needs root. Run as root, the script uses no sudo;
# otherwise it needs sudo. Both this and the checksum tool are checked before
# anything is downloaded. --check changes nothing, so it needs neither.
SUDO=sudo
[ "$(id -u)" -eq 0 ] && SUDO=""
if [ "$CHECK_ONLY" -ne 1 ]; then
	[ -z "$SUDO" ] || command -v sudo >/dev/null 2>&1 \
		|| fatal "sudo is required to install into /usr/local/bin (or run this script as root)"
	command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 \
		|| fatal "sha256sum or shasum is required to verify downloads"
fi

# Where the packaged binaries live, where unit files that override packaged
# ones live, and where a replaced unit file is kept (PACKAGING-009). The
# TETRON_SUITE_* overrides exist for the hermetic tests only.
PKG_BIN_DIR="${TETRON_SUITE_PKG_BIN_DIR:-/usr/bin}"
UNIT_DIR="${TETRON_SUITE_UNIT_DIR:-/etc/systemd/system}"
BACKUP_DIR="${TETRON_SUITE_BACKUP_DIR:-/var/backups/tetron}"

# Downloads go to one private temporary directory, removed when the script
# exits for any reason, including Ctrl-C.
WORKDIR="$(mktemp -d)" || fatal "cannot create a temporary directory"
trap 'rm -rf "$WORKDIR"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# in_list <word> <space-separated list>: true if the word is in the list. A
# plain string rather than an array also works with the bash 3.2 that macOS
# ships.
in_list() {
	case " $2 " in *" $1 "*) return 0 ;; esac
	return 1
}

# dpkg_owner <path>: name of the Debian package that owns <path> (symlinks
# resolved), or nothing when no package owns it or dpkg is absent. Never
# fails. Used to decide whether a file may be overwritten (PACKAGING-007).
dpkg_owner() {
	local path="$1" real out
	command -v dpkg >/dev/null 2>&1 || return 0
	real="$(readlink -f "$path" 2>/dev/null)" || real=""
	[ -n "$real" ] || real="$path"
	out="$(dpkg -S -- "$real" 2>/dev/null)" || return 0
	printf '%s\n' "$out" | awk -F': ' '$0 !~ /^diversion by / && $2 ~ /^\// { n = split($1, a, ", "); split(a[1], b, ":"); print b[1]; exit }'
}

# component_origin <dest>: `package:<name>` when a package owns <dest>,
# `binary` when it is a file nothing owns, `none` when it does not exist.
component_origin() {
	local dest="$1" owner
	[ -e "$dest" ] || { echo none; return 0; }
	owner="$(dpkg_owner "$dest")"
	if [ -n "$owner" ]; then echo "package:$owner"; else echo binary; fi
}

# path_copies <binary>: every distinct executable <binary> along PATH, in PATH
# order. Entries that resolve to the same file (/bin -> /usr/bin, a symlink)
# count once.
path_copies() {
	local bin="$1" dir cand real seen=" " IFS=:
	for dir in $PATH; do
		[ -n "$dir" ] || continue
		cand="$dir/$bin"
		{ [ -f "$cand" ] && [ -x "$cand" ]; } || continue
		real="$(readlink -f "$cand" 2>/dev/null)" || real=""
		[ -n "$real" ] || real="$cand"
		case "$seen" in *" $real "*) continue ;; esac
		seen="$seen$real "
		printf '%s\n' "$cand"
	done
}

# shadowed_package_copy <binary>: when the first copy of <binary> on PATH is a
# raw file and a later copy is package-owned, print
# `<raw path>|<package copy>|<package>` -- the raw one runs and hides the
# package. Prints nothing when the first copy is packaged, when there is only
# one copy, or when no later copy is package-owned (PACKAGING-008).
shadowed_package_copy() {
	local bin="$1" first="" copy owner
	while IFS= read -r copy; do
		if [ -z "$first" ]; then
			first="$copy"
			[ -z "$(dpkg_owner "$first")" ] || return 0
			continue
		fi
		owner="$(dpkg_owner "$copy")"
		if [ -n "$owner" ]; then
			printf '%s|%s|%s\n' "$first" "$copy" "$owner"
			return 0
		fi
	done < <(path_copies "$bin")
	return 0
}

# offer_remove_shadow <comp> <raw path>: ask before deleting the raw copy that
# shadows a package. The default is no, and with no terminal it never deletes
# anything (PACKAGING-008). Returns 0 only if the copy was removed.
offer_remove_shadow() {
	local comp="$1" raw="$2" reply
	have_tty || return 1
	read -r -p "Remove the raw copy $raw so the packaged $comp runs instead? This needs sudo. [y/N] " reply < /dev/tty
	[[ "$reply" =~ ^[Yy]$ ]] || return 1
	$SUDO rm -f -- "$raw" || { log_warn "$comp: could not remove $raw"; return 1; }
	log_pass "$comp: removed $raw"
}

# choose_method <comp> <dest> <have_deb 0|1>: what to do for this component,
# from $METHOD, who owns the installed copy, and whether the latest release
# has a .deb for it. Prints one of: deb, binary, replace-package (the
# installed copy is package-owned and --method binary asks for the raw
# install, PACKAGING-012), skip-owned (auto, package-owned, no .deb to
# upgrade it with), error-no-deb (--method deb but no .deb published). Never
# picks an action that overwrites a package-owned file (PACKAGING-007). auto
# on a fresh host takes the .deb when there is one (PACKAGING-011); the
# caller sets have_deb only where dpkg and apt-get exist.
choose_method() {
	local dest="$2" have_deb="$3" owner
	owner="$(dpkg_owner "$dest")"
	case "$METHOD" in
	binary)
		if [ -n "$owner" ]; then echo replace-package; else echo binary; fi
		;;
	deb)
		if [ "$have_deb" -eq 1 ]; then echo deb; else echo error-no-deb; fi
		;;
	*)
		if [ -z "$owner" ]; then
			if [ ! -e "$dest" ] && [ "$have_deb" -eq 1 ]; then echo deb; else echo binary; fi
		elif [ "$have_deb" -eq 1 ]; then
			echo deb
		else
			echo skip-owned
		fi
		;;
	esac
}

# deb_arch_for <uname -m style arch>: the Debian architecture name, or nothing.
deb_arch_for() {
	case "$1" in
	x86_64 | amd64) echo amd64 ;;
	aarch64 | arm64) echo arm64 ;;
	esac
}

# deb_asset_from_json <package> <deb arch>: read a GitHub release JSON on
# stdin, print the name of its `<package>_<version>-<rev>_<arch>.deb` asset
# (never the .sha256 sibling), or nothing. Matching on the asset list means a
# packaging-only revision bump (-2) is found without guessing.
deb_asset_from_json() {
	local pkg="$1" debarch="$2"
	grep -o "\"name\": *\"${pkg}_[^\"]*_${debarch}\.deb\"" | sed -E 's/.*"([^"]+)"$/\1/' | head -1
}

# deb_supported: true where the .deb method can work -- Linux with dpkg and
# apt-get. Defined here, ahead of the platform detection that calls it.
deb_supported() {
	[ "${plat_os:-}" = linux ] && command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1
}

# True when a graphical display is available. Decides whether webui and
# systray are in the default set.
host_has_display() {
	# macOS always counts as having a display.
	[ "$(uname -s)" = Darwin ] && return 0
	[ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]
}

# True when a terminal can be reached through /dev/tty. This works even when
# the script itself arrives on stdin through `curl | bash`, and is false under
# cron, CI, or a container without a terminal.
have_tty() {
	{ : < /dev/tty; } 2>/dev/null
}

# Binary name of each component.
component_binary() { case "$1" in core) echo tetron ;; webui) echo tetron-webui ;; tui) echo tetron-tui ;; systray) echo tetron-systray ;; hosts) echo tetron-hosts ;; sync-receiver) echo tetron-sync-receiver ;; messageboard) echo tetron-messageboard ;; backup) echo tetron-backup.sh ;; esac; }

# True when the component is already on this host: on PATH, or where older
# installers put add-ons (~/.local/bin, the macOS TetronSystray app). Such
# copies are upgraded into /usr/local/bin.
component_installed() {
	command -v "$(component_binary "$1")" >/dev/null 2>&1 && return 0
	[ -x "$HOME/.local/bin/$(component_binary "$1")" ] && return 0
	[ "$1" = systray ] && [ -x "$HOME/Applications/TetronSystray.app/Contents/MacOS/tetron-systray" ] && return 0
	return 1
}

# Add-ons you answered "no" to. They are not upgraded, even if installed.
DECLINED=""

# Components are installed in list order. tui comes before systray because
# the tray adds its "Open tui" menu item only if tetron-tui is already there.
COMPONENTS=(core)
if [ "$INSTALL_ALL" -eq 1 ]; then
	COMPONENTS=(core webui tui systray hosts sync-receiver messageboard backup)
elif [ "$SELECTION_FLAG_GIVEN" -eq 1 ]; then
	[ "$CORE_ONLY" -eq 1 ] || {
		[ "$INSTALL_WEBUI" -eq 1 ] && COMPONENTS+=(webui)
		[ "$INSTALL_TUI" -eq 1 ] && COMPONENTS+=(tui)
		[ "$INSTALL_SYSTRAY" -eq 1 ] && COMPONENTS+=(systray)
		[ "$INSTALL_HOSTS" -eq 1 ] && COMPONENTS+=(hosts)
		[ "$INSTALL_SYNC_RECEIVER" -eq 1 ] && COMPONENTS+=(sync-receiver)
		[ "$INSTALL_MESSAGEBOARD" -eq 1 ] && COMPONENTS+=(messageboard)
		[ "$INSTALL_BACKUP" -eq 1 ] && COMPONENTS+=(backup)
	}
elif [ "$CHECK_ONLY" -eq 1 ]; then
	# --check reports on the default set without asking.
	COMPONENTS=(core tui)
	host_has_display && COMPONENTS=(core webui tui systray)
elif ! have_tty; then
	# No terminal to ask at: install core and tui.
	COMPONENTS=(core tui)
	log_info "no controlling terminal detected -- defaulting to core + tui (pass --install-webui/--install-systray/--install-hosts/--install-sync-receiver/--install-messageboard/--install-backup/--install-all, or run this script with a terminal attached, to include more addons)"
else
	if host_has_display; then
		DEFAULT_COMPONENTS=(core webui tui systray)
		log_info "display detected"
	else
		# tui runs in a terminal, so it is a default even without a display.
		DEFAULT_COMPONENTS=(core tui)
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
		tui_default=1
		# An add-on that is already installed defaults to "yes": this is an
		# upgrade.
		component_installed webui && webui_default=1
		component_installed systray && systray_default=1
		ask_addon webui "$webui_default" && COMPONENTS+=(webui) || DECLINED="$DECLINED webui"
		ask_addon tui "$tui_default" && COMPONENTS+=(tui) || DECLINED="$DECLINED tui"
		ask_addon systray "$systray_default" && COMPONENTS+=(systray) || DECLINED="$DECLINED systray"
		# These default to "no": each adds something not every host needs.
		# hosts registers a system service that edits /etc/hosts.
		hosts_default=0; sync_receiver_default=0; messageboard_default=0; backup_default=0
		component_installed hosts && hosts_default=1
		component_installed sync-receiver && sync_receiver_default=1
		component_installed messageboard && messageboard_default=1
		component_installed backup && backup_default=1
		ask_addon hosts "$hosts_default" && COMPONENTS+=(hosts) || DECLINED="$DECLINED hosts"
		ask_addon sync-receiver "$sync_receiver_default" && COMPONENTS+=(sync-receiver) || DECLINED="$DECLINED sync-receiver"
		ask_addon messageboard "$messageboard_default" && COMPONENTS+=(messageboard) || DECLINED="$DECLINED messageboard"
		ask_addon backup "$backup_default" && COMPONENTS+=(backup) || DECLINED="$DECLINED backup"
	fi
fi

# Add every component already installed, so it is upgraded with the rest: the
# add-ons talk to the core daemon and need matching releases. --check reports
# them too.
if [ "$CORE_ONLY" -ne 1 ]; then
	for comp in core webui tui systray hosts sync-receiver messageboard backup; do
		component_installed "$comp" || continue
		in_list "$comp" "$DECLINED" && continue
		in_list "$comp" "${COMPONENTS[*]}" && continue
		COMPONENTS+=("$comp")
		log_info "$comp: already installed -- including it so it is not left stale"
	done
fi

# --no-core removes core, including when the step above added it.
if [ "$NO_CORE" -eq 1 ]; then
	KEPT=()
	for comp in "${COMPONENTS[@]}"; do
		[ "$comp" = "core" ] || KEPT+=("$comp")
	done
	if [ "${#KEPT[@]}" -eq 0 ]; then
		log_warn "--no-core leaves nothing to do (no addons selected or installed)"
		exit 0
	fi
	COMPONENTS=("${KEPT[@]}")
	log_info "--no-core: skipping core"
fi

# --- platform detection: release files are named <binary>-<os>-<cpu> ---
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
DEB_ARCH="$(deb_arch_for "$plat_arch")"
if [ "$METHOD" = deb ] && ! deb_supported; then
	fatal "--method deb needs Linux with dpkg and apt-get (Debian-family); this host has neither"
fi

# Alpine Linux uses musl instead of glibc, so it needs the static (musl) build
# of core. Detected from /etc/os-release.
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

# True if the binary at $1 is statically linked, so a static (musl) install of
# core stays static when upgraded.
dest_is_static() {
	local dest="$1"
	[ -x "$dest" ] || return 1
	if command -v ldd >/dev/null 2>&1; then
		ldd "$dest" 2>&1 | grep -q "not a dynamic executable"
	else
		command -v file >/dev/null 2>&1 && file "$dest" 2>/dev/null | grep -q "statically linked"
	fi
}

# Prints why core should use the musl build, or nothing. Only core has a musl
# build.
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

# --- per-component settings ---
# GitHub repository of each component.
component_repo() { case "$1" in core) echo ErikAllanKincaid/tetron ;; webui) echo ErikAllanKincaid/tetron-webui ;; tui) echo ErikAllanKincaid/tetron-tui ;; systray) echo ErikAllanKincaid/tetron-systray ;; hosts) echo ErikAllanKincaid/tetron-hosts ;; sync-receiver) echo ErikAllanKincaid/tetron-sync-receiver ;; messageboard) echo ErikAllanKincaid/tetron-messageboard ;; backup) echo ErikAllanKincaid/tetron ;; esac; }
# Install location. /usr/local/bin is on PATH by default on Linux and macOS;
# writing to it needs root.
component_default_dir() { echo "${TETRON_SUITE_RAW_BIN_DIR:-/usr/local/bin}"; }

# Whether registering the component's service needs root. core and hosts are
# system services (hosts edits /etc/hosts). The others are per-user services,
# which must be registered as you, not as root.
component_service_needs_sudo() { case "$1" in core | hosts) echo 1 ;; *) echo 0 ;; esac; }

# Components whose `install` this script does not run, so an up-to-date
# binary means nothing more to do. messageboard runs one board per tetron
# network, started from tetron-webui's Add-ons panel or with
# `tetron-messageboard install --network <name>`. tui has no service.
component_binary_only() { case "$1" in messageboard | tui) echo 1 ;; *) echo 0 ;; esac; }

# Components with no service at all: the binary is copied and nothing is run.
component_no_service() { case "$1" in tui) echo 1 ;; *) echo 0 ;; esac; }

# systemd unit used to check whether a component is running. For hosts this is
# its timer, which stays active between scheduled runs.
component_unit() { case "$1" in core) echo tetron.service ;; hosts) echo tetron-hosts.timer ;; *) component_binary "$1" ;; esac; }

# True if the component's service is running. When the binary is already up
# to date but its service is not running, the service is registered and
# started again.
component_service_active() {
	local comp="$1" unit
	unit="$(component_unit "$comp")"
	case "$plat_os" in
	linux)
		if [ "$(component_service_needs_sudo "$comp")" -eq 1 ]; then
			systemctl is-active --quiet "$unit" 2>/dev/null
		else
			systemctl --user is-active --quiet "$unit" 2>/dev/null
		fi
		;;
	macos)
		# launchd job names are com.tetron.<component>, except core, which is
		# com.tetron.vpn.
		local label
		case "$comp" in
		core) label="com.tetron.vpn" ;;
		*) label="com.tetron.$comp" ;;
		esac
		launchctl list "$label" >/dev/null 2>&1
		;;
	*)
		false
		;;
	esac
}

# Where to install. An existing copy found on PATH outside the home directory
# (for example /usr/bin) is upgraded in place. A copy inside $HOME is not:
# /usr/local/bin is used instead, so sudo never writes a root-owned file into a
# home directory.
component_dest() {
	local bin path
	bin="$(component_binary "$1")"
	path="$(command -v "$bin" 2>/dev/null)"
	case "$path" in
	"" | "$HOME"/*) echo "$(component_default_dir)/$bin" ;;
	*) echo "$path" ;;
	esac
}

# Warn when PATH would still run a different copy than the one just installed.
warn_if_shadowed() {
	local comp="$1" dest="$2" found
	hash -r
	found="$(command -v "$(component_binary "$comp")" 2>/dev/null)"
	[ -n "$found" ] && [ "$found" != "$dest" ] \
		&& log_warn "$comp: $found comes first on PATH and runs instead of $dest -- remove it: rm '$found'"
	return 0
}

# One GitHub API request per component: the JSON is fetched once and both the
# tag (tag_from_json) and the .deb asset (deb_asset_from_json) are read from
# it, so the unauthenticated rate limit is not doubled by --method.
release_json() {
	curl -fsSL "https://api.github.com/repos/$1/releases/latest"
}

# Reads a release JSON on stdin and prints its tag (e.g. "v0.14.3").
tag_from_json() {
	local tag
	tag="$(grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
	[ -n "$tag" ] || return 1
	printf '%s\n' "$tag"
}

# Runs an installed binary with a version option and prints its output.
# `setsid` keeps the binary from using the terminal and `timeout` stops it
# after 5 seconds, so a misbehaving binary cannot stall the installer. Both
# are skipped where unavailable (for example on macOS).
probe_version() {
	local dest="$1" arg="$2"
	if command -v setsid >/dev/null 2>&1 && command -v timeout >/dev/null 2>&1; then
		setsid timeout 5 "$dest" "$arg" </dev/null 2>/dev/null
	elif command -v timeout >/dev/null 2>&1; then
		timeout 5 "$dest" "$arg" </dev/null 2>/dev/null
	else
		"$dest" "$arg" </dev/null 2>/dev/null
	fi
}

# Prints the installed version (e.g. "0.14.3"), or nothing if the binary is
# missing or does not report one, in which case it is installed.
installed_version() {
	local dest="$1" out
	[ -x "$dest" ] || return 0
	out="$(probe_version "$dest" --version)" || out="$(probe_version "$dest" version)" || true
	# Version output looks like "tetron 0.14.3 (abc1234)"; print the second word.
	printf '%s\n' "$out" | awk 'NF>=2{print $2}'
}

# True if the file matches the SHA-256 checksum in its .sha256 file.
verify_checksum() {
	local file="$1" sumfile="$2" dir sumfile_name
	dir="$(dirname "$file")"
	sumfile_name="$(basename "$sumfile")"
	if command -v sha256sum >/dev/null 2>&1; then
		(cd "$dir" && sha256sum -c "$sumfile_name") >/dev/null
	else
		(cd "$dir" && shasum -a 256 -c "$sumfile_name") >/dev/null
	fi
}

# Asks before installing or upgrading a component. Enter means yes; "n" skips
# it. Without a terminal it proceeds. Upgrading core has its own question that
# states the cost: the daemon restarts and peers disconnect briefly.
# $1 = component, $2 = 1 for an upgrade, 0 for a new install.
confirm_sudo_install() {
	local comp="$1" is_upgrade="$2" reply
	if [ "$is_upgrade" -eq 0 ] || [ "$comp" != "core" ]; then
		have_tty || return 0
		read -r -p "$([ "$is_upgrade" -eq 1 ] && echo Upgrade || echo Install) $comp now? This needs sudo. [Y/n] " reply < /dev/tty
		[[ "$reply" =~ ^[Nn]$ ]] && return 1
		return 0
	fi
	# Upgrading core. --no-core skips this entirely.
	if ! have_tty; then
		log_warn "$comp: upgrading without a terminal to confirm at -- every peer on this host drops briefly while the daemon restarts (pass --no-core to skip core)"
		return 0
	fi
	read -r -p "Upgrade $comp now? This needs sudo and briefly disconnects every peer on this host. [Y/n] " reply < /dev/tty
	[[ "$reply" =~ ^[Nn]$ ]] && return 1
	return 0
}

# Downloads one component from release $4, verifies it, installs it to $2,
# and starts its service. With a 5th argument, that package is removed after
# the raw copy is in place and before the service restarts (PACKAGING-012).
install_component() {
	local comp="$1" dest="$2" asset="$3" tag="$4" remove_pkg="${5:-}" repo tmpdir sudo_prefix
	repo="$(component_repo "$comp")"
	sudo_prefix="$SUDO"

	tmpdir="$WORKDIR/$comp"
	mkdir -p "$tmpdir"

	log_info "$comp: downloading $asset from $repo release $tag..."
	curl -fsSL -o "$tmpdir/$asset" "https://github.com/$repo/releases/download/$tag/$asset" \
		|| { log_error "$comp: failed to download $asset"; return 1; }
	curl -fsSL -o "$tmpdir/$asset.sha256" "https://github.com/$repo/releases/download/$tag/$asset.sha256" \
		|| { log_error "$comp: failed to download $asset.sha256"; return 1; }
	verify_checksum "$tmpdir/$asset" "$tmpdir/$asset.sha256" \
		|| { log_error "$comp: checksum verification failed, not installed"; return 1; }

	log_info "$comp: installing to $dest..."
	$sudo_prefix mkdir -p "$(dirname "$dest")"
	$sudo_prefix install -m 0755 "$tmpdir/$asset" "$dest" \
		|| { log_error "$comp: failed to install binary to $dest"; return 1; }

	if [ -n "$remove_pkg" ]; then
		# Disable the packaged unit first so its enable link goes with it; the
		# daemon keeps running until the raw binary's `install` restarts it.
		if component_has_system_unit "$comp"; then
			$SUDO systemctl disable "$(component_unit "$comp")" >/dev/null 2>&1 \
				|| log_warn "$comp: could not disable $(component_unit "$comp") (continuing)"
		fi
		log_info "$comp: removing package $remove_pkg with apt-get (remove, not purge)..."
		$SUDO env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$remove_pkg" \
			|| { log_error "$comp: apt-get could not remove $remove_pkg -- the raw copy $dest is in place but the service still runs the package"; return 1; }
		hash -r
	fi

	post_install_component "$comp" "$dest"
}

# Steps after a component's binary is in place, shared by the raw-binary and
# the .deb install paths: register/restart its service (or just report, for
# binary-only components), then report the version at $dest.
post_install_component() {
	local comp="$1" dest="$2" service_sudo_prefix ver
	service_sudo_prefix=""
	[ "$(component_service_needs_sudo "$comp")" -eq 1 ] && service_sudo_prefix="$SUDO"

	if [ "$(component_no_service "$comp")" -eq 1 ]; then
		# A plain binary (tetron-tui): placed on PATH, nothing to start.
		log_info "$comp: binary placed at $dest -- run '$(component_binary "$comp")'"
	elif [ "$(component_binary_only "$comp")" -eq 1 ]; then
		log_info "$comp: binary placed -- start a board per network from tetron-webui's Add-ons panel (or '$dest install --network <name>')"
		# Restart boards already running so they use the new binary. Does
		# nothing when there are none.
		$service_sudo_prefix "$dest" restart-all >/dev/null 2>&1 || true
	else
		log_info "$comp: running '$dest install' to register/restart its service..."
		$service_sudo_prefix "$dest" install \
			|| { log_error "$comp: '$dest install' failed -- binary is updated but its service was not restarted, check logs"; return 1; }
	fi

	verify_runs_installed "$comp" "$dest" || return 1
	log_pass "$comp: installed/upgraded successfully"
	ver="$(installed_version "$dest")"
	if [ -n "$ver" ]; then log_pass "$comp: version $ver"; fi
}

# resolved <path>: <path> with symlinks resolved, or <path> itself.
resolved() {
	local r
	r="$(readlink -f "$1" 2>/dev/null)" || r=""
	printf '%s\n' "${r:-$1}"
}

# True when the component runs as a system unit (core) on Linux.
component_has_system_unit() {
	[ "${plat_os:-}" = linux ] \
		&& [ "$(component_service_needs_sudo "$1")" -eq 1 ] \
		&& [ "$(component_no_service "$1")" -eq 0 ] \
		&& [ "$(component_binary_only "$1")" -eq 0 ]
}

# unit_exec_path <unit>: the program systemd starts for <unit>'s ExecStart, or
# nothing (no systemctl, no such unit, a timer).
unit_exec_path() {
	command -v systemctl >/dev/null 2>&1 || return 0
	systemctl show -p ExecStart --value "$1" 2>/dev/null | sed -n 's/.*path=\([^ ;]*\).*/\1/p' | head -1
}

# verify_runs_installed <comp> <dest>: true when what will run is <dest>: the
# first copy on PATH and, for a system unit, the unit's ExecStart. Otherwise
# names what runs instead, with its version, and fails (PACKAGING-010).
verify_runs_installed() {
	local comp="$1" dest="$2" found exec unit
	hash -r
	found="$(command -v "$(component_binary "$comp")" 2>/dev/null)"
	if [ -n "$found" ] && [ "$(resolved "$found")" != "$(resolved "$dest")" ]; then
		log_fail "$comp: $found comes first on PATH and runs instead of $dest (it is version $(installed_version "$found" | grep . || echo unknown)) -- remove it: rm '$found'"
		return 1
	fi
	if component_has_system_unit "$comp"; then
		unit="$(component_unit "$comp")"
		exec="$(unit_exec_path "$unit")"
		if [ -n "$exec" ] && [ "$(resolved "$exec")" != "$(resolved "$dest")" ]; then
			log_fail "$comp: its service $unit runs $exec (version $(installed_version "$exec" | grep . || echo unknown)), not $dest"
			return 1
		fi
	fi
	return 0
}

# raw_unit_to_migrate <comp> <raw path>: the unit file in $UNIT_DIR that no
# package owns and whose ExecStart runs <raw path>, or nothing (PACKAGING-009).
raw_unit_to_migrate() {
	local comp="$1" raw="$2" ufile
	component_has_system_unit "$comp" || return 0
	ufile="$UNIT_DIR/$(component_unit "$comp")"
	[ -f "$ufile" ] || return 0
	[ -z "$(dpkg_owner "$ufile")" ] || return 0
	awk -v p="$raw" '/^ExecStart=/ { sub(/^ExecStart=[-@:+!]*/, ""); split($0, a, " "); if (a[1] == p) f = 1 } END { exit !f }' "$ufile" \
		&& printf '%s\n' "$ufile"
	return 0
}

# migrate_raw_install <comp> <raw path>: after the package is installed, move
# the unit that runs the raw copy out of the way, then remove the raw copy
# (PACKAGING-009 steps 3 and 4). The daemon keeps running until the packaged
# binary's `install` restarts it.
migrate_raw_install() {
	local comp="$1" raw="$2" ufile unit backup
	ufile="$(raw_unit_to_migrate "$comp" "$raw")"
	if [ -n "$ufile" ]; then
		unit="$(basename "$ufile")"
		backup="$BACKUP_DIR/$unit.$(date +%Y%m%d%H%M%S)"
		$SUDO systemctl disable "$unit" >/dev/null 2>&1 \
			|| log_warn "$comp: could not disable $unit (continuing)"
		{ $SUDO mkdir -p "$BACKUP_DIR" && $SUDO mv -- "$ufile" "$backup"; } \
			|| { log_error "$comp: could not move $ufile -- the raw copy $raw is left in place"; return 1; }
		log_info "$comp: moved $ufile to $backup"
		$SUDO systemctl daemon-reload \
			|| log_warn "$comp: systemctl daemon-reload failed (continuing)"
	fi
	if [ -z "$(dpkg_owner "$raw")" ] && [ "$(resolved "$raw")" != "$(resolved "$PKG_BIN_DIR/$(component_binary "$comp")")" ]; then
		$SUDO rm -f -- "$raw" || { log_error "$comp: could not remove the raw copy $raw"; return 1; }
		log_info "$comp: removed the raw copy $raw"
	fi
	hash -r
}

# replace_guard <comp> <pkg>: PACKAGING-012 guards. Fails, printing the manual
# commands, when other packages would be removed with <pkg> or when there is
# no terminal to confirm at.
replace_guard() {
	local comp="$1" pkg="$2" sim others
	if ! sim="$(apt-get -s remove "$pkg" 2>/dev/null)"; then
		log_error "$comp: could not simulate 'apt-get remove $pkg'; not replacing the package"
		return 1
	fi
	others="$(printf '%s\n' "$sim" | awk -v p="$pkg" '$1 == "Remv" && $2 != p { printf "%s ", $2 }')"
	if [ -n "$others" ]; then
		log_error "$comp: removing package $pkg would also remove: ${others% } -- not replacing it. To do it by hand: sudo apt-get remove $pkg, then rerun with --method binary"
		return 1
	fi
	if ! have_tty; then
		log_error "$comp: replacing package $pkg with the raw binary needs a terminal to confirm at. To do it by hand: sudo apt-get remove $pkg, then rerun with --method binary"
		return 1
	fi
}

# print_replace_plan <comp> <pkg> <raw dest> <asset>: what --method binary
# will do to replace the package (PACKAGING-012).
print_replace_plan() {
	local comp="$1" pkg="$2" dest="$3" asset="$4"
	log_info "$comp: replacing package $pkg with the raw binary:"
	log_info "  1. download and verify $asset, install it to $dest"
	log_info "  2. sudo apt-get remove $pkg (not purge: the node identity and settings stay)"
	[ "$(component_no_service "$comp")" -eq 1 ] && return 0
	log_info "  3. run '$dest install' to register the service for the raw binary and restart it"
}

# print_conversion_plan <comp> <raw path> <deb asset>: what --method deb will
# do to convert a raw install, shown before the one question (PACKAGING-009).
print_conversion_plan() {
	local comp="$1" raw="$2" asset="$3" ufile n=1
	ufile="$(raw_unit_to_migrate "$comp" "$raw")"
	log_info "$comp: converting the raw install at $raw to the package:"
	log_info "  $n. install $asset with apt-get"; n=$((n + 1))
	if [ -n "$ufile" ]; then
		log_info "  $n. disable $ufile (it runs $raw) and move it to $BACKUP_DIR/"; n=$((n + 1))
	fi
	log_info "  $n. remove $raw"; n=$((n + 1))
	[ "$(component_no_service "$comp")" -eq 1 ] && return 0
	log_info "  $n. run '$PKG_BIN_DIR/$(component_binary "$comp") install' to start the service from the packaged binary"
}

# confirm_conversion <comp>: the one question for a conversion. Without a
# terminal it proceeds, since --method deb asked for it.
confirm_conversion() {
	local comp="$1" reply cost=""
	[ "$comp" = core ] && cost=" and briefly disconnects every peer on this host"
	if ! have_tty; then
		log_warn "$comp: converting without a terminal to confirm at (--method deb was given)"
		return 0
	fi
	read -r -p "Convert $comp to the package as listed above? This needs sudo$cost. [Y/n] " reply < /dev/tty
	[[ "$reply" =~ ^[Nn]$ ]] && return 1
	return 0
}

# Installs a component from asset $2 of release $3, a .deb, with apt-get
# (PACKAGING-007). The package puts the binary in /usr/bin; the component's
# own `install` subcommand runs on that path, not on whichever copy is first
# on PATH.
# With a 4th argument, the raw copy at that path is migrated away after the
# package is installed (PACKAGING-009).
install_component_deb() {
	local comp="$1" asset="$2" tag="$3" raw="${4:-}" repo tmpdir dest
	repo="$(component_repo "$comp")"
	dest="$PKG_BIN_DIR/$(component_binary "$comp")"

	tmpdir="$WORKDIR/$comp"
	mkdir -p "$tmpdir"

	log_info "$comp: downloading $asset from $repo release $tag..."
	curl -fsSL -o "$tmpdir/$asset" "https://github.com/$repo/releases/download/$tag/$asset" \
		|| { log_error "$comp: failed to download $asset"; return 1; }
	curl -fsSL -o "$tmpdir/$asset.sha256" "https://github.com/$repo/releases/download/$tag/$asset.sha256" \
		|| { log_error "$comp: failed to download $asset.sha256"; return 1; }
	verify_checksum "$tmpdir/$asset" "$tmpdir/$asset.sha256" \
		|| { log_error "$comp: checksum verification failed, not installed"; return 1; }

	log_info "$comp: installing $asset with apt-get..."
	$SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y "$tmpdir/$asset" \
		|| { log_error "$comp: apt-get failed to install $asset"; return 1; }

	if [ -n "$raw" ]; then
		migrate_raw_install "$comp" "$raw" || return 1
	fi

	post_install_component "$comp" "$dest"
}

# backup is a plain script from the tetron repository's main branch. It has no
# release and no checksum file. It is copied next to the core binary.
install_backup() {
	local comp="$1" dest="$2" bin repo url tmpdir sudo_prefix
	bin="$(component_binary "$comp")"
	repo="$(component_repo "$comp")"
	url="https://raw.githubusercontent.com/$repo/main/contrib/$bin"
	sudo_prefix="$SUDO"

	tmpdir="$WORKDIR/$comp"
	mkdir -p "$tmpdir"

	log_info "$comp: downloading $url ..."
	curl -fsSL -o "$tmpdir/$bin" "$url" \
		|| { log_error "$comp: failed to download $bin"; return 1; }
	log_info "$comp: installing to $dest..."
	$sudo_prefix mkdir -p "$(dirname "$dest")"
	$sudo_prefix install -m 0755 "$tmpdir/$bin" "$dest" \
		|| { log_error "$comp: failed to install $bin to $dest"; return 1; }

	log_pass "$comp: installed/upgraded successfully"
	warn_if_shadowed "$comp" "$dest"
}

any_failed=0
# Components that failed. One failure does not stop the others; all are
# listed in the closing summary and the script exits 1.
failed=""
# Components you declined at a prompt. Listed in the closing summary so a
# skip is never silent.
skipped=""
for comp in "${COMPONENTS[@]}"; do
	dest="$(component_dest "$comp")"

	# backup has no version to compare: if present it is left alone,
	# otherwise it is installed.
	if [ "$comp" = "backup" ]; then
		if [ -x "$dest" ]; then
			log_pass "$comp: installed ($dest)"
		else
			log_info "$comp: not installed"
			[ "$CHECK_ONLY" -eq 1 ] && continue
			confirm_sudo_install "$comp" 0 || { skipped="$skipped $comp"; continue; }
			install_backup "$comp" "$dest" || failed="$failed $comp"
		fi
		continue
	fi

	current="$(installed_version "$dest")"
	release="$(release_json "$(component_repo "$comp")")" \
		&& tag="$(printf '%s' "$release" | tag_from_json)" || {
		log_error "$comp: failed to query latest release -- skipping"
		failed="$failed $comp"
		continue
	}
	latest="${tag#v}"

	# Install method (PACKAGING-007): look for a .deb asset in the release JSON
	# already fetched, then let choose_method pick deb, binary, or a safe
	# alternative that never overwrites a package-owned file.
	have_deb=0
	deb_asset=""
	if [ "$METHOD" != binary ] && [ -n "$DEB_ARCH" ] && deb_supported; then
		deb_asset="$(printf '%s' "$release" | deb_asset_from_json "$(component_binary "$comp")" "$DEB_ARCH")"
		[ -n "$deb_asset" ] && have_deb=1
	fi
	method="$(choose_method "$comp" "$dest" "$have_deb")"
	origin="$(component_origin "$dest")"
	# A raw copy first on PATH hides a package copy: report it every run
	# (--check included); removal is only ever offered at install time.
	shadow="$(shadowed_package_copy "$(component_binary "$comp")")"
	shadow_raw=""
	if [ -n "$shadow" ]; then
		IFS='|' read -r shadow_raw shadow_pkgcopy shadow_pkg <<<"$shadow"
		log_warn "$comp: $shadow_raw (raw binary) comes first on PATH and shadows $shadow_pkgcopy from package $shadow_pkg"
	fi
	# --method deb also converts a raw install to the package, even at the same version.
	convert_to_deb=0
	[ "$METHOD" = deb ] && [ "$method" = deb ] && [ "${origin#package:}" = "$origin" ] && convert_to_deb=1
	# A raw copy to convert: migrated away in the same step (PACKAGING-009).
	conv_raw=""
	[ "$convert_to_deb" -eq 1 ] && [ "$origin" = binary ] && conv_raw="$dest"

	asset="$(component_binary "$comp")-${PLATFORM}"
	if [ "$comp" = "core" ]; then
		reason="$(musl_reason "$dest")"
		if [ -n "$reason" ]; then
			asset="${asset}-musl"
			log_info "$comp: selecting musl asset ($reason)"
		fi
	fi

	status="$comp: installed=${current:-none} latest=$latest origin=$origin"
	if [ -n "$current" ] && [ "$current" = "$latest" ] && [ "$convert_to_deb" -eq 0 ] && [ "$method" != replace-package ]; then
		# messageboard and tui: an up-to-date binary is all there is to check.
		if [ "$(component_binary_only "$comp")" -eq 1 ]; then
			log_pass "$status (up to date)"
			continue
		fi
		if component_service_active "$comp"; then
			log_pass "$status (up to date)"
			continue
		fi
		# Up to date, but the service is not running: register and start it
		# again with `install`, without downloading anything.
		log_info "$status, but its service is not active -- registering/starting it"
		if [ "$CHECK_ONLY" -eq 1 ]; then
			any_failed=1
			continue
		fi
		service_sudo_prefix=""
		[ "$(component_service_needs_sudo "$comp")" -eq 1 ] && service_sudo_prefix="$SUDO"
		if $service_sudo_prefix "$dest" install; then
			log_pass "$comp: service registered/started"
		else
			log_error "$comp: '$dest install' failed to register/start its service"
			failed="$failed $comp"
		fi
		continue
	fi
	if [ -z "$current" ]; then
		note="not installed or version unknown"
	elif [ "$current" = "$latest" ] && [ "$convert_to_deb" -eq 1 ]; then
		note="same version; converting to the package"
	elif [ "$current" = "$latest" ] && [ "$method" = replace-package ]; then
		note="same version; replacing the package"
	else
		note="update available"
	fi
	log_info "$status ($note)"

	[ "$CHECK_ONLY" -eq 1 ] && continue

	case "$method" in
	error-no-deb)
		log_error "$comp: --method deb requested but release $latest has no .deb for $DEB_ARCH"
		failed="$failed $comp"
		continue
		;;
	skip-owned)
		log_warn "$comp: the installed copy ($dest) is owned by package ${origin#package:} and release $latest has no .deb to upgrade it with -- not overwriting it (use --method binary to replace the package with the raw binary)"
		skipped="$skipped $comp"
		continue
		;;
	replace-package)
		rep_pkg="${origin#package:}"
		rep_dest="$(component_default_dir "$comp")/$(component_binary "$comp")"
		replace_guard "$comp" "$rep_pkg" || { failed="$failed $comp"; continue; }
		print_replace_plan "$comp" "$rep_pkg" "$rep_dest" "$asset"
		read -r -p "Replace package $rep_pkg with the raw binary as listed above? This needs sudo$([ "$comp" = core ] && echo " and briefly disconnects every peer on this host"). [Y/n] " reply < /dev/tty
		if [[ "$reply" =~ ^[Nn]$ ]]; then skipped="$skipped $comp"; continue; fi
		install_component "$comp" "$rep_dest" "$asset" "$tag" "$rep_pkg" || failed="$failed $comp"
		continue
		;;
	binary)
		# A raw install under auto stays raw; say how to convert (PACKAGING-011).
		[ "$METHOD" = auto ] && [ "$origin" = binary ] && [ "$have_deb" -eq 1 ] \
			&& log_info "$comp: this is a raw install; --method deb converts it to the package"
		;;
	esac

	# Shadowed package: offer to delete the raw copy (default no; never without a
	# terminal). If it was removed, stop here for this component -- re-running
	# continues with the packaged copy rather than reinstalling a raw one.
	# A conversion removes the raw copy itself, so it is not asked about twice.
	if [ -z "$conv_raw" ] && [ -n "$shadow_raw" ] && offer_remove_shadow "$comp" "$shadow_raw"; then
		log_info "$comp: re-run this script to continue with the packaged copy"
		skipped="$skipped $comp"
		continue
	fi

	is_upgrade=0
	[ -n "$current" ] && is_upgrade=1
	if [ -n "$conv_raw" ]; then
		print_conversion_plan "$comp" "$conv_raw" "$deb_asset"
		confirm_conversion "$comp" || { skipped="$skipped $comp"; continue; }
	else
		confirm_sudo_install "$comp" "$is_upgrade" || { skipped="$skipped $comp"; continue; }
	fi

	if [ "$method" = deb ]; then
		install_component_deb "$comp" "$deb_asset" "$tag" "$conv_raw" || failed="$failed $comp"
	else
		install_component "$comp" "$dest" "$asset" "$tag" || failed="$failed $comp"
	fi
done

# Report anything you declined. A declined core upgrade gets its own warning,
# because add-ons newer than the core daemon may not work with it.
if [ -n "$skipped" ]; then
	log_warn "not installed/upgraded:$skipped"
	if in_list core "$skipped"; then
		log_warn "core was NOT upgraded -- the daemon is still on the version it was already running"
		log_warn "addons talk tetron-proto to the core daemon, so leaving core behind risks version skew; re-run and accept the core prompt to finish the upgrade"
	fi
fi

if [ -n "$failed" ]; then
	log_error "failed:$failed"
	any_failed=1
fi

exit "$any_failed"
