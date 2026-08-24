//! Peer address cache — write known peer addresses to disk so reconnection
//! works without DHT after an all-offline gap.
//!
//! # Design
//!
//! On graceful shutdown (and every 5 minutes), this module iterates live
//! connections from [`PeerTable`], extracts each peer's [`TransportAddr`]s
//! (relay URL and direct IPs) from the connection's paths, and persists them
//! to `<config_dir>/peercache.msgpack`.
//!
//! On startup, the cache is loaded and [`connect_to_peer_with_alpn`] checks it
//! before dialing. If cached addresses exist, iroh tries them directly,
//! bypassing DHT lookup. Stale addresses are harmless because iroh verifies
//! identity via the QUIC crypto handshake — wrong addresses produce connection
//! failure, not wrong peers.
//!
//! Written atomically (temp file + rename). Entries older than 30 days are
//! pruned on load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iroh::endpoint::Connection;
use iroh::{EndpointId, TransportAddr};
use serde::{Deserialize, Serialize};

use crate::peers::PeerTable;

const CACHE_FILENAME: &str = "peercache.msgpack";
const SAVE_INTERVAL: Duration = Duration::from_secs(300);
const PRUNE_DAYS: u64 = 30;

/// A persisted cache entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    id: EndpointId,
    addrs: Vec<TransportAddr>,
    last_seen: u64,
}

/// Thread-safe peer address cache, backed by a global.
struct PeerAddrCache {
    path: PathBuf,
    inner: Mutex<HashMap<EndpointId, (Vec<TransportAddr>, u64)>>,
}

