//! Filesystem location for the daemon's rolling log files.
//!
//! The daemon runs as root, so these files are root-owned; read them with
//! `sudo` (or `journalctl -u tetron` for the service console log).

use std::path::PathBuf;

/// Directory where the daemon writes rolling daily log files (`tetron.log.*`).
///
/// Linux uses the conventional `/var/log/tetron`; macOS uses `/Library/Logs/tetron`
/// (visible in Console.app). Other platforms fall back to the user config dir.
///
/// The appender retains the 7 most recent daily files (see `main::init_tracing`),
/// so logs older than ~a week are pruned automatically.
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/log/tetron")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Logs/tetron")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tetron")
            .join("logs")
    }
}
