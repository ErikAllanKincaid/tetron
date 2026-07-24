//! Verified-blob reconvergence: resolve the network-key-signed pkarr record,
//! fetch + verify the `GroupBlob`, re-seat IP collisions, then apply the roster
//! to DNS. The 60s group poller and the peer-cleanup-adjacent helpers that drive
//! reconvergence live here.

use super::super::*;
use std::time::Instant;

/// Whether [`spawn_group_poller`] should fetch and adopt a freshly resolved
/// DHT record, given its `(hash, generation)` against this node's current
/// state. Generation is the authority (CONVERGE-005), not raw hash equality:
/// a DHT record at a generation we've already passed has nothing new for us,
/// even if our local hash momentarily differs (e.g. a pending local mutation
/// not yet published) — only a strictly higher generation is normally worth
/// the fetch.
///
/// **Except at an exact tie with a genuinely different hash**, which is
/// worth fetching too (found via live testing, 2026-07-17): this node's own
/// unrelated local mutations (e.g. pruning departed peers) can independently
/// advance its generation to the same number another coordinator's
/// mutations reached, entirely by coincidence. Treating "equal" as "nothing
/// new" in that case gets this node stuck polling forever — never noticing,
/// for example, that the network was nuked by someone else while it was
/// doing its own unrelated bookkeeping. A tie with a matching hash really is
/// a no-op and still skips.
fn poller_should_fetch(
    remote_generation: u64,
    remote_hash: blake3::Hash,
    current_generation: u64,
    current_hash: Option<blake3::Hash>,
) -> bool {
    !(remote_generation < current_generation
        || (remote_generation == current_generation && Some(remote_hash) == current_hash))
}

/// Resolve the network's *signed* group-blob hash (and seed peers) from the
/// pkarr record. This is the sole authority for the roster.
pub(crate) async fn resolve_signed(
    endpoint: &Endpoint,
    net_pubkey: EndpointId,
) -> Option<(blake3::Hash, u64, Vec<EndpointId>)> {
    let client = dht::create_pkarr_client(endpoint).ok()?;
    dht::resolve_network(&client, net_pubkey).await.ok()
}

/// Fetch the group blob for `signed` from any connected peer or seed, and verify
/// its bytes against `signed`. Returns the verified blob, or `None` if no source
/// could serve a blob matching the signed hash. The blob is content-addressed by
/// `signed`, so a peer can only ever serve the authentic blob — never a forgery.
pub(crate) async fn fetch_verified_blob(
    endpoint: &Endpoint,
    blob_store: &FsStore,
    peers: &PeerTable,
    signed: blake3::Hash,
    network_name: &str,
    seeds: &[EndpointId],
) -> Option<crate::membership::GroupBlob> {
    let blob_hash = iroh_blobs::Hash::from_bytes(*signed.as_bytes());
    let mut peer_ids: Vec<EndpointId> = peers
        .peers_for_network(network_name)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    peer_ids.extend_from_slice(seeds);
    peer_ids.sort_by_key(|id| id.to_string());
    peer_ids.dedup();
    for pid in &peer_ids {
        if let Ok(conn) =
            transport::connect_to_peer_with_alpn(endpoint, *pid, iroh_blobs::protocol::ALPN).await
            && blob_store
                .remote()
                .fetch(conn, HashAndFormat::raw(blob_hash))
                .await
                .is_ok()
            && let Ok(bytes) = blob_store.blobs().get_bytes(blob_hash).await
            && let Ok(data) = crate::membership::verify_group_blob(&bytes, &signed)
        {
            return Some(data);
        }
    }
    None
}