impl PeerAddrCache {
    /// Load from disk, or create empty if no cache file exists.
    fn new(config_dir: &Path) -> Self {
        let path = config_dir.join(CACHE_FILENAME);
        let entries: Vec<Entry> = if path.exists() {
            std::fs::read(&path)
                .ok()
                .and_then(|data| rmp_serde::from_slice(&data).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let now = now_secs();
        let inner = entries
            .into_iter()
            .filter(|e| is_fresh(e.last_seen, now))
            .map(|e| (e.id, (e.addrs, e.last_seen)))
            .collect();
        PeerAddrCache {
            path,
            inner: Mutex::new(inner),
        }
    }

    /// Return cached addresses for `id`, or `None`.
    fn lookup(&self, id: &EndpointId) -> Option<Vec<TransportAddr>> {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .map(|(addrs, _)| addrs.clone())
            .filter(|a| !a.is_empty())
    }

    /// Populate or refresh the entry for `id` with a set of addresses.
    fn update(&self, id: EndpointId, addrs: Vec<TransportAddr>) {
        let now = now_secs();
        self.inner.lock().unwrap().insert(id, (addrs, now));
    }

    /// Drop entries older than `PRUNE_DAYS` (CACHE-002). Without this the
    /// in-memory map only ever grows: `update` has no eviction path, and
    /// the age filter in `new` only runs once, at daemon startup.
    fn prune_stale(&self) {
        let now = now_secs();
        self.inner
            .lock()
            .unwrap()
            .retain(|_, (_, last_seen)| is_fresh(*last_seen, now));
    }

    /// Write the cache to disk atomically.
    fn save(&self) {
        self.prune_stale();
        let inner = self.inner.lock().unwrap();
        let entries: Vec<Entry> = inner
            .iter()
            .map(|(id, (addrs, last_seen))| Entry {
                id: *id,
                addrs: addrs.clone(),
                last_seen: *last_seen,
            })
            .collect();
        let data = match rmp_serde::to_vec(&entries) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize peer cache");
                return;
            }
        };
        // Atomic write: temp file then rename.
        let tmp = self.path.with_extension("msgpack.tmp");
        match std::fs::write(&tmp, &data) {
            Ok(_) => {
                if let Err(e) = std::fs::rename(&tmp, &self.path) {
                    tracing::warn!(error = %e, "failed to rename peer cache");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to write peer cache");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Global singleton — the daemon initialises it once at startup.
// ---------------------------------------------------------------------------

use std::sync::LazyLock;

static CACHE: LazyLock<Mutex<Option<PeerAddrCache>>> = LazyLock::new(|| Mutex::new(None));

/// Initialise the global cache from disk. Must be called once at daemon
/// startup, before any connections are made.
pub fn init(config_dir: &Path) {
    let cache = PeerAddrCache::new(config_dir);
    *CACHE.lock().unwrap() = Some(cache);
    tracing::debug!("peer address cache initialised");
}

/// Look up cached addresses for a peer. Called by
/// [`connect_to_peer_with_alpn`] in `transport.rs`.
pub(crate) fn lookup(id: &EndpointId) -> Option<Vec<TransportAddr>> {
    CACHE.lock().unwrap().as_ref().and_then(|c| c.lookup(id))
}

/// Persist the current cache to disk. Called on graceful shutdown.
pub fn save() {
    if let Some(cache) = CACHE.lock().unwrap().as_ref() {
        cache.save();
    }
}

/// Extract addresses from a live connection and update the cache.
///
/// Walks every path on the connection and collects direct IPs, relay URLs,
/// and custom addresses. Idempotent — called periodically and on shutdown.
pub fn update_from_connection(endpoint_id: EndpointId, conn: &Connection) {
    let paths = conn.paths();
    let addrs: Vec<TransportAddr> = paths.iter().map(|p| p.remote_addr().clone()).collect();
    if addrs.is_empty() {
        return;
    }
    if let Some(cache) = CACHE.lock().unwrap().as_ref() {
        cache.update(endpoint_id, addrs);
    }
}

/// Refresh the cache from all entries in the peer table. Called periodically
/// and before save-on-shutdown to capture live addresses.
pub fn refresh_from_peers(peers: &PeerTable) {
    for (id, _ipv4, conn) in peers.all_entries() {
        update_from_connection(id, &conn);
    }
}

/// Spawn a background task that periodically refreshes the cache from live
/// connections and saves to disk. The task runs until `token` is cancelled.
pub fn spawn_periodic_save(token: tokio_util::sync::CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SAVE_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    save();
                }
                _ = token.cancelled() => {
                    save();
                    break;
                }
            }
        }
    });
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Whether an entry last seen at `last_seen` is still within the
/// `PRUNE_DAYS` retention window as of `now`.
fn is_fresh(last_seen: u64, now: u64) -> bool {
    now.saturating_sub(last_seen) < PRUNE_DAYS * 86400
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = iroh::SecretKey::from(key_bytes);
        key.public()
    }

    fn addr() -> Vec<TransportAddr> {
        vec![iroh::TransportAddr::Ip(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            4242,
        )))]
    }

    #[test]
    fn prune_stale_drops_entries_past_the_retention_window() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PeerAddrCache::new(dir.path());
        let now = now_secs();
        let stale_id = test_id(1);
        let fresh_id = test_id(2);
        {
            let mut inner = cache.inner.lock().unwrap();
            inner.insert(stale_id, (addr(), now - (PRUNE_DAYS + 1) * 86400));
            inner.insert(fresh_id, (addr(), now));
        }

        cache.prune_stale();

        let inner = cache.inner.lock().unwrap();
        assert!(!inner.contains_key(&stale_id));
        assert!(inner.contains_key(&fresh_id));
    }

    #[test]
    fn save_prunes_stale_entries_before_persisting() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PeerAddrCache::new(dir.path());
        let now = now_secs();
        {
            let mut inner = cache.inner.lock().unwrap();
            inner.insert(test_id(1), (addr(), now - (PRUNE_DAYS + 1) * 86400));
            inner.insert(test_id(2), (addr(), now));
        }

        cache.save();

        let data = std::fs::read(dir.path().join(CACHE_FILENAME)).unwrap();
        let entries: Vec<Entry> = rmp_serde::from_slice(&data).unwrap();
        assert_eq!(entries.len(), 1);
    }
}
