//! Filesystem location for the daemon's rolling log files.
//!
//! The daemon runs as root, so these files are root-owned. `ray report` reads
//! them daemon-side (it already has access) and bundles them for the user.

use std::path::PathBuf;

/// Directory where the daemon writes rolling daily log files (`rayfish.log.*`).
///
/// Linux uses the conventional `/var/log/rayfish`; macOS uses `/Library/Logs/rayfish`
/// (visible in Console.app). Other platforms fall back to the user config dir.
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/log/rayfish")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Logs/rayfish")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rayfish")
            .join("logs")
    }
}
