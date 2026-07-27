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
///
/// `TETRON_LOG_DIR` overrides the resolved path on every platform
/// (PORTABILITY-003, same override-then-fixed-defaults shape as
/// `config::config_dir`'s `TETRON_CONFIG_DIR`) -- an install that never
/// sets it gets the exact same path as before this existed.
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TETRON_LOG_DIR") {
        return PathBuf::from(dir);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CONFIG_ENV_LOCK;

    #[test]
    fn log_dir_override() {
        // Reuses config.rs's env lock -- its own doc comment covers "any
        // other env var read by" a config-resolution function, not just
        // TETRON_CONFIG_DIR specifically.
        let _lock = CONFIG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("TETRON_LOG_DIR", "/tmp/custom-tetron-logs");
        }
        assert_eq!(log_dir(), PathBuf::from("/tmp/custom-tetron-logs"));
        unsafe {
            std::env::remove_var("TETRON_LOG_DIR");
        }
    }
}
