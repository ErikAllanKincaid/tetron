#!/bin/sh
# tetron-backup.sh - encrypted backup/restore of the tetron config tree.
#
# Backup:   sudo ./tetron-backup.sh [OUTPUT]
# Restore:  sudo ./tetron-backup.sh --restore FILE
#
# The archive is a passphrase-encrypted tar (age -p). The passphrase is the
# only way to ever decrypt it: lose it, lose the backup.
#
# This file is the single source of truth, living in the tetron repo at
# contrib/tetron-backup.sh. Delivery channels (no repo clone needed):
#   - raw.githubusercontent.com/ErikAllanKincaid/tetron/main/contrib/tetron-backup.sh
#   - `install-tetron-suite.sh backup` (installs to /usr/local/bin)
#   - tetron-webui: Add-ons > Config Backup popup (webui proxies this file)
#
# Config dir resolution mirrors tetron's own config_dir() (src/config.rs):
#   Linux            /etc/tetron
#   macOS (daemon)   /var/root/Library/Application Support/tetron  (root LaunchDaemon)
#   override         $TETRON_CONFIG_DIR
#
# Env overrides for scripted use:
#   TETRON_CONFIG_DIR       config tree to back up / restore into
#   TETRON_BACKUP_NO_SERVICE=1   skip stopping/starting the tetron service
set -eu

# --- config dir ---
if [ -n "${TETRON_CONFIG_DIR:-}" ]; then
    CONFIG_DIR="$TETRON_CONFIG_DIR"
elif [ "$(uname -s)" = "Linux" ]; then
    CONFIG_DIR="/etc/tetron"
elif [ "$(uname -s)" = "Darwin" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        CONFIG_DIR="/var/root/Library/Application Support/tetron"
    else
        CONFIG_DIR="$HOME/Library/Application Support/tetron"
    fi
else
    CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/tetron"
fi

# --- parse args ---
mode="backup"
case "${1:-}" in
    --restore)
        mode="restore"
        shift
        archive="${1:-}"
        [ -n "$archive" ] || { echo "error: --restore requires an archive file" >&2; exit 1; }
        ;;
    -h|--help|--*)
        echo "usage: $0 [OUTPUT] | $0 --restore FILE" >&2
        exit 1
        ;;
    *)
        out="${1:-}"
        ;;
esac

# --- escalate only when the operation actually needs it. Preserve PATH so a
# non-standard age install (e.g. ~/.local/bin) survives the sudo boundary.
# Re-exec with the original argv intact: the args re-parse identically after. ---
escalate() {
    exec sudo env "PATH=$PATH" "$0" "$@"
}
if [ "$mode" = "backup" ] && [ ! -r "$CONFIG_DIR/secret_key" ]; then
    [ "$(id -u)" -eq 0 ] || escalate "$@"
fi
if [ "$mode" = "restore" ] && [ ! -w "$CONFIG_DIR" ]; then
    [ "$(id -u)" -eq 0 ] || escalate "$@"
fi

need_age() {
    if ! command -v age >/dev/null 2>&1; then
        echo "error: 'age' not found. Install it first:" >&2
        echo "  Debian/Ubuntu: sudo apt install age" >&2
        echo "  Fedora:        sudo dnf install age" >&2
        echo "  Arch:          sudo pacman -S age" >&2
        echo "  macOS:         brew install age" >&2
        exit 1
    fi
}

do_backup() {
    need_age
    out="${1:-tetron-backup-$(hostname)-$(date +%F).tar.age}"
    tar -C "$CONFIG_DIR" -czf - . | age -p > "$out"
    chmod 600 "$out"
    # if we ran under sudo, hand the archive back to the invoking user
    if [ -n "${SUDO_USER:-}" ]; then
        chown "$SUDO_USER" "$out" 2>/dev/null || true
    fi
    echo "backup written to $out"
    echo "verify with: age -d '$out' | tar -tzf -"
}

do_restore() {
    need_age
    archive="$1"
    if [ "${TETRON_BACKUP_NO_SERVICE:-0}" != "1" ]; then
        case "$(uname -s)" in
            Linux)
                echo "stopping tetron daemon..."
                systemctl stop tetron || true
                ;;
            Darwin)
                echo "stopping tetron daemon..."
                launchctl bootout system /Library/LaunchDaemons/com.tetron.vpn.plist || true
                ;;
        esac
    fi
    mkdir -p "$CONFIG_DIR"
    age -d "$archive" | tar -xzf - -C "$CONFIG_DIR"
    case "$(uname -s)" in
        Linux) chmod 750 "$CONFIG_DIR" ;;
    esac
    echo "config restored to $CONFIG_DIR"
    if [ "${TETRON_BACKUP_NO_SERVICE:-0}" != "1" ]; then
        case "$(uname -s)" in
            Linux)
                echo "starting tetron daemon..."
                systemctl start tetron || true
                ;;
            Darwin)
                echo "starting tetron daemon..."
                launchctl bootstrap system /Library/LaunchDaemons/com.tetron.vpn.plist || true
                ;;
        esac
    fi
}

if [ "$mode" = "backup" ]; then
    do_backup "$out"
else
    do_restore "$archive"
fi
