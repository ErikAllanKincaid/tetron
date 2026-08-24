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
    /// CONVERGE-012: what `spawn_coordinator_dial_retry` needs that isn't
    /// already threaded through `spawn_peer_cleanup`'s own `peers`/`token`
    /// params -- `ctx` alone provides `peers`/`tun_tx`/`stats`/
    /// `pruned_peers`/`network_key` (the net pubkey), so this bundle only
    /// adds what a coordinator (unlike a member) doesn't otherwise carry
    /// at this call site.
    pub(crate) endpoint: Endpoint,
    pub(crate) ctx: MeshCtx,
    pub(crate) my_identity: EndpointId,
    pub(crate) my_ip: Ipv4Addr,
    /// The coordinator's own sender half of the channel `spawn_peer_cleanup`
    /// reads from -- needed so a peer reader spawned by a successful redial
    /// can report its own eventual disconnect back into this same loop,
    /// exactly like `dial_all_members`'s success branch already does.
    pub(crate) disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
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

                                // CONVERGE-012: a coordinator previously had
                                // no outbound reconnect mechanism at all --
                                // `removed` is always true here (the early
                                // `continue` above already handled the false
                                // case), so this mirrors the member path's
                                // own gate (`join.rs::spawn_reconnect_loop`)
                                // exactly: skip a deliberate leave/kick,
                                // otherwise redial.
                                let was_pruned_locally = c
                                    .ctx
                                    .pruned_peers
                                    .remove(&(c.network_name.clone(), member_id))
                                    .is_some();
                                if reconnect_decision(removed, ev.reason.prunes_member(), was_pruned_locally)
                                    == ReconnectDecision::Reconnect
                                {
                                    spawn_coordinator_dial_retry(
                                        member_id,
                                        ev.ip,
                                        c.endpoint.clone(),
                                        c.network_name.clone(),
                                        c.my_identity,
                                        c.my_ip,
                                        c.state.clone(),
                                        c.ctx.clone(),
                                        c.disconnect_tx.clone(),
                                        token.clone(),
                                    );
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

/// Coordinator-side per-peer redial task (CONVERGE-012), analogous to
/// `join::spawn_reconnect_loop`'s inner per-peer task but simpler: a
/// coordinator already holds `state: SharedNetworkState` directly at spawn
/// time, so unlike the member path there is no `live_state_rx`/
/// `reconverge_notify_rx` oneshot handshake to wait on (that exists on the
/// member path only because `join_mesh_shared` produces that state
/// asynchronously, racing this task's own start).
///
/// Reuses CONVERGE-010/011/013's pure decision functions verbatim
/// (`dial_retry_decision`, `backoff_cap`, `next_backoff`) so roster-
/// authority and cold/frozen-escalation behavior are identical to the
/// member path regardless of which role initiated the dial. On success,
/// registers into the `PeerTable` and spawns a peer reader exactly like
/// `dial_all_members`'s own success branch (no control-reader spawn either
/// -- same asymmetry already present there: a coordinator's outbound dial
/// never spawns `spawn_coordinator_control_reader`, only inbound accepts
/// do).
#[allow(clippy::too_many_arguments)]
fn spawn_coordinator_dial_retry(
    peer_id: EndpointId,
    peer_ip: Ipv4Addr,
    endpoint: Endpoint,
    network_name: String,
    my_identity: EndpointId,
    my_ip: Ipv4Addr,
    state: SharedNetworkState,
    ctx: MeshCtx,
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    token: CancellationToken,
) {
    let MeshCtx {
        peers,
        tun_tx,
        stats,
        pruned_peers,
        network_key,
        ..
    } = ctx;
    tokio::spawn(async move {
        let peer_ipv6 = derive_ipv6(&peer_id, &network_key);
        let alpn = transport::network_alpn(&network_key);
        let mut backoff = BACKOFF_INITIAL;
        let cfg = crate::config::load().unwrap_or_default();
        let cold_threshold = cfg
            .reconnect_cold
            .threshold
            .unwrap_or(BACKOFF_COLD_THRESHOLD);
        let cold_max = std::time::Duration::from_secs(
            cfg.reconnect_cold
                .backoff_secs
                .unwrap_or(BACKOFF_COLD_MAX.as_secs()),
        );
        let frozen_threshold = cfg
            .reconnect_frozen
            .threshold
            .unwrap_or(BACKOFF_FROZEN_THRESHOLD);
        let frozen_max = std::time::Duration::from_secs(
            cfg.reconnect_frozen
                .backoff_secs
                .unwrap_or(BACKOFF_FROZEN_MAX.as_secs()),
        );
        let mut failed_attempts: u32 = 0;
        loop {
            if token.is_cancelled() {
                return;
            }
            tracing::debug!(peer = %peer_id.fmt_short(), secs = backoff.as_secs(), "coordinator reconnecting in");
            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = next_backoff(
                backoff,
                backoff_cap(
                    failed_attempts,
                    cold_threshold,
                    frozen_threshold,
                    BACKOFF_MAX,
                    cold_max,
                    frozen_max,
                ),
            );

            // CONVERGE-010's own roster-recheck, reused verbatim: an offline
            // peer never produces a fresh disconnect event for the outer
            // handler to react to, so this task must re-check itself before
            // every attempt.
            let in_roster = state.read().unwrap().members.get(&peer_id).is_some();
            let was_pruned = pruned_peers
                .remove(&(network_name.clone(), peer_id))
                .is_some();
            if dial_retry_decision(in_roster, was_pruned) == DialRetryDecision::AbandonPeerGone {
                tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "peer no longer in roster, stopping coordinator reconnect attempts");
                return;
            }
            // CONVERGE-011's live-route guard, reused verbatim: the peer may
            // have dialed us inbound while this task slept.
            if peers
                .peers_for_network_with_conn(&network_name)
                .iter()
                .any(|(eid, _, _)| *eid == peer_id)
            {
                tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "peer already reconnected inbound, stopping coordinator reconnect attempts");
                return;
            }

            match transport::connect_to_peer_with_alpn(&endpoint, peer_id, &alpn).await {
                Ok(conn) => {
                    let Ok((mut send, _)) = conn.open_bi().await else {
                        tracing::warn!(peer = %peer_id.fmt_short(), "coordinator reconnect handshake failed");
                        failed_attempts += 1;
                        continue;
                    };
                    if let Err(e) = control::send_msg(
                        &mut send,
                        &ControlMsg::MeshHello {
                            identity: my_identity,
                            ip: my_ip,
                            hostname: outgoing_hostname(&network_name),
                        },
                    )
                    .await
                    {
                        tracing::warn!(peer = %peer_id.fmt_short(), error = %e, "coordinator reconnect MeshHello failed");
                        failed_attempts += 1;
                        continue;
                    }
                    tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "coordinator reconnected to member");
                    peers.add(peer_ip, peer_ipv6, conn.clone(), peer_id, &network_name);
                    forward::spawn_peer_reader(
                        conn,
                        peer_id,
                        peer_ip,
                        peer_ipv6,
                        network_name,
                        state.read().unwrap().subnet,
                        forward::ForwardCtx {
                            tun_tx,
                            disconnect_tx,
                            token,
                            stats,
                        },
                    );
                    return;
                }
                Err(e) => {
                    tracing::debug!(peer = %peer_id.fmt_short(), error = %e, "coordinator reconnect attempt failed");
                    failed_attempts += 1;
                }
            }
        }
    });
}

/// Coordinator-side per-member control reader. Continuously accepts control
/// streams from one member and answers `Ping`; every other message (including
/// `MeshHello` — hostname is fixed at join, MINIMAL-014 removed rename
/// propagation) is received but not acted on. Runs until the network token is
/// cancelled or the connection drops.
pub(crate) fn spawn_coordinator_control_reader(
    conn: Connection,
    remote_id: EndpointId,
    _peer_ip: Ipv4Addr,
    _network_name: String,
    token: CancellationToken,
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
            // Every other control message (including an inbound Pong, and
            // MeshHello — whose hostname is inert since MINIMAL-014 fixed
            // hostname at join) is received but not acted on here.
            if let ControlMsg::Ping { nonce } = msg {
                respond_pong(&conn, nonce).await;
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
