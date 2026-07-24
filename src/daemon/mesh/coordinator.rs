//! Coordinator-side background loops: the per-member control reader (renames,
//! invite gossip, ping/pong), the dead-peer cleanup loop, and the invite-gossip
//! send helpers.

use super::super::*;

/// Extra context a coordinator needs to prune the canonical member list when a
/// peer leaves deliberately (`tetron leave`). Members pass `None` and only ever
/// drop the connection from the [`PeerTable`].
pub(crate) struct CoordinatorCleanup {
    pub(crate) state: SharedNetworkState,
    pub(crate) blob_store: FsStore,
    pub(crate) dht_notify: Option<Arc<tokio::sync::Notify>>,
    pub(crate) network_name: String,
}

pub(crate) fn spawn_peer_cleanup(
    mut disconnect_rx: mpsc::Receiver<forward::DisconnectEvent>,
    peers: PeerTable,
    token: CancellationToken,
    coordinator: Option<CoordinatorCleanup>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                event = disconnect_rx.recv() => {
                    match event {
                        Some(ev) => {
                            // Drop only this network's route, and only if the
                            // stored connection is still the one that died. A
                            // peer that was killed and re-dialed with the same
                            // identity already has a fresh connection registered;
                            // the stale connection's delayed disconnect must not
                            // evict it (see DisconnectEvent::conn_stable_id).
                            let removed = match ev.conn_stable_id {
                                Some(id) => peers.remove_peer_from_network_if(&ev.ip, &ev.ipv6, &ev.network, id),
                                None => {
                                    peers.remove_peer_from_network(&ev.ip, &ev.ipv6, &ev.network);
                                    true
                                }
                            };
                            if !removed {
                                tracing::debug!(peer = %ev.endpoint_id.fmt_short(), ip = %ev.ip, network = %ev.network, "ignoring stale disconnect; peer already reconnected");
                                continue;
                            }
                            tracing::info!(peer = %ev.endpoint_id.fmt_short(), ip = %ev.ip, network = %ev.network, reason = ?ev.reason, "removing dead peer");

                            // A deliberate `tetron leave` prunes the member from the
                            // roster; anything else (including a KICK_CODE close —
                            // CONVERGE-007: never roster authority on its own, since
                            // prune_departed_peers sends it from any node's possibly
                            // transiently stale view, not just a real kick) stamps
                            // the member's `last_seen` so the ephemeral pruner can
                            // age it out. Both republish the signed blob and
                            // broadcast a MemberSync so co-coordinators converge.
                            // Only the coordinator is authoritative, so members pass
                            // `coordinator = None` and do neither.
                            if let Some(c) = &coordinator {
                                let member_id = ev.endpoint_id;
                                let mut changed = false;
                                {
                                    let mut st = c.state.write().unwrap();
                                    if ev.reason.prunes_member() {
                                        st.members.remove(&member_id);
                                        changed = true;
                                    } else if let Some(m) = st.members.get_mut(&member_id) {
                                        m.last_seen = Some(crate::membership::now_secs());
                                        changed = true;
                                    }
                                }
                                if changed {
                                    update_snapshot_and_publish(&c.state, &c.blob_store, &c.dht_notify).await;
                                    broadcast_member_sync(&peers, None).await;
                                    if ev.reason.prunes_member() {
                                        tracing::info!(peer = %member_id.fmt_short(), "pruned member after leave");
                                    } else {
                                        tracing::debug!(peer = %member_id.fmt_short(), network = %c.network_name, "stamped last_seen on member disconnect");
                                    }
                                }
                            }
                        }
                        None => return,
                    }
                }
            }
        }
    })
}

/// Coordinator-side per-member control reader. Continuously accepts control
/// streams from one member and answers `Ping`/`Pong`; every other message
/// (including `MeshHello` — hostname is fixed at join, MINIMAL-014 removed
/// rename propagation) is received but not acted on. Runs until the network
/// token is cancelled or the connection drops.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_coordinator_control_reader(
    conn: Connection,
    remote_id: EndpointId,
    _peer_ip: Ipv4Addr,
    _network_name: String,
    token: CancellationToken,
    // Fires the waiting `tetron ping` handler when a matching `Pong` arrives.
    pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
    global_gate: Arc<crate::ratelimit::GlobalRateLimiter>,
) {
    tokio::spawn(async move {
        let mut gate = crate::ratelimit::ControlGate::new();
        loop {
            let accepted = tokio::select! {
                _ = token.cancelled() => return,
                r = conn.accept_bi() => r,
            };
            let mut recv = match accepted {
                Ok((_send, recv)) => recv,
                Err(_) => return, // connection closed
            };
            let msg = match control::recv_msg(&mut recv).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            // Throttle inbound control messages: this connection's own gate
            // plus the shared daemon-wide gate (HARDEN-004) -- drop
            // over-budget ones, and drop the peer entirely if it sustains a
            // flood.
            match gate.check_with_global(&global_gate) {
                crate::ratelimit::Verdict::Allow => {}
                crate::ratelimit::Verdict::Drop => continue,
                crate::ratelimit::Verdict::Close => {
                    tracing::warn!(peer = %remote_id.fmt_short(), "control-plane flood; closing connection");
                    conn.close(VarInt::from_u32(forward::ABUSE_CODE), b"control flood");
                    return;
                }
            }
            match msg {
                ControlMsg::Ping { nonce } => {
                    respond_pong(&conn, nonce).await;
                    continue;
                }
                ControlMsg::Pong { nonce } => {
                    if let Some((_, tx)) = pending_pongs.remove(&nonce) {
                        let _ = tx.send(());
                    }
                    continue;
                }
                // Every other control message (including MeshHello — its
                // hostname is inert since MINIMAL-014 fixed hostname at join)
                // is received but not acted on here.
                _ => {}
            }
        }
    });
}

/// Remove one identity from the roster + approved list. Does NOT publish or
/// broadcast; the caller batches that via [`finalize_removal`] so several
/// removals collapse into one publish. Used by the manual kick handler.
pub(crate) fn remove_member_roster_only(state: &SharedNetworkState, member_id: EndpointId) {
    let mut s = state.write().unwrap();
    s.members.remove(&member_id);
    s.approved.remove(&member_id);
}

/// Republish the signed blob, broadcast a payload-free `MemberSync`, and sever
/// our own link(s) to every `victim` with `KICK_CODE`. Call once after one or
/// more [`remove_member_roster_only`] edits. Other members drop the victims when
/// they reconverge from the freshly published record (`prune_departed_peers`).
pub(crate) async fn finalize_removal(
    ctx: &MeshCtx,
    network: &str,
    state: &SharedNetworkState,
    dht_notify: &Option<Arc<tokio::sync::Notify>>,
    victims: &[EndpointId],
) {
    update_snapshot_and_publish(state, &ctx.blob_store, dht_notify).await;
    broadcast_member_sync(&ctx.peers, None).await;
    for (pid, ip, conn) in ctx.peers.peers_for_network_with_conn(network) {
        if victims.contains(&pid) {
            conn.close(VarInt::from_u32(forward::KICK_CODE), b"kicked from network");
            ctx.peers
                .remove_peer_from_network(&ip, &derive_ipv6(&pid, &ctx.network_key), network);
        }
    }
}