/// Reconverge the live network state from the signed pkarr record and apply it
/// (roster + DNS). Invoked when a peer sends a `MemberSync`
/// or `BlobUpdated` *hint* — the hint is only a trigger; the roster comes
/// exclusively from the network-key-signed record, never from the peer message.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn reconverge_and_apply(
    endpoint: &Endpoint,
    ctx: &MeshCtx,
    net_pubkey: EndpointId,
    network_name: &str,
    state: &SharedNetworkState,
    my_identity: EndpointId,
    // Retained for call-site stability with tetron (the rename drain that used
    // these was removed in MINIMAL-014).
    _alpn: &[u8],
    _my_ip: Ipv4Addr,
    left_tx: &mpsc::Sender<String>,
) {
    let MeshCtx {
        peers,
        blob_store,
        pruned_peers,
        ..
    } = ctx;
    let current = state.read().unwrap().snapshot.as_ref().map(|s| s.hash);
    let Some((signed, remote_generation, seeds)) = resolve_signed(endpoint, net_pubkey).await
    else {
        tracing::debug!(network = %network_name, "reconverge: signed record unavailable");
        return;
    };
    if crate::membership::trusted_reconverge_hash(current, signed).is_none() {
        // Already converged on the signed hash; nothing to apply.
        return;
    }
    // A nuke tombstone is fully deterministic given just its generation, so
    // check for that locally before ever attempting a peer fetch — the
    // publishing coordinator leaves immediately after publishing and is
    // typically the only node that ever held the bytes (see
    // `membership::try_decode_tombstone`).
    let data = match crate::membership::try_decode_tombstone(signed, remote_generation) {
        Some(tombstone) => tombstone,
        None => {
            let Some(data) =
                fetch_verified_blob(endpoint, blob_store, peers, signed, network_name, &seeds)
                    .await
            else {
                tracing::warn!(network = %network_name, "reconverge: could not fetch verified blob");
                return;
            };
            data
        }
    };
    // We are no longer in the authoritative roster (kicked, or a casualty of
    // the CONVERGE-001 publish race): leave locally instead of silently
    // applying a roster that excludes us (CONVERGE-003).
    if member_removed(&data.members, &data.approved, my_identity) {
        tracing::warn!(network = %network_name, "we have been removed from the network");
        let _ = left_tx.send(network_name.to_string()).await;
        return;
    }
    // Defense in depth (CONVERGE-005): never let a fetch from a lagging seed
    // peer downgrade local state below what we already hold. The publish-side
    // guard should prevent an actually-stale blob from ever reaching the DHT,
    // but a seed peer's own blob store can lag behind the DHT record it's
    // serving against.
    let current_generation = state.read().unwrap().generation;
    if data.generation < current_generation {
        tracing::debug!(
            network = %network_name,
            fetched = data.generation,
            current = current_generation,
            "fetched blob is older than local state; ignoring"
        );
        return;
    }
    // Two coordinators can independently admit a fresh joiner at the same
    // collision index, producing a roster with duplicate IPs. Resolve it
    // deterministically (lowest identity keeps the slot, others re-roll) before
    // it reaches the PeerTable/DNS so every node converges on the same map.
    let tiebroken = crate::membership::resolve_ip_tiebreak(
        data.members.clone(),
        crate::membership::resolve_subnet(data.subnet),
    );
    if let Err(e) = crate::membership::validate_no_duplicate_ips(&tiebroken) {
        tracing::warn!(network = %network_name, error = %e, "roster still has duplicate IPs after tiebreak; applying tiebroken version");
    }
    let roster = {
        let mut s = state.write().unwrap();
        s.generation = data.generation;
        s.members = MemberList::from_members(tiebroken);
        s.approved = ApprovedList::from_entries(data.approved.clone());
        // NUKE-CONSENSUS: synced purely for `tetron status` visibility. Nothing
        // reconverge-driven acts on this — the only place a nuke ever executes
        // is the synchronous `MeshManager::nuke_network` command handler.
        s.nuke_proposals = data.nuke_proposals.clone();
        s.refresh_snapshot();
        s.roster()
    };
    reconcile_local_hostname(&roster, network_name, my_identity);
    // Drop any live connection to a peer the signed roster no longer lists (it was
    // kicked, or left while we were offline). Removing it from the roster alone
    // stops us *routing* to it, but the peer reader keeps injecting its inbound
    // datagrams until the connection closes — so close it. We record the peer in
    // `pruned_peers` first: closing wakes our own reconnect loop, which would
    // otherwise re-dial the peer (it still lists us) and re-form the link.
    prune_departed_peers(peers, pruned_peers, state, network_name, my_identity);
    tracing::info!(network = %network_name, "reconverged from signed record");
}

