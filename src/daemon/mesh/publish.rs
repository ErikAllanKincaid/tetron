//! DHT publishers for the mesh core: the notify-driven network-record
//! publisher, the lazy co-coordinator publisher, and the shared
//! snapshot-refresh + publish step.

use super::super::*;

/// Read-before-write guard for DHT publishing. Returns `true` if the caller
/// should proceed with publishing `local_hash`, `false` if the DHT already has
/// a different (presumably newer) blob that the group poller should reconcile.
///
/// The rule: publish if we have never published before (`last_published` is
/// `None`), OR the DHT record matches our `last_published` hash (no one else
/// changed it since we did), OR the DHT has no record yet. Skip if the DHT
/// hash differs from `last_published` — another coordinator published a newer
/// blob (CONVERGE-001).
async fn dht_read_before_write(
    client: &PkarrRelayClient,
    net_pubkey: EndpointId,
    local_hash: blake3::Hash,
    last_published: Option<blake3::Hash>,
) -> bool {
    let Some(lp) = last_published else {
        // First publish for this coordinator — always proceed.
        return true;
    };
    match crate::dht::resolve_network(client, net_pubkey).await {
        Ok((dht_hash, _)) => {
            if dht_hash == lp {
                // DHT still points to our last published blob — safe to publish.
                true
            } else if dht_hash == local_hash {
                // DHT already matches our local state — no-op, skip publish.
                false
            } else {
                // DHT has a different hash — another coordinator published a
                // newer blob. Don't overwrite; the group poller will reconcile.
                tracing::info!(
                    dht_hash = %dht_hash,
                    local_hash = %local_hash,
                    last_published = %lp,
                    "DHT record changed externally; skipping publish (reconverge will pick up)"
                );
                false
            }
        }
        Err(_) => {
            // No DHT record yet — safe to publish.
            true
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_network_publisher(
    client: PkarrRelayClient,
    net_secret_key: SecretKey,
    state: SharedNetworkState,
    endpoint_id: EndpointId,
    peers: PeerTable,
    network_name: String,
    notify: Arc<tokio::sync::Notify>,
    token: CancellationToken,
) -> JoinHandle<()> {
    let net_pubkey = net_secret_key.public();
    tokio::spawn(async move {
        let mut last_published: Option<blake3::Hash> = None;
        loop {
            let hash = {
                let s = state.read().unwrap();
                s.snapshot
                    .as_ref()
                    .map(|snap| snap.hash)
                    .unwrap_or_else(|| {
                        group_blob_hash(
                            &s.members,
                            &s.approved,
                            &s.suggested_firewall,
                            s.network_name.as_deref(),
                            &s.reusable_keys,
                            s.blob_subnet(),
                            &s.invites,
                        )
                    })
            };
            if dht_read_before_write(&client, net_pubkey, hash, last_published).await {
                let mut seed_peers: Vec<EndpointId> = peers
                    .peers_for_network(&network_name)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                seed_peers.push(endpoint_id);
                seed_peers.sort_by_key(|id| id.to_string());
                seed_peers.dedup();

                match crate::dht::publish_network(
                    &client,
                    &net_secret_key,
                    &hash,
                    &seed_peers,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!(peers = seed_peers.len(), "published network record");
                        last_published = Some(hash);
                    }
                    Err(e) => tracing::warn!(error = %e, "failed to publish network record"),
                }
            }
            tokio::select! {
                _ = token.cancelled() => break,
                _ = notify.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(300)) => {},
            }
        }
    })
}

/// A polling publisher for a *granted* co-coordinator (a member that received
/// the network key via `AdminGrant`). Unlike [`spawn_network_publisher`] (which
/// is notify-driven and spawned at create/restore time), this is spawned at
/// runtime when a member is promoted: it has no `dht_notify` handle, so it
/// re-reads the snapshot hash every few seconds and republishes on change.
/// Latency is bounded by `LAZY_PUBLISH_INTERVAL`; members' 60s group poller is
/// the downstream backstop regardless. Uses the same read-before-write guard
/// as [`spawn_network_publisher`] (CONVERGE-001).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_lazy_publisher(
    client: PkarrRelayClient,
    net_secret_key: SecretKey,
    state: SharedNetworkState,
    endpoint_id: EndpointId,
    peers: PeerTable,
    network_name: String,
    token: CancellationToken,
) -> JoinHandle<()> {
    let net_pubkey = net_secret_key.public();
    const LAZY_PUBLISH_INTERVAL: Duration = Duration::from_secs(10);
    tokio::spawn(async move {
        let mut last_published: Option<blake3::Hash> = None;
        loop {
            let hash = {
                let s = state.read().unwrap();
                s.snapshot
                    .as_ref()
                    .map(|snap| snap.hash)
                    .unwrap_or_else(|| {
                        group_blob_hash(
                            &s.members,
                            &s.approved,
                            &s.suggested_firewall,
                            s.network_name.as_deref(),
                            &s.reusable_keys,
                            s.blob_subnet(),
                            &s.invites,
                        )
                    })
            };
            // Only attempt publish if the hash changed since our last publish
            // AND the read-before-write guard passes (CONVERGE-001).
            if last_published != Some(hash)
                && dht_read_before_write(&client, net_pubkey, hash, last_published).await
            {
                let mut seed_peers: Vec<EndpointId> = peers
                    .peers_for_network(&network_name)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                seed_peers.push(endpoint_id);
                seed_peers.sort_by_key(|id| id.to_string());
                seed_peers.dedup();
                match crate::dht::publish_network(
                    &client,
                    &net_secret_key,
                    &hash,
                    &seed_peers,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!(
                            network = %network_name,
                            "lazy publisher: published network record"
                        );
                        last_published = Some(hash);
                    }
                    Err(e) => tracing::warn!(error = %e, "lazy publisher: publish failed"),
                }
            }
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(LAZY_PUBLISH_INTERVAL) => {},
            }
        }
    })
}

