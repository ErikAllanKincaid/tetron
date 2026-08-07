//! Persistent Ed25519 identity stored at `~/.config/tetron/secret_key`.
//!
//! The same keypair is used across restarts, giving each node a stable
//! [`EndpointId`](iroh::EndpointId) and deterministic virtual IP.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use iroh::{EndpointId, SecretKey};

use crate::addressing::{Subnet, derive_ip, derive_ip_with_index};

use crate::config::config_dir;

fn key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("secret_key"))
}

/// Loads the secret key from disk, or generates and persists a new one.
pub fn load_or_create() -> Result<SecretKey> {
    let path = key_path()?;
    if path.exists() {
        let bytes: [u8; 32] = std::fs::read(&path)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("corrupt secret key file"))?;
        let key = SecretKey::from_bytes(&bytes);
        tracing::info!(id = %key.public().fmt_short(), "loaded identity");
        Ok(key)
    } else {
        let key = SecretKey::generate();
        crate::config::write_file(&path, &key.to_bytes(), true)?;
        tracing::info!(id = %key.public().fmt_short(), "generated new identity");
        Ok(key)
    }
}

fn collision_index_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("collision_index"))
}

pub fn load_collision_index() -> Result<u32> {
    let path = collision_index_path()?;
    if path.exists() {
        let s = std::fs::read_to_string(&path).context("read collision_index")?;
        s.trim().parse::<u32>().context("parse collision_index")
    } else {
        Ok(0)
    }
}

/// Abstracts identity and IP derivation so the membership system doesn't
/// depend directly on iroh types.
///
/// Moved from `membership.rs` (MODULARIZE-001); `crate::membership::…`
/// paths to both items here keep working via re-export.
pub trait IdentityProvider: Send + Sync {
    fn local_ip(&self) -> Ipv4Addr;
    fn local_identity(&self) -> EndpointId;
    fn derive_ip(&self, peer_identity: &EndpointId) -> Ipv4Addr;
}

/// [`IdentityProvider`] backed by an iroh [`EndpointId`].
#[derive(Clone)]
pub struct IrohIdentityProvider {
    endpoint_id: EndpointId,
    ip: Ipv4Addr,
    /// The node's operative overlay subnet (from `AppConfig.subnet` at
    /// bootstrap; [`crate::addressing::default_subnet`] otherwise). Drives
    /// `local_ip` and every peer IP derived through this provider.
    subnet: Subnet,
}

impl IrohIdentityProvider {
    pub fn new(endpoint_id: EndpointId, collision_index: u32, subnet: Subnet) -> Self {
        let ip = derive_ip_with_index(&endpoint_id, collision_index, subnet);
        Self {
            endpoint_id,
            ip,
            subnet,
        }
    }

    /// The node's operative overlay subnet.
    pub fn subnet(&self) -> Subnet {
        self.subnet
    }
}

impl IdentityProvider for IrohIdentityProvider {
    fn local_ip(&self) -> Ipv4Addr {
        self.ip
    }

    fn local_identity(&self) -> EndpointId {
        self.endpoint_id
    }

    fn derive_ip(&self, peer_identity: &EndpointId) -> Ipv4Addr {
        derive_ip(peer_identity, self.subnet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addressing::{default_subnet, ip_in_subnet};

    #[test]
    fn test_iroh_identity_provider() {
        let key = SecretKey::generate();
        let endpoint_id = key.public();
        let provider = IrohIdentityProvider::new(endpoint_id, 0, default_subnet());

        let ip = provider.local_ip();
        assert!(ip_in_subnet(ip, default_subnet()));

        let id = provider.local_identity();
        assert_eq!(provider.derive_ip(&id), ip);
    }
}
