//! DHT publishers for the mesh core: the notify-driven network-record
//! publisher, the lazy co-coordinator publisher, and the shared
//! snapshot-refresh + publish step.

use super::super::*;

/// Read-before-write guard for DHT publishing (CONVERGE-005). Returns `true`
/// if the caller should proceed with publishing `(local_generation,
/// local_hash)`.
///
/// CONVERGE-001's original guard compared raw hashes: it could tell "did the
/// DHT change under me" but not "is that change actually newer" — so an
/// out-of-date coordinator's periodic republish could permanently win over a
/// co-coordinator's fresher admission just by writing later, and once a
/// publisher saw a hash it didn't recognize it deferred to it *forever*, even
/// when its own state was objectively the correct, newer one.
///
/// Generation is the authority now, not write order: publish whenever the DHT
/// is at a strictly lower generation than ours (regardless of whether we
/// recognize its hash), defer whenever it's at a strictly higher one. Only at
/// an exact generation tie (two coordinators independently mutated from the
/// same base) does a hash comparison arbitrate — and only to detect an
/// already-published no-op. A genuine tie with different content at the same
/// generation is left alone rather than fought over: the loser's own next
/// local mutation bumps its generation past the tie and wins outright next
/// time, rather than the two publishers flip-flopping.
///
/// **Always does the real comparison — no "first publish" bypass** (removed
/// 2026-07-17, found via live testing): a previous version treated a
/// caller's first-ever publish attempt (no `last_published` yet) as
/// automatically safe, unconditionally overwriting whatever was actually on
/// the DHT. That was fine for a genuinely brand-new network (nothing to
/// compare against — the `Err` arm below already handles that correctly),
/// but for a coordinator whose restore fell back to stale local config
/// (DHT/blob unreachable at restart), its very first publish attempt could
/// resurrect that stale state over a concurrently-mutated (or even already
/// nuked) record. Removing the bypass unifies "first publish" and "any
/// subsequent publish" onto one path: always compare against what's
/// actually live first.
pub(crate) async fn dht_read_before_write(
    client: &PkarrRelayClient,
    net_pubkey: EndpointId,
    local_generation: u64,
    local_hash: blake3::Hash,
) -> bool {
    match crate::dht::resolve_network(client, net_pubkey).await {
        Ok((dht_hash, dht_generation, _)) => match local_generation.cmp(&dht_generation) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => {
                tracing::info!(
                    dht_generation,
                    local_generation,
                    "DHT record is at a newer generation; skipping publish (reconverge will pick up)"
                );
                false
            }
            std::cmp::Ordering::Equal => {
                if dht_hash == local_hash {
                    false
                } else {
                    tracing::info!(
                        dht_hash = %dht_hash,
                        local_hash = %local_hash,
                        generation = local_generation,
                        "DHT record diverged at the same generation (concurrent mutation); leaving it — next local mutation will move past the tie"
                    );
                    false
                }
            }
        },
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
        loop {
            let (generation, hash) = {
                let s = state.read().unwrap();
                let hash = s
                    .snapshot
                    .as_ref()
                    .map(|snap| snap.hash)
                    .unwrap_or_else(|| {
                        group_blob_hash(
                            s.generation,
                            &s.members,
                            &s.approved,
                            &s.suggested_firewall,
                            s.network_name.as_deref(),
                            &s.reusable_keys,
                            s.blob_subnet(),
                            &s.invites,
                            &s.nuke_proposals,
                        )
                    });
                (s.generation, hash)
            };
            if dht_read_before_write(&client, net_pubkey, generation, hash).await {
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
                    generation,
                    &seed_peers,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!(peers = seed_peers.len(), "published network record");
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
            let (generation, hash) = {
                let s = state.read().unwrap();
                let hash = s
                    .snapshot
                    .as_ref()
                    .map(|snap| snap.hash)
                    .unwrap_or_else(|| {
                        group_blob_hash(
                            s.generation,
                            &s.members,
                            &s.approved,
                            &s.suggested_firewall,
                            s.network_name.as_deref(),
                            &s.reusable_keys,
                            s.blob_subnet(),
                            &s.invites,
                            &s.nuke_proposals,
                        )
                    });
                (s.generation, hash)
            };
            // Only attempt publish if the hash changed since our last publish
            // AND the read-before-write guard passes (CONVERGE-005).
            if last_published != Some(hash)
                && dht_read_before_write(&client, net_pubkey, generation, hash).await
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
                    generation,
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
        s.bump_generation_and_refresh();
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
        let (hash, generation, net_key, snap_bytes) = {
            let Some(handle) = self.networks.get(network) else {
                return;
            };
            let s = handle.state.read().unwrap();
            (
                s.snapshot.as_ref().map(|x| x.hash),
                s.generation,
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
            if let Err(e) =
                dht::publish_network(&client, &key, &hash, generation, &seed_peers).await
            {
                tracing::warn!(error = %e, "failed to publish network record after accept");
            }
        }
    }
}