pub(crate) async fn update_snapshot_and_publish(
    state: &SharedNetworkState,
    blob_store: &FsStore,
    dht_notify: &Option<Arc<tokio::sync::Notify>>,
) {
    let snap_bytes = {
        let mut s = state.write().unwrap();
        s.refresh_snapshot();
        s.snapshot.as_ref().map(|snap| snap.msgpack_bytes.clone())
    };
    if let Some(bytes) = snap_bytes
        && let Err(e) = blob_store.blobs().add_slice(&bytes).await
    {
        tracing::error!(error = %e, "update_snapshot_and_publish: add_slice failed");
    }
    if let Some(notify) = dht_notify {
        notify.notify_one();
    }
}

impl MeshManager {
    /// Store the current group snapshot as a blob and re-publish the pkarr record
    /// so members reconcile the new membership (used after `tetron accept`).
    pub(crate) async fn store_and_publish_group(&self, network: &str) {
        let (hash, net_key, snap_bytes) = {
            let Some(handle) = self.networks.get(network) else {
                return;
            };
            let s = handle.state.read().unwrap();
            (
                s.snapshot.as_ref().map(|x| x.hash),
                s.network_secret_key.clone(),
                s.snapshot.as_ref().map(|x| x.msgpack_bytes.clone()),
            )
        };
        if let Some(bytes) = snap_bytes
            && let Err(e) = self.blob_store.blobs().add_slice(&bytes).await
        {
            tracing::error!(error = %e, "store_and_publish_group: add_slice failed");
        }
        if let (Some(hash), Some(key)) = (hash, net_key)
            && let Ok(client) = dht::create_pkarr_client(&self.endpoint)
        {
            let mut seed_peers: Vec<EndpointId> = self
                .peers
                .peers_for_network(network)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            seed_peers.push(self.endpoint.id());
            seed_peers.sort_by_key(|id| id.to_string());
            seed_peers.dedup();
            if let Err(e) = dht::publish_network(&client, &key, &hash, &seed_peers).await {
                tracing::warn!(error = %e, "failed to publish network record after accept");
            }
        }
    }
}
