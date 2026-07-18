//! Mesh join handshake and reconnect loop. Moved out of `daemon/mod.rs` to keep
//! the core module focused on type definitions and process wiring.
//!
//! `join_mesh_shared` runs one coordinator handshake (fresh join sends
//! `JoinRequest` first; reconnect/restore lets the coordinator speak first) and,
//! on admission, registers the peer and starts its data-plane reader.
//! `spawn_reconnect_loop` keeps a member's connection alive with backoff.

use super::super::*;
use crate::config::TransportMode;

/// Result of the initial join handshake against the coordinator.
pub(crate) enum JoinResult {
    /// Admitted (open network, valid invite, or pre-approved): live network state
    /// and the reconverge-notify handle created inside `join_mesh_shared`.
    Joined(SharedNetworkState, Arc<tokio::sync::Notify>),
    /// Queued for live approval on a closed network; the caller should retry.
    Pending,
}

/// Outcome of one `join_network_inner` attempt.
pub(crate) enum TryJoin {
    Joined(IpcMessage),
    Pending,
}

/// Result of [`perform_join_handshake`]: the admitted roster, or a closed-network
/// queue signal the caller turns into [`JoinResult::Pending`].
enum HandshakeOutcome {
    Admitted {
        members: Vec<crate::membership::Member>,
        approved: Vec<ApprovedEntry>,
    },
    Pending,
}

