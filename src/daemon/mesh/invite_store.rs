//! Per-network invite store for single-use invite keys.
//!
//! Each invite is a TOML file at `<config_dir>/invites/<network>/<invite-id>.toml`
//! containing the blake3 hash of the secret (never the plaintext), timestamps,
//! and a used flag. The `InviteStore` type wraps a directory path and provides
//! create/list/revoke/validate-and-burn operations, all through `&self` (the
//! filesystem provides interior mutability).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Length of the random invite secret in bytes (128 bits).
pub(crate) const INVITE_SECRET_LEN: usize = 16;

/// Length of the random invite id in bytes (64 bits, rendered as 16 hex chars).
const INVITE_ID_LEN: usize = 8;

/// A stored invite loaded from disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredInvite {
    pub(crate) id: String,
    pub(crate) secret_hash: String,
    pub(crate) created_at: u64,
    pub(crate) expires_at: u64,
    pub(crate) used: bool,
}

/// The on-disk format for a single invite file.
#[derive(Debug, Serialize, Deserialize)]
struct InviteFile {
    id: String,
    secret_hash: String,
    created_at: u64,
    expires_at: u64,
    used: bool,
}

/// Per-network invite store backed by TOML files on disk.
///
/// All methods take `&self` — the filesystem is the synchronization boundary.
/// The store directory is created lazily on first write operation.
#[derive(Clone)]
pub(crate) struct InviteStore {
    dir: PathBuf,
}

impl InviteStore {
    /// Open (or create) the invite store for a network.
    ///
    /// The directory is created if it does not exist. If creation fails, an
    /// error is returned so misuse is caught early.
    pub(crate) fn new(network_name: &str) -> Result<Self> {
        let dir = crate::config::config_dir()?
            .join("invites")
            .join(sanitize_name(network_name));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create invite store at {}", dir.display()))?;
        Ok(Self { dir })
    }

    /// Create a new invite and return `(invite_id, raw_secret)`.
    ///
    /// The caller encodes the full invite key using
    /// [`invite::encode_invite_code`] with the network pubkey and coordinator
    /// pubkey. The raw secret is 16 random bytes; only its blake3 hash is
    /// persisted.
    ///
    /// `None` means a 7-day default expiry (INVITE-009). `Some(0)` means
    /// permanent (never expires). Any other `Some(secs)` sets an explicit
    /// lifetime in seconds.
    pub(crate) fn create(&self, ttl_secs: Option<u64>) -> Result<(String, Vec<u8>)> {
        let id = random_hex(INVITE_ID_LEN);
        let secret: [u8; INVITE_SECRET_LEN] = rand::random();
        let hash = blake3::hash(&secret);
        let now = crate::daemon::mesh::reconverge::now_secs();
        let expires_at = match ttl_secs {
            None => now + 7 * 24 * 3600, // default: 7 days
            Some(0) => 0,                 // explicit permanent
            Some(ttl) => now + ttl,
        };

        let file = InviteFile {
            id: id.clone(),
            secret_hash: hash.to_hex().to_string(),
            created_at: now,
            expires_at,
            used: false,
        };

        let path = self.dir.join(format!("{id}.toml"));
        let content = toml::to_string(&file)
            .with_context(|| format!("failed to serialize invite {id}"))?;
        // Atomic write: temp file + rename
        let tmp = self.dir.join(format!("{id}.tmp"));
        std::fs::write(&tmp, &content)
            .with_context(|| format!("failed to write invite {id}"))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("failed to finalize invite {id}"))?;

