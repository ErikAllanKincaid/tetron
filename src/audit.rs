use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};

pub struct AuditLog {
    file: Mutex<std::fs::File>,
}

impl AuditLog {
    pub fn open() -> Result<Self> {
        let path = log_path()?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("failed to open audit log")?;
        tracing::info!(path = %path.display(), "audit log opened");
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn log_connect(&self, peer_ip: Ipv4Addr, endpoint_id: &str) {
        self.write_entry("connect", peer_ip, endpoint_id);
    }

    pub fn log_disconnect(&self, peer_ip: Ipv4Addr, endpoint_id: &str) {
        self.write_entry("disconnect", peer_ip, endpoint_id);
    }

    fn write_entry(&self, event: &str, peer_ip: Ipv4Addr, endpoint_id: &str) {
        use std::io::Write;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let line = format!("{timestamp}\t{event}\t{peer_ip}\t{endpoint_id}\n");
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn log_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine config directory")?
        .join("pitopi");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("audit.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_path() {
        let path = log_path().unwrap();
        assert!(path.to_string_lossy().contains("pitopi"));
        assert!(path.to_string_lossy().ends_with("audit.log"));
    }
}