/// By-value parameters for one [`join_mesh_shared`] handshake, grouped so the
/// function's argument list stays manageable. These are all decided once, at the
/// call site, per join: the joiner's chosen hostname, the invite secret
/// it presents, the blob-derived `reusable_keys` it inherits, and whether
/// this is a fresh join or a reconnect.
pub(crate) struct JoinParams {
    pub(crate) my_hostname: Option<String>,
    pub(crate) net_pubkey: EndpointId,
    pub(crate) invite_secret: Option<Vec<u8>>,
    /// From the fetched blob: reusable join keys, so this node can validate
    /// redemptions if it later holds the network key (HA admission).
    pub(crate) reusable_keys: BTreeMap<String, crate::membership::ReusableKey>,
    /// From the fetched blob: single-use invite entries, so this node can mint
    /// invites and validate redemptions if it later holds the network key.
    pub(crate) invites: BTreeMap<String, crate::membership::InviteEntry>,
    /// From the fetched blob: pending nuke proposals (NUKE-CONSENSUS), carried
    /// into the joiner's state purely for `tetron status` visibility.
    pub(crate) nuke_proposals: BTreeMap<String, u64>,
    /// From the fetched blob: its generation (CONVERGE-005), adopted directly
    /// into the joiner's initial `NetworkState` (never bumped — this node hasn't
    /// mutated anything yet). Only load-bearing if this node later publishes
    /// (promoted to co-coordinator); a plain member's own generation is
    /// otherwise cosmetic and self-corrects on the next reconverge.
    pub(crate) generation: u64,
    /// Per-network transport preference (None = default, Some(Tor) = Tor routed).
    pub(crate) transport: Option<TransportMode>,
    /// Fresh join (send `JoinRequest` first) vs reconnect/restore (coordinator
    /// speaks first).
    pub(crate) initial: bool,
    /// This node's own IP in this network (MULTISEG-004's per-network
    /// derivation, already resolved by the caller from the network's own
    /// blob-carried subnet). Threaded through rather than recomputed here via
    /// `identity.local_ip()`, which is bound to the daemon's single node-wide
    /// identity subnet and is wrong whenever this network's subnet differs
    /// from it (found live-testing MULTISEG-002..006: a `--subnet`-diverging
    /// network's join persisted the wrong `my_ip` to config).
    pub(crate) my_ip: Ipv4Addr,
    /// This network's own subnet (MULTISEG-001/004), already resolved by the
    /// caller from the fetched blob. Threaded through to `build_member_state`
    /// instead of that function defaulting to the node-wide subnet
    /// (`config::node_subnet()`) — a stale SUBNET-010-era assumption from
    /// before multi-segment TUN made subnets per-network. Every other
    /// `NetworkState` construction site was updated for this during
    /// `MULTISEG-004`; this one was missed (found live 2026-07-18 debugging
    /// `MACOS-001`).
    pub(crate) network_subnet: crate::membership::Subnet,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn join_mesh_shared(
    initial_conn: Connection,
    ep: &Endpoint,
    network_name: &str,
    alpn: &[u8],
    ctx: MeshCtx,
    params: JoinParams,
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    token: CancellationToken,
    // Promotion signal: the per-peer control reader sends this network's name
    // here after persisting an `AdminGrant` key, so the daemon loop can swap in
    // the coordinator accept handler (see `MeshManager::promote_to_coordinator`).
    promote_tx: mpsc::Sender<String>,
    // Self-removal signal (CONVERGE-003): the debounced reconverge worker sends
    // this network's name here if it discovers we've been dropped from the
    // authoritative roster, so the daemon loop can leave it locally.
    left_tx: mpsc::Sender<String>,
    // Shared with the router; lets the member control reader resolve `tetron ping`
    // Pongs back to the waiting handler.
    pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
) -> Result<JoinResult> {
    // A whole-bundle clone for the debounced reconverge worker, which forwards
    // the ctx straight to `reconverge_and_apply`.
    let worker_ctx = ctx.clone();
    let MeshCtx {
        identity,
        peers,
        blob_store,
        ..
    } = ctx;
    let JoinParams {
        my_hostname,
        net_pubkey,
        invite_secret,
        reusable_keys,
        invites,
        nuke_proposals,
        generation,
        transport,
        initial,
        my_ip,
        network_subnet,
    } = params;
    let my_identity = identity.local_identity();

    let (members, approved) = match perform_join_handshake(
        &initial_conn,
        ep,
        network_name,
        &blob_store,
        &peers,
        net_pubkey,
        my_ip,
        my_identity,
        initial,
        invite_secret,
        &my_hostname,
    )
    .await?
    {
        HandshakeOutcome::Admitted { members, approved } => (members, approved),
        HandshakeOutcome::Pending => return Ok(JoinResult::Pending),
    };

    persist_join_config(
        network_name,
        &members,
        &approved,
        my_identity,
        my_ip,
        net_pubkey,
        &my_hostname,
        transport,
    )?;

    // On reconnect/restore the coordinator hasn't seen our hostname this session,
    // so send a MeshHello. A fresh join already conveyed it in the JoinRequest.
    if !initial {
        send_reconnect_hello(&initial_conn, my_identity, my_ip, network_name).await?;
    }

    // Register the coordinator connection as our first peer, then dial the rest
    // of the roster.
    let remote_id = initial_conn.remote_id();
    // The coordinator's IP comes from the just-admitted roster (authoritative,
    // network-scoped), not `identity.derive_ip` (bound to this daemon's single
    // node-wide identity subnet, wrong whenever this network's subnet differs
    // from it — found live-testing MULTISEG-002..006: this produced an
    // anti-spoof false-positive that dropped every real packet from the
    // coordinator on a `--subnet`-diverging network). The coordinator is
    // always present in its own roster, so the fallback below is defensive
    // only, never expected to trigger.
    let remote_ip = members
        .iter()
        .find(|m| m.identity == remote_id)
        .map(|m| m.ip)
        .unwrap_or_else(|| identity.derive_ip(&remote_id));
    crate::spawn_path_logger(initial_conn.clone(), remote_id.fmt_short().to_string());
    register_mesh_peer(
        &peers,
        &worker_ctx,
        &disconnect_tx,
        &token,
        initial_conn.clone(),
        remote_id,
        remote_ip,
        network_name,
    );
    // Dial the rest of the roster in the background (DIAL-001). The
    // coordinator link is already registered above, so the network is usable
    // now; blocking the join on the full mesh means one slow or dead member
    // (e.g. a stale offline peer whose discovery record still resolves)
    // stalls the join for that peer's whole dial timeout. Peer links fill in
    // as they connect; the reconnect loop recovers any that time out.
    spawn_roster_peer_dials(
        ep.clone(),
        alpn.to_vec(),
        members.clone(),
        network_name.to_string(),
        my_identity,
        my_ip,
        remote_id,
        worker_ctx.clone(),
        disconnect_tx.clone(),
        token.clone(),
    );

    let live_state = build_member_state(
        members,
        approved,
        net_pubkey,
        network_name,
        reusable_keys,
        invites,
        nuke_proposals,
        generation,
        network_subnet,
        &blob_store,
    )
    .await;

    // Reconverge worker: `MemberSync`/`BlobUpdated` triggers fan into this single
    // debounced task (see `spawn_reconverge_worker`).
    let reconverge_notify = Arc::new(tokio::sync::Notify::new());
    spawn_reconverge_worker(
        reconverge_notify.clone(),
        token.clone(),
        live_state.clone(),
        network_name.to_string(),
        worker_ctx,
        ep.clone(),
        my_identity,
        net_pubkey,
        alpn.to_vec(),
        my_ip,
        left_tx.clone(),
    );

    spawn_member_control_listener(
        initial_conn.clone(),
        remote_id,
        token.clone(),
        live_state.clone(),
        network_name.to_string(),
        peers.clone(),
        ep.clone(),
        my_identity,
        net_pubkey,
        promote_tx.clone(),
        reconverge_notify.clone(),
        pending_pongs.clone(),
    );

    Ok(JoinResult::Joined(live_state, reconverge_notify))
}

/// The hostname this node should announce to peers for `network_name`: its
/// persisted (join-fixed) name, read fresh from config. Hostname rename was
/// removed (MINIMAL-014), so this is simply `my_hostname`.
pub(crate) fn outgoing_hostname(network_name: &str) -> Option<String> {
    match config::load_network(network_name) {
        Ok(Some(net)) => net.my_hostname,
        _ => None,
    }
}

/// Persist this network's membership to config after a successful handshake.
/// Preserves the `direct` flag from the existing config (the freshly fetched
/// blob doesn't carry it).
#[allow(clippy::too_many_arguments)]
fn persist_join_config(
    network_name: &str,
    members: &[crate::membership::Member],
    approved: &[ApprovedEntry],
    my_identity: EndpointId,
    my_ip: Ipv4Addr,
    net_pubkey: EndpointId,
    my_hostname: &Option<String>,
    transport: Option<TransportMode>,
) -> Result<()> {
    let persisted_hostname = members
        .iter()
        .find(|m| m.identity == my_identity)
        .and_then(|m| m.hostname.clone())
        .or(my_hostname.clone());
    let direct = config::load_network(network_name)?
        .map(|n| n.direct)
        .unwrap_or(false);
    config::save_network(&config::NetworkConfig {
        name: network_name.to_string(),
        group_mode: GroupMode::Restricted,
        my_ip: Some(my_ip),
        my_hostname: persisted_hostname,
        members: to_member_entries(members.iter()),
        approved: to_approved_entries(approved.iter()),
        network_secret_key: None,
        network_public_key: Some(net_pubkey),
        transport,
        admins: vec![],
        direct,
        subnet: None,
    })
}

/// Send a `MeshHello` to the coordinator on reconnect/restore (a fresh join
/// already conveyed the hostname in its `JoinRequest`). Reads the hostname fresh
/// from config (its join-fixed name).
async fn send_reconnect_hello(
    conn: &Connection,
    my_identity: EndpointId,
    my_ip: Ipv4Addr,
    network_name: &str,
) -> Result<()> {
    let (mut send, _recv) = conn.open_bi().await?;
    control::send_msg(
        &mut send,
        &ControlMsg::MeshHello {
            identity: my_identity,
            ip: my_ip,
            hostname: outgoing_hostname(network_name),
            device_cert: None,
        },
    )
    .await
}

/// Build the in-memory `NetworkState` cell for a joined member from the admitted
/// roster + blob-derived keys, refresh its snapshot, and seed the local
/// blob store with those bytes.
#[allow(clippy::too_many_arguments)]
async fn build_member_state(
    members: Vec<crate::membership::Member>,
    approved: Vec<ApprovedEntry>,
    net_pubkey: EndpointId,
    network_name: &str,
    reusable_keys: BTreeMap<String, crate::membership::ReusableKey>,
    invites: BTreeMap<String, crate::membership::InviteEntry>,
    nuke_proposals: BTreeMap<String, u64>,
    generation: u64,
    subnet: crate::membership::Subnet,
    blob_store: &FsStore,
) -> SharedNetworkState {
    let mut ns = NetworkState {
        generation,
        members: MemberList::from_members(members),
        approved: ApprovedList::from_entries(approved),
        snapshot: None,
        network_secret_key: None,
        network_public_key: net_pubkey,
        network_name: Some(network_name.to_string()),
        mode: GroupMode::Restricted,
        // MULTISEG-004: this network's own subnet, already resolved by the
        // caller from the fetched blob — not the node-wide default
        // (see JoinParams::network_subnet's doc comment for how this was
        // found: found live 2026-07-18, this site was missed during the
        // original MULTISEG-004 sweep even though every other NetworkState
        // construction site was updated).
        subnet,
        reusable_keys,
        invites,
        nuke_proposals,
    };
    ns.refresh_snapshot();
    if let Some(snap) = &ns.snapshot {
        let _ = blob_store.blobs().add_slice(&snap.msgpack_bytes).await;
    }
    Arc::new(std::sync::RwLock::new(ns))
}

/// Add a peer's route to the table and start its data-plane reader. Shared by the
/// initial coordinator connection and each roster member dialed afterward.
#[allow(clippy::too_many_arguments)]
fn register_mesh_peer(
    peers: &PeerTable,
    ctx: &MeshCtx,
    disconnect_tx: &mpsc::Sender<forward::DisconnectEvent>,
    token: &CancellationToken,
    conn: Connection,
    peer_id: EndpointId,
    peer_ip: Ipv4Addr,
    network_name: &str,
) {
    let peer_ipv6 = derive_ipv6(&peer_id, &ctx.network_key);
    peers.add(peer_ip, peer_ipv6, conn.clone(), peer_id, network_name);
    forward::spawn_peer_reader(
        conn,
        peer_id,
        peer_ip,
        peer_ipv6,
        network_name.to_string(),
        ctx.forward_ctx(disconnect_tx.clone(), token.clone()),
    );
}

/// Upper bound on a single background roster dial (DIAL-001). Generous on
/// purpose: the dial runs off the join path, so a slow-but-live member
/// (relay plus NAT holepunch on a flaky link) is worth waiting for, while a
/// truly dead member is still bounded instead of lingering on iroh's own
/// internal handshake timeout.
const MESH_PEER_DIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Dial every other roster member (skipping ourselves and the
/// already-connected coordinator) concurrently in the background, sending
/// each a `MeshHello` and registering it as a peer (DIAL-001). Best-effort and
/// non-blocking: this is spawned so the join/reconnect completes as soon as
/// the coordinator link is up, and a member that's offline or stale is
/// bounded by [`MESH_PEER_DIAL_TIMEOUT`] and simply logged rather than
/// stalling the whole join. The owned arguments let the task outlive the join
/// call.
#[allow(clippy::too_many_arguments)]
fn spawn_roster_peer_dials(
    ep: Endpoint,
    alpn: Vec<u8>,
    members: Vec<crate::membership::Member>,
    network_name: String,
    my_identity: EndpointId,
    my_ip: Ipv4Addr,
    skip_id: EndpointId,
    ctx: MeshCtx,
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    token: CancellationToken,
) {
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut dials = futures::stream::FuturesUnordered::new();
        for member in &members {
            if member.identity == my_identity || member.identity == skip_id {
                continue;
            }
            // Borrow the owned task-locals into each future; the values live
            // for the whole `while dials.next()` drain below.
            let (ep, alpn, ctx) = (&ep, &alpn, &ctx);
            let (disconnect_tx, token) = (&disconnect_tx, &token);
            let network_name = &network_name;
            dials.push(async move {
                // Bound the dial and honor cancellation so one unreachable
                // member can't keep this task alive far longer than the dial
                // is worth.
                let conn = tokio::select! {
                    _ = token.cancelled() => return,
                    r = tokio::time::timeout(
                        MESH_PEER_DIAL_TIMEOUT,
                        transport::connect_to_peer_with_alpn(ep, member.identity, alpn),
                    ) => r,
                };
                match conn {
                    Ok(Ok(conn)) => {
                        let mut send = match conn.open_bi().await {
                            Ok((send, _recv)) => send,
                            Err(e) => {
                                tracing::warn!(peer_ip = %member.ip, error = %e, "mesh peer stream open failed");
                                return;
                            }
                        };
                        if let Err(e) = control::send_msg(
                            &mut send,
                            &ControlMsg::MeshHello {
                                identity: my_identity,
                                ip: my_ip,
                                hostname: outgoing_hostname(network_name),
                                device_cert: None,
                            },
                        )
                        .await
                        {
                            tracing::warn!(peer_ip = %member.ip, error = %e, "mesh peer hello failed");
                            return;
                        }
                        register_mesh_peer(
                            &ctx.peers,
                            ctx,
                            disconnect_tx,
                            token,
                            conn,
                            member.identity,
                            member.ip,
                            network_name,
                        );
                        tracing::info!(peer_ip = %member.ip, "connected to mesh peer");
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(peer_ip = %member.ip, error = %e, "mesh peer unavailable");
                    }
                    Err(_elapsed) => {
                        tracing::warn!(
                            peer_ip = %member.ip,
                            timeout_secs = MESH_PEER_DIAL_TIMEOUT.as_secs(),
                            "mesh peer dial timed out"
                        );
                    }
                }
            });
        }
        while dials.next().await.is_some() {}
    });
}