/// Close and drop every connection to a peer that `network`'s current roster no
/// longer contains. Runs on every node after it applies a verified roster, so a
/// kicked (or departed) peer is severed mesh-wide, not just by the coordinator
/// that removed it. Each pruned peer is recorded in `pruned_peers` so this node's
/// reconnect loop skips the re-dial that closing the connection would trigger.
pub(crate) fn prune_departed_peers(
    peers: &PeerTable,
    pruned_peers: &Arc<DashSet<(String, EndpointId)>>,
    state: &SharedNetworkState,
    network_name: &str,
    my_identity: EndpointId,
) {
    let net_key = state.read().unwrap().network_public_key;
    for (peer_id, ip, conn) in peers.peers_for_network_with_conn(network_name) {
        let still_member = {
            let s = state.read().unwrap();
            s.members.all().iter().any(|m| m.identity == peer_id)
        };
        if still_member || peer_id == my_identity {
            continue;
        }
        tracing::info!(peer = %peer_id.fmt_short(), network = %network_name, "pruning peer no longer in roster");
        pruned_peers.insert((network_name.to_string(), peer_id));
        conn.close(
            VarInt::from_u32(forward::KICK_CODE),
            b"removed from network",
        );
        peers.remove_peer_from_network(&ip, &derive_ipv6(&peer_id, &net_key), network_name);
    }
}

/// Adopt this node's coordinator-assigned hostname for `network_name` from the
/// freshly-verified roster. Hostname is fixed at join (MINIMAL-014 removed
/// rename), but the coordinator may have collision-resolved it at admission
/// (e.g. `alice` → `alice-1`), so keep `config.my_hostname` in sync with the
/// signed blob's authoritative name (it backs `tetron status`).
pub(crate) fn reconcile_local_hostname(
    members: &[Member],
    network_name: &str,
    my_identity: EndpointId,
) {
    // Our own name in the freshly-fetched (authoritative) blob.
    let blob_self = members
        .iter()
        .find(|m| m.identity == my_identity)
        .and_then(|m| m.hostname.clone());

    if let (Some(mine), Ok(Some(mut net))) = (blob_self, config::load_network(network_name))
        && net.my_hostname.as_deref() != Some(mine.as_str())
    {
        net.my_hostname = Some(mine);
        let _ = config::save_network(&net);
    }
}

/// Whether `my_id` is still present in a freshly-fetched roster, as either a
/// full member or a pending-approval entry. Shared by `spawn_group_poller` and
/// `reconverge_and_apply` (CONVERGE-003) so both self-removal checks agree.
fn member_removed(
    members: &[crate::membership::Member],
    approved: &[ApprovedEntry],
    my_id: EndpointId,
) -> bool {
    !members.iter().any(|m| m.identity == my_id) && !approved.iter().any(|a| a.identity == my_id)
}