        Ok((id, secret.to_vec()))
    }

    /// List all stored invites.
    pub(crate) fn list(&self) -> Result<Vec<StoredInvite>> {
        let mut invites = Vec::new();
        if !self.dir.exists() {
            return Ok(invites);
        }
        for entry in self.dir.read_dir().context("reading invite store")? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match load_invite_file(&path) {
                Ok(file) => invites.push(StoredInvite {
                    id: file.id,
                    secret_hash: file.secret_hash,
                    created_at: file.created_at,
                    expires_at: file.expires_at,
                    used: file.used,
                }),
                Err(e) => warn!(path=%path.display(), "failed to load invite: {e}"),
            }
        }
        // Sort by creation time so the most recent appear last.
        invites.sort_by_key(|i| i.created_at);
        Ok(invites)
    }

    /// Revoke an invite by id (mark as used so it cannot be redeemed).
    pub(crate) fn revoke(&self, id: &str) -> Result<()> {
        let path = self.dir.join(format!("{id}.toml"));
        if !path.exists() {
            bail!("invite '{id}' not found");
        }
        let mut file = load_invite_file(&path)?;
        file.used = true;
        let content = toml::to_string(&file)
            .with_context(|| format!("failed to serialize invite {id}"))?;
        let tmp = self.dir.join(format!("{id}.tmp"));
        std::fs::write(&tmp, &content)
            .with_context(|| format!("failed to write invite {id}"))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("failed to finalize invite {id}"))?;
        Ok(())
    }

    /// Validate a presented secret against the store.
    ///
    /// Returns `true` if the secret matches an unused, non-expired invite.
    /// On success (single-use invite), the invite is marked as used (burned).
    /// The store is *not* consulted for reuse: if the secret matches a
    /// reusable key on the signed blob, that path is handled separately via
    /// `membership::validate_reusable_key`.
    pub(crate) fn validate_and_burn(&self, secret: &[u8]) -> Result<bool> {
        if !self.dir.exists() {
            return Ok(false);
        }
        let hash = blake3::hash(secret);
        let hash_hex = hash.to_hex().to_string();
        let now = crate::daemon::mesh::reconverge::now_secs();

        // Build a quick lookup of all invite files, keyed by hash, to avoid
        // repeated deserialization. In practice the store is tiny (< 100).
        let mut by_hash: HashMap<String, PathBuf> = HashMap::new();
        for entry in self.dir.read_dir().context("reading invite store")? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            if let Ok(file) = load_invite_file(&path) {
                by_hash.insert(file.secret_hash, path);
            }
        }

        if let Some(path) = by_hash.get(&hash_hex) {
            let mut file = load_invite_file(path)?;
            if file.used {
                return Ok(false);
            }
            if file.expires_at > 0 && now >= file.expires_at {
                // Expired — mark as used so it won't be retried.
                file.used = true;
                let _ = save_invite_file(path, &file);
                return Ok(false);
            }
            // Single-use: burn it.
            file.used = true;
            if let Err(e) = save_invite_file(path, &file) {
                warn!(id = %file.id, "failed to mark invite as used: {e}");
                // Still admit — the invite was valid and the burn is best-effort.
            }
            return Ok(true);
        }

        Ok(false)
    }
}

/// Load a single invite file from disk.
fn load_invite_file(path: &Path) -> Result<InviteFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parsing {}", path.display()))
}

/// Atomically write an invite file (temp + rename).
fn save_invite_file(path: &Path, file: &InviteFile) -> Result<()> {
    let dir = path.parent().unwrap();
    let tmp = dir.join(format!("{}.tmp", file.id));
    let content = toml::to_string(file)?;
    std::fs::write(&tmp, &content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Generate a random hex string of `n` bytes (2n hex chars).
fn random_hex(n: usize) -> String {
    let buf: Vec<u8> = (0..n).map(|_| rand::random::<u8>()).collect();
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sanitize a network name for use as a directory component:
/// replace non-alphanumeric characters with underscores.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep the TempDir alive so its path is valid for the store's lifetime.
    struct StoreWithTemp {
        _temp: tempfile::TempDir,
        store: InviteStore,
    }

    fn test_store() -> StoreWithTemp {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("invites");
        std::fs::create_dir_all(&store_dir).unwrap();
        StoreWithTemp {
            _temp: dir,
            store: InviteStore { dir: store_dir },
        }
    }

    #[test]
    fn test_create_and_list() {
        let t = test_store();

        assert!(t.store.list().unwrap().is_empty());

        let (id, secret) = t.store.create(None).unwrap();
        assert_eq!(id.len(), 16); // 8 bytes = 16 hex chars
        assert_eq!(secret.len(), INVITE_SECRET_LEN);

        let invites = t.store.list().unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].id, id);
        assert!(!invites[0].used);
    }

    #[test]
    fn test_validate_and_burn() {
        let t = test_store();

        let (_id, secret) = t.store.create(None).unwrap();

        // Valid secret
        assert!(t.store.validate_and_burn(&secret).unwrap());

        // Already burned — second call fails
        assert!(!t.store.validate_and_burn(&secret).unwrap());
    }

    #[test]
    fn test_unknown_secret() {
        let t = test_store();

        assert!(!t.store.validate_and_burn(b"some random bytes").unwrap());
    }

    #[test]
    fn test_revoke() {
        let t = test_store();

        let (id, secret) = t.store.create(None).unwrap();

        t.store.revoke(&id).unwrap();

        // Revoked invite should not validate
        assert!(!t.store.validate_and_burn(&secret).unwrap());

        // Listing shows it as used
        let invites = t.store.list().unwrap();
        assert!(invites.iter().any(|i| i.id == id && i.used));
    }

    #[test]
    fn test_permanent_invite() {
        let t = test_store();

        // Some(0) means permanent (never expires) after INVITE-009.
        let (_id, secret) = t.store.create(Some(0)).unwrap();

        // Should not be expired — permanent invites are always valid.
        assert!(t.store.validate_and_burn(&secret).unwrap());
    }
}