/// Run one coordinator handshake. A fresh join (`initial`) opens a stream, sends
/// a `JoinRequest` (invite secret + hostname), and reads the verdict on the same
/// stream. A reconnect/restore keeps the legacy handshake where the coordinator
/// speaks first (Welcome/JoinApproved/MemberSync) — on a `MemberSync` trigger the
/// roster comes from the network-key-signed pkarr record, never peer-supplied
/// membership. Returns the admitted roster, or `Pending` on a closed network.
#[allow(clippy::too_many_arguments)]
async fn perform_join_handshake(
    initial_conn: &Connection,
    ep: &Endpoint,
    network_name: &str,
    blob_store: &FsStore,
    peers: &PeerTable,
    net_pubkey: EndpointId,
    my_ip: Ipv4Addr,
    my_identity: EndpointId,
    initial: bool,
    invite_secret: Option<Vec<u8>>,
    my_hostname: &Option<String>,
) -> Result<HandshakeOutcome> {
    if initial {
        let (mut send, mut recv) = initial_conn
            .open_bi()
            .await
            .context("open join control stream")?;
        control::send_msg(
            &mut send,
            &ControlMsg::JoinRequest {
                invite_secret,
                hostname: my_hostname.clone(),
                device_cert: None,
            },
        )
        .await
        .context("send join request")?;
        let msg = tokio::time::timeout(Duration::from_secs(30), control::recv_msg(&mut recv))
            .await
            .context("timeout awaiting join response")??;
        match msg {
            ControlMsg::Welcome { members, approved } => {
                tracing::info!(network = %network_name, "welcomed to network");
                if let Some(existing) = members
                    .iter()
                    .find(|m| m.ip == my_ip && m.identity != my_identity)
                {
                    anyhow::bail!(
                        "IP collision: {} is already assigned to {}",
                        my_ip,
                        existing.identity
                    );
                }
                Ok(HandshakeOutcome::Admitted { members, approved })
            }
            ControlMsg::JoinPending => {
                tracing::info!(network = %network_name, "join pending operator approval");
                Ok(HandshakeOutcome::Pending)
            }
            ControlMsg::JoinDenied { reason } => anyhow::bail!("join denied: {reason}"),
            other => anyhow::bail!("expected Welcome or JoinPending, got {other:?}"),
        }
    } else {
        let (_send, mut recv) = initial_conn
            .accept_bi()
            .await
            .context("accept control stream")?;
        let msg = control::recv_msg(&mut recv).await?;
        let (members, approved) = match msg {
            ControlMsg::Welcome { members, approved } => {
                tracing::info!(network = %network_name, "welcomed to network");
                (members, approved)
            }
            ControlMsg::JoinApproved { your_ip, members } => {
                tracing::info!(ip = %your_ip, network = %network_name, "joined network (legacy)");
                (members, vec![])
            }
            ControlMsg::MemberSync => {
                // Reconnected via a peer. The message is only a trigger — fetch
                // the authoritative roster from the network-key-signed pkarr
                // record. If it's briefly unreachable, fall back to our last
                // persisted roster rather than trusting peer-supplied membership.
                tracing::info!(network = %network_name, "reconnected via peer; reconverging from signed record");
                match resolve_signed(ep, net_pubkey).await {
                    Some((signed, _generation, seeds)) => {
                        match fetch_verified_blob(
                            ep,
                            blob_store,
                            peers,
                            signed,
                            network_name,
                            &seeds,
                        )
                        .await
                        {
                            Some(data) => (data.members, data.approved),
                            None => (persisted_roster(network_name), vec![]),
                        }
                    }
                    None => (persisted_roster(network_name), vec![]),
                }
            }
            ControlMsg::JoinDenied { reason } => anyhow::bail!("join denied: {reason}"),
            other => anyhow::bail!("expected Welcome or MemberSync, got {other:?}"),
        };
        Ok(HandshakeOutcome::Admitted { members, approved })
    }
}