/// Floor on how often a manual `tetron sync` trigger (SYNC-001) can actually
/// force a fresh DHT resolve, so a spammed trigger can't reduce the
/// *effective* poll interval below this — a timer-driven tick is unaffected
/// (the configured interval itself already gates that path).
const MIN_MANUAL_SYNC_INTERVAL: Duration = Duration::from_secs(2);

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_group_poller(
    client: PkarrRelayClient,
    net_pubkey: EndpointId,
    state: SharedNetworkState,
    endpoint: Endpoint,
    ctx: MeshCtx,
    network_name: String,
    token: CancellationToken,
    left_tx: mpsc::Sender<String>,
    notify: Arc<Notify>,
) -> JoinHandle<()> {
    let MeshCtx {
        peers, blob_store, ..
    } = ctx;
    // CONFIG-AUDIT-002: read once at spawn time, not per tick -- matches
    // every other config-backed daemon setting (relay/discovery/ratelimit),
    // none of which live-reload mid-run either; a changed value takes effect
    // on the next `tetron restart`.
    let interval_secs = config::load()
        .ok()
        .and_then(|c| c.poller_interval)
        .unwrap_or(60);
    tracing::info!(network = %network_name, interval_secs, "group poller spawned");
    tokio::spawn(async move {
        // Backdated so a manual trigger fired immediately after spawn isn't
        // held back by the cooldown.
        let mut last_poll = Instant::now() - MIN_MANUAL_SYNC_INTERVAL;
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {},
                _ = notify.notified() => {
                    if last_poll.elapsed() < MIN_MANUAL_SYNC_INTERVAL {
                        continue;
                    }
                    tracing::debug!(network = %network_name, "group poller manually triggered");
                }
            }
            last_poll = Instant::now();
            tracing::debug!(network = %network_name, "group poller tick");

            let (current_generation, current_hash) = {
                let s = state.read().unwrap();
                (s.generation, s.snapshot.as_ref().map(|snap| snap.hash))
            };

            let (remote_hash, remote_generation, seed_peers) =
                match dht::resolve_network(&client, net_pubkey).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!(error = %e, "group poll failed");
                        continue;
                    }
                };

            if !poller_should_fetch(
                remote_generation,
                remote_hash,
                current_generation,
                current_hash,
            ) {
                continue;
            }

            tracing::info!(
                current_generation,
                remote_generation,
                new_hash = %remote_hash,
                "group blob changed"
            );

            // A nuke tombstone is fully deterministic given just its
            // generation — check locally before ever attempting a peer fetch
            // (see `membership::try_decode_tombstone`; the publishing
            // coordinator leaves immediately after, so it's typically the
            // only node that ever held the bytes).
            let data = match crate::membership::try_decode_tombstone(remote_hash, remote_generation)
            {
                Some(tombstone) => tombstone,
                None => {
                    // Use the same peer+seed fallback fetch as reconverge_and_apply
                    // (fetch_verified_blob) instead of only trying live PeerTable
                    // connections — the poller's own hand-rolled fetch used to give up
                    // ("could not fetch updated group blob from any peer") even with a
                    // live mesh connection to the same peer, since it never fell back
                    // to the DHT record's seed peers.
                    let Some(data) = fetch_verified_blob(
                        &endpoint,
                        &blob_store,
                        &peers,
                        remote_hash,
                        &network_name,
                        &seed_peers,
                    )
                    .await
                    else {
                        tracing::warn!("could not fetch updated group blob from any peer or seed");
                        continue;
                    };
                    data
                }
            };

            // Reconcile: find removed peers
            let old_members: Vec<EndpointId> = {
                let s = state.read().unwrap();
                s.members.all().iter().map(|m| m.identity).collect()
            };
            let new_member_ids: std::collections::HashSet<EndpointId> =
                data.members.iter().map(|m| m.identity).collect();

            for old_id in &old_members {
                if !new_member_ids.contains(old_id) {
                    let s = state.read().unwrap();
                    if let Some(member) = s.members.get(old_id) {
                        peers.remove(&member.ip, &derive_ipv6(old_id, &s.network_public_key));
                        tracing::info!(peer = %old_id.fmt_short(), "removed kicked peer");
                    }
                }
            }

            let my_id = endpoint.id();
            if member_removed(&data.members, &data.approved, my_id) {
                tracing::warn!(network = %network_name, "we have been removed from the network");
                let _ = left_tx.send(network_name.clone()).await;
                break;
            }

            // Update state from the freshly verified blob.
            {
                let mut s = state.write().unwrap();
                s.generation = data.generation;
                s.members = MemberList::from_members(data.members.clone());
                s.approved = ApprovedList::from_entries(data.approved.clone());
                // NUKE-CONSENSUS: visibility only, same as reconverge_and_apply.
                s.nuke_proposals = data.nuke_proposals.clone();
                s.refresh_snapshot();
            }
        }
    })
}

/// Current Unix time in seconds. Reusable-key expiry uses wall-clock time (the
/// same convention as the single-use invite ledger).
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(b: u8) -> blake3::Hash {
        blake3::hash(&[b])
    }

    #[test]
    fn poller_fetches_strictly_newer_generation() {
        assert!(poller_should_fetch(5, h(1), 4, Some(h(2))));
    }

    #[test]
    fn poller_skips_strictly_older_generation() {
        assert!(!poller_should_fetch(4, h(1), 5, Some(h(2))));
    }

    #[test]
    fn poller_skips_matching_tie() {
        assert!(!poller_should_fetch(5, h(1), 5, Some(h(1))));
    }

    #[test]
    fn poller_fetches_diverged_tie() {
        // The tie-fix (2026-07-17): same generation, different hash, must
        // still be fetched -- otherwise a node whose own unrelated mutations
        // happened to land on the same generation number gets stuck forever.
        assert!(poller_should_fetch(5, h(1), 5, Some(h(2))));
    }

    #[test]
    fn poller_fetches_when_no_local_snapshot_yet() {
        assert!(poller_should_fetch(5, h(1), 5, None));
    }
}