/// Debounced reconverge worker for a joined member. `MemberSync`/`BlobUpdated`
/// triggers fan into this single task instead of each driving a reconverge
/// inline: a burst of triggers collapses into one pkarr resolve + reconverge,
/// and a slow reconverge never blocks the control listener's accept loop. The
/// network-key-signed record stays the source of truth, so converging once per
/// burst suffices.
#[allow(clippy::too_many_arguments)]
fn spawn_reconverge_worker(
    notify: Arc<tokio::sync::Notify>,
    token: CancellationToken,
    live_state: SharedNetworkState,
    network_name: String,
    ctx_w: MeshCtx,
    endpoint_w: Endpoint,
    my_identity_w: EndpointId,
    net_pubkey_w: EndpointId,
    alpn_w: Vec<u8>,
    my_ip_w: Ipv4Addr,
    left_tx: mpsc::Sender<String>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                _ = notify.notified() => {}
            }
            // Debounce: absorb a burst of triggers into a single reconverge.
            // A trigger that arrives during the sleep or the reconverge is
            // retained by `Notify` and handled on the next iteration.
            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
            }
            reconverge_and_apply(
                &endpoint_w,
                &ctx_w,
                net_pubkey_w,
                &network_name,
                &live_state,
                my_identity_w,
                &alpn_w,
                my_ip_w,
                &left_tx,
            )
            .await;
        }
    });
}

/// Per-connection control listener for a joined member: reads control messages
/// off the coordinator connection under a [`ControlGate`] rate limit and applies
/// each (approval, reconverge triggers, `AdminGrant` promotion, ping/pong).
/// Roster/firewall state comes only from the signed pkarr record, so
/// `MemberSync`/`BlobUpdated` are mere triggers into the reconverge worker.
#[allow(clippy::too_many_arguments)]
fn spawn_member_control_listener(
    initial_conn: Connection,
    remote_id: EndpointId,
    token: CancellationToken,
    live_state: SharedNetworkState,
    network_name: String,
    peers_c: PeerTable,
    endpoint_c: Endpoint,
    my_identity_c: EndpointId,
    net_pubkey_c: EndpointId,
    promote_tx: mpsc::Sender<String>,
    reconverge_notify: Arc<tokio::sync::Notify>,
    pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
) {
    tokio::spawn(async move {
        let mut gate = crate::ratelimit::ControlGate::new();
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                result = initial_conn.accept_bi() => {
                    match result {
                        Ok((_send, mut recv)) => {
                            let msg = match control::recv_msg(&mut recv).await {
                                Ok(m) => m,
                                Err(_) => continue,
                            };
                            // Throttle inbound control messages per connection:
                            // drop over-budget ones, drop the peer on a flood.
                            match gate.check() {
                                crate::ratelimit::Verdict::Allow => {}
                                crate::ratelimit::Verdict::Drop => continue,
                                crate::ratelimit::Verdict::Close => {
                                    tracing::warn!(peer = %remote_id.fmt_short(), "control-plane flood; closing connection");
                                    initial_conn.close(VarInt::from_u32(forward::ABUSE_CODE), b"control flood");
                                    return;
                                }
                            }
                            match msg {
                                ControlMsg::MemberApproved { identity, ip, hostname, .. } => {
                                    let entry = ApprovedEntry { identity, ip, hostname, user_identity: None, device_cert: None, collision_index: 0 };
                                    let mut s = live_state.write().unwrap();
                                    let members = s.members.clone();
                                    let _ = s.approved.approve(entry, &members);
                                }
                                ControlMsg::MemberSync => {
                                    // Trigger only. The roster/firewall come exclusively
                                    // from the network-key-signed pkarr record, never from
                                    // peer-supplied membership. Coalesced into the debounced
                                    // reconverge worker.
                                    reconverge_notify.notify_one();
                                }
                                ControlMsg::BlobUpdated => {
                                    // Trigger only. Reconverge from the network-key-signed
                                    // pkarr record — a malicious member can't inject a
                                    // forged roster/firewall blob via this message. Coalesced
                                    // into the debounced reconverge worker.
                                    reconverge_notify.notify_one();
                                }
                                ControlMsg::AdminGrant { network_pubkey, secret_key } => {
                                    // Coordinator granted us the per-network key.
                                    // Verify it targets this network (the stream is
                                    // already ALPN-scoped, but defense in depth).
                                    if network_pubkey != net_pubkey_c {
                                        tracing::warn!(
                                            peer = %remote_id.fmt_short(),
                                            "admin grant for a different network; ignoring"
                                        );
                                        continue;
                                    }
                                    // Self-authenticating: only adopt a key
                                    // that genuinely is this network's key
                                    // (its public half must equal the network
                                    // pubkey). Defeats a forged AdminGrant
                                    // from a non-coordinator member without
                                    // relying on reconverge timing for the
                                    // granter's is_coordinator flag.
                                    if !admin_grant_key_valid(secret_key, net_pubkey_c) {
                                        tracing::warn!(
                                            peer = %remote_id.fmt_short(),
                                            "admin grant key does not match network pubkey; ignoring"
                                        );
                                        continue;
                                    }
                                    let key = SecretKey::from(secret_key);
                                    // Persist + take local publish capability.
                                    if let Ok(Some(mut net)) = config::load_network(&network_name) {
                                        net.network_secret_key = Some(key.clone());
                                        let _ = config::save_network(&net);
                                    }
                                    let endpoint_id = endpoint_c.id();
                                    {
                                        let mut s = live_state.write().unwrap();
                                        s.network_secret_key = Some(key.clone());
                                        if let Some(m) = s.members.get_mut(&my_identity_c) {
                                            m.is_coordinator = true;
                                        }
                                        s.refresh_snapshot();
                                    }
                                    // Spawn a lazy publisher (this node can now
                                    // publish the signed blob / suggest rules).
                                    if let Ok(client) = dht::create_pkarr_client(&endpoint_c) {
                                        spawn_lazy_publisher(
                                            client,
                                            key,
                                            live_state.clone(),
                                            endpoint_id,
                                            peers_c.clone(),
                                            network_name.clone(),
                                            token.clone(),
                                        );
                                        tracing::info!(
                                            network = %network_name,
                                            "promoted to co-coordinator; lazy publisher started"
                                        );
                                    }
                                    // Signal the daemon loop to swap this
                                    // network's accept handler to coordinator
                                    // so it can admit fresh joiners (not just
                                    // welcome pre-approved peers). The loop
                                    // holds the `Arc<MeshManager>` this task
                                    // does not. Best-effort: a closed channel
                                    // only means the daemon is shutting down.
                                    let _ = promote_tx.send(network_name.clone()).await;
                                }
                                ControlMsg::Ping { nonce } => {
                                    respond_pong(&initial_conn, nonce).await;
                                }
                                ControlMsg::Pong { nonce } => {
                                    if let Some((_, tx)) = pending_pongs.remove(&nonce) {
                                        let _ = tx.send(());
                                    }
                                }
                                _ => {}
                            }
                        }
                        Err(_) => return,
                    }
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_reconnect_loop(
    mut disconnect_rx: mpsc::Receiver<forward::DisconnectEvent>,
    ep: Endpoint,
    alpn: Vec<u8>,
    network_name: String,
    my_identity: EndpointId,
    my_ip: Ipv4Addr,
    ctx: MeshCtx,
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    token: CancellationToken,
    // Control-listener resources: the original listener dies with the connection,
    // so we must spawn a fresh one on each reconnect. These are delivered via
    // oneshot because they only exist after join_mesh_shared completes — forward
    // readers (and thus disconnect events) are spawned inside join_mesh_shared, so
    // no disconnect can arrive before these are sent.
    live_state_rx: tokio::sync::oneshot::Receiver<SharedNetworkState>,
    reconverge_notify_rx: tokio::sync::oneshot::Receiver<Arc<tokio::sync::Notify>>,
    promote_tx: mpsc::Sender<String>,
    pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
) -> JoinHandle<()> {
    // The reconnect MeshHello reads the current hostname fresh from config
    // (`outgoing_hostname`), so no captured hostname is threaded through.
    let MeshCtx {
        peers,
        tun_tx,
        stats,
        pruned_peers,
        ..
    } = ctx;
    use tracing::Instrument as _;
    // Tag all reconnect-loop logs for this network so they correlate in reports.
    let span = tracing::info_span!("reconnect", net = %network_name);
    let reconnect_loop = async move {
        // Wait for join_mesh_shared to complete before processing disconnects.
        // Forward readers (and therefore any disconnect event) are only spawned
        // inside join_mesh_shared, so this always resolves before any event.
        let live_state = match live_state_rx.await {
            Ok(s) => s,
            Err(_) => return, // daemon shut down
        };
        let reconverge_notify = match reconverge_notify_rx.await {
            Ok(n) => n,
            Err(_) => return, // daemon shut down
        };
        loop {
            let event = tokio::select! {
                _ = token.cancelled() => return,
                event = disconnect_rx.recv() => match event {
                    Some(ev) => ev,
                    None => return,
                },
            };
            let peer_id = event.endpoint_id;
            let peer_ip = event.ip;
            let peer_ipv6 = event.ipv6;
            // Drop only this network's route, and only if the stored connection
            // is still the one that died. If the peer already re-dialed and a
            // fresh connection is registered, this is a stale disconnect for the
            // old connection: ignore it entirely rather than tearing down the
            // live link and redialing on top of it (see conn_stable_id).
            let removed = match event.conn_stable_id {
                Some(id) => {
                    peers.remove_peer_from_network_if(&peer_ip, &peer_ipv6, &event.network, id)
                }
                None => {
                    // Synthetic cold-restore kick: nothing is registered yet, so
                    // force the reconnect dial below.
                    peers.remove_peer_from_network(&peer_ip, &peer_ipv6, &event.network);
                    true
                }
            };
            if !removed {
                tracing::debug!(peer = %peer_id.fmt_short(), ip = %peer_ip, "ignoring stale disconnect; peer already reconnected");
                continue;
            }

            // A deliberate `tetron leave` (graceful close with the leave code) means
            // the peer is gone for good — don't spin a reconnect task against it.
            // The coordinator's MemberSync will prune it from our roster. Narrowed
            // to a genuine leave only (CONVERGE-007): a KICK_CODE close falls
            // through to the `pruned_peers` check below instead, since
            // `prune_departed_peers` sends that code from any node's own (possibly
            // transiently stale) view of the roster, not just a real kick — the
            // signed-roster-driven `pruned_peers` set is the correct arbiter for
            // whether to actually stop reconnecting, not the close code alone.
            if event.reason.prunes_member() {
                tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "peer left, not reconnecting");
                continue;
            }
            // We just pruned this peer from the roster (it was kicked or departed)
            // and closed the connection ourselves — that close is what woke this
            // loop. The peer still lists us, so re-dialing would re-form the link.
            // Consume the one-shot suppression entry and skip.
            if pruned_peers
                .remove(&(network_name.clone(), peer_id))
                .is_some()
            {
                tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "peer removed from roster, not reconnecting");
                continue;
            }
            tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "peer disconnected, will reconnect");

            let ep = ep.clone();
            let alpn = alpn.clone();
            let network_name = network_name.clone();
            let peers = peers.clone();
            let tun_tx = tun_tx.clone();
            let disconnect_tx = disconnect_tx.clone();
            let token = token.clone();
            let stats = stats.clone();
            let promote_tx = promote_tx.clone();
            let reconverge_notify = reconverge_notify.clone();
            let pending_pongs = pending_pongs.clone();
            let live_state = live_state.clone();

            tokio::spawn(async move {
                let mut backoff = BACKOFF_INITIAL;
                let net_name = network_name.clone();
                loop {
                    if token.is_cancelled() {
                        return;
                    }
                    tracing::info!(peer = %peer_id.fmt_short(), secs = backoff.as_secs(), "reconnecting in");
                    tokio::select! {
                        _ = token.cancelled() => return,
                        _ = tokio::time::sleep(backoff) => {}
                    }
                    backoff = (backoff * 2).min(BACKOFF_MAX);

                    match transport::connect_to_peer_with_alpn(&ep, peer_id, &alpn).await {
                        Ok(conn) => {
                            let (mut send, _) = match conn.open_bi().await {
                                Ok(bi) => bi,
                                Err(e) => {
                                    tracing::warn!(error = %e, "reconnect handshake failed");
                                    continue;
                                }
                            };
                            if let Err(e) = control::send_msg(
                                &mut send,
                                &ControlMsg::MeshHello {
                                    identity: my_identity,
                                    ip: my_ip,
                                    hostname: outgoing_hostname(&net_name),
                                    device_cert: None,
                                },
                            )
                            .await
                            {
                                tracing::warn!(error = %e, "reconnect MeshHello failed");
                                continue;
                            }
                            tracing::info!(peer = %peer_id.fmt_short(), ip = %peer_ip, "reconnected to peer");
                            peers.add(peer_ip, peer_ipv6, conn.clone(), peer_id, &net_name);
                            // Spawn a fresh control listener on the new connection
                            // (the old one died when the connection dropped).
                            {
                                let cl_live = live_state.clone();
                                let cl_net_pubkey = cl_live.read().unwrap().network_public_key;
                                spawn_member_control_listener(
                                    conn.clone(),
                                    peer_id,
                                    token.clone(),
                                    cl_live,
                                    net_name.clone(),
                                    peers.clone(),
                                    ep.clone(),
                                    my_identity,
                                    cl_net_pubkey,
                                    promote_tx.clone(),
                                    reconverge_notify.clone(),
                                    pending_pongs.clone(),
                                );
                            }
                            forward::spawn_peer_reader(
                                conn,
                                peer_id,
                                peer_ip,
                                peer_ipv6,
                                net_name,
                                forward::ForwardCtx {
                                    tun_tx,
                                    disconnect_tx,
                                    token: token.clone(),
                                    stats,
                                },
                            );
                            return;
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "reconnect attempt failed");
                        }
                    }
                }
            });
        }
    };
    tokio::spawn(reconnect_loop.instrument(span))
}
