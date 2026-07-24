//! Connection-accept machinery for the mesh core. Moved out of `daemon/mod.rs`.
//!
//! Holds the per-network accept handlers (`CoordinatorAcceptState` admits
//! joiners via invite keys), the `AcceptHandler` enum the router dispatches
//! through, and the `ProtocolRouter` that fans incoming connections out by
//! ALPN (mesh handlers plus the `blobs`/`files`/`pair` arms). `MeshCtx`
//! and the roster-projection helpers stay in `daemon/mod.rs` since they are
//! shared infrastructure.

use super::super::*;

pub(crate) struct CoordinatorAcceptState {
    pub(crate) ctx: MeshCtx,
    pub(crate) network_name: String,
    pub(crate) state: SharedNetworkState,
    pub(crate) disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    pub(crate) token: CancellationToken,
    pub(crate) dht_notify: Option<Arc<tokio::sync::Notify>>,
    /// Shared with the router; lets the control reader resolve `tetron ping` Pongs.
    pub(crate) pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
}

impl CoordinatorAcceptState {
    /// Fast path for a known member reconnecting: re-add its route, send a
    /// `MemberSync`, and spawn the control reader + peer reader. `peer_ip` carries
    /// the member's stored collision index (not a fresh index-0 derivation).
    fn handle_known_member_reconnect(
        &self,
        conn: Connection,
        remote_id: EndpointId,
        peer_ip: Ipv4Addr,
    ) {
        tracing::info!(ip = %peer_ip, "known member reconnecting");
        crate::spawn_path_logger(conn.clone(), remote_id.fmt_short().to_string());
        let peer_ipv6 = derive_ipv6(&remote_id, &self.ctx.network_key);
        self.ctx.peers.add(
            peer_ip,
            peer_ipv6,
            conn.clone(),
            remote_id,
            &self.network_name,
        );
        let token = self.token.clone();
        let disconnect_tx = self.disconnect_tx.clone();
        let network = self.network_name.clone();
        let pending_pongs = self.pending_pongs.clone();
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            send_member_sync(&conn).await;
            spawn_coordinator_control_reader(
                conn.clone(),
                remote_id,
                peer_ip,
                network.clone(),
                token.clone(),
                pending_pongs,
                ctx.global_gate.clone(),
            );
            forward::spawn_peer_reader(
                conn,
                remote_id,
                peer_ip,
                peer_ipv6,
                network,
                ctx.forward_ctx(disconnect_tx, token),
            );
        });
    }

    async fn handle_connection(&self, conn: Connection) {
        let remote_id = conn.remote_id();

        // Known member reconnecting: reuse its roster IP (which carries any
        // collision_index), not a fresh index-0 derivation.
        let member_ip = {
            let s = self.state.read().unwrap();
            s.members.get(&remote_id).map(|m| m.ip)
        };
        let peer_ip = member_ip.unwrap_or_else(|| self.ctx.identity.derive_ip(&remote_id));
        if member_ip.is_some() {
            self.handle_known_member_reconnect(conn, remote_id, peer_ip);
            return;
        }

        // Non-member: read the joiner's JoinRequest first, then gate by prior
        // approval, invite secret, and access mode. Known members are handled
        // above (send-first) and never reach here; fresh joiners always send a
        // JoinRequest first (see `join_mesh_shared`).
        let (send, mut recv) =
            match tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await {
                Ok(Ok(pair)) => pair,
                _ => return,
            };
        let msg = match tokio::time::timeout(Duration::from_secs(5), control::recv_msg(&mut recv))
            .await
        {
            Ok(Ok(m)) => m,
            _ => return,
        };
        let (invite_secret, hostname, device_cert) = match msg {
            ControlMsg::JoinRequest {
                invite_secret,
                hostname,
                device_cert,
            } => (invite_secret, hostname, device_cert),
            // Tolerate a bare MeshHello from older clients as a no-invite join.
            ControlMsg::MeshHello {
                hostname,
                device_cert,
                ..
            } => (None, hostname, device_cert),
            _ => return,
        };

        // Verify a device certificate if one is presented (full-tetron peers
        // with paired devices send one; tetron itself never does). A verified
        // cert is stored in the roster verbatim so full peers keep their
        // multi-device metadata; tetron does no revocation-floor checking or
        // reissue (pairing was removed by MINIMAL-004).
        if let Some(ref cert) = device_cert
            && (!cert.verify() || cert.device_key != remote_id)
        {
            tracing::warn!(peer = %remote_id.fmt_short(), "invalid device certificate");
            return;
        }

        // Unknown peer presenting an invite secret: verify and burn it.
        if let Some(secret) = invite_secret {
            self.redeem_invite_and_admit(
                conn,
                send,
                remote_id,
                peer_ip,
                hostname,
                device_cert,
                secret,
            )
            .await;
            return;
        }

        // Unknown peer, no invite: always denied. The only enrollment method
        // after LIVE-001 is an invite key — tetron itself can never create an
        // open network (MINIMAL-013), and a tetron node could only encounter
        // one by connecting to a full-tetron coordinator, which the ALPN
        // split makes impossible (D1 severed by RENAME-M02). `GroupMode::Open`
        // auto-admit accordingly has no reachable path left; removed
        // 2026-07-17 rather than left as unreachable dead code.
        tracing::warn!(peer = %remote_id.fmt_short(), "no invite presented; denied");
        self.deny(&conn, send, "a valid invite key is required to join".to_string())
            .await;
    }

    /// Admit (or reject) an unknown peer that presented an invite `secret`.
    ///
    /// Checks the signed `GroupBlob` invite table first (BLOB-001). If the secret
    /// matches a valid (not revoked, not expired) invite entry, the entry is
    /// removed from the blob, `dht_notify` is pulsed (so the background publisher
    /// republishes the updated blob, burning the invite), and the peer is admitted.
    /// Falls back to `GroupBlob.reusable_keys` if the secret isn't a single-use
    /// invite. tetron's own CLI has no way to mint a reusable key today (no
    /// `--reusable` flag on `tetron invite create` — see the trust-driven
    /// admission model: a coordinator vouches per-join, not via a standing
    /// credential), so this validation path is currently dormant, not D1
    /// wire compat — that scenario (a full-tetron coordinator minting one) is
    /// unreachable since RENAME-M02 severed the ALPN. Kept as the substrate
    /// for a possible future tetron-native reusable-key feature, which the
    /// validation logic here (product-agnostic: it just checks a presented
    /// secret against the blob) already supports without changes.
    ///
    /// **Replay race:** there is a narrow window between removing the invite from
    /// our local blob copy and the updated blob propagating to other coordinators
    /// (~30-60s DHT poll interval). A second coordinator that hasn't received the
    /// update could also accept the same secret. This is accepted for the initial
    /// implementation; a local reject cache will close the window in a follow-up.
    #[allow(clippy::too_many_arguments)]
    async fn redeem_invite_and_admit(
        &self,
        conn: Connection,
        send: iroh::endpoint::SendStream,
        remote_id: EndpointId,
        peer_ip: Ipv4Addr,
        hostname: Option<String>,
        device_cert: Option<control::DeviceCert>,
        secret: Vec<u8>,
    ) {
        // Phase 1: check blob invite table.
        let invite_valid = {
            let s = self.state.read().unwrap();
            crate::membership::validate_invite(&s.invites, &secret, now_secs()).is_some()
        };
        if invite_valid {
            // Burn the invite: remove from blob, refresh snapshot, notify publisher.
            let hash = blake3::hash(&secret).to_hex().to_string();
            {
                let mut s = self.state.write().unwrap();
                s.invites.remove(&hash);
                s.bump_generation_and_refresh();
            }
            // Pulse the publisher so the updated blob is signed + published.
            if let Some(ref notify) = self.dht_notify {
                notify.notify_one();
            }
            tracing::info!(
                peer = %remote_id.fmt_short(),
                "invite redeemed from blob"
            );
            self
                .admit_peer(
                    conn,
                    send,
                    remote_id,
                    peer_ip,
                    hostname,
                    device_cert,
                    false,
                )
                .await;
            return;
        }

        // Phase 2: fall back to GroupBlob reusable keys (currently dormant --
        // see the doc comment above).
        let reusable_id = {
            let s = self.state.read().unwrap();
            crate::membership::validate_reusable_key(&s.reusable_keys, &secret, now_secs())
                .map(|k| k.id.clone())
        };
        if let Some(key_id) = reusable_id {
            tracing::info!(
                peer = %remote_id.fmt_short(),
                key_id = %key_id,
                "reusable key redeemed"
            );
            // Reusable joins are non-authoritative: joiner-chosen name,
            // collision --> suffix.
            self.admit_peer(
                conn,
                send,
                remote_id,
                peer_ip,
                hostname,
                device_cert,
                false,
            )
            .await;
        } else {
            tracing::warn!(peer = %remote_id.fmt_short(), "invite rejected");
            self.deny(&conn, send, "invite rejected".to_string())
                .await;
        }
    }

    /// Reply on the joiner's stream that the join was refused, then wait for the
    /// joiner to close so the JoinDenied flushes before `conn` is dropped.
    async fn deny(&self, conn: &Connection, mut send: iroh::endpoint::SendStream, reason: String) {
        let _ = control::send_msg(&mut send, &ControlMsg::JoinDenied { reason }).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    }

    /// Admit a non-member peer into the network: assign hostname/IP, add to the
    /// member list, broadcast `MemberApproved`, reply `Welcome` on the joiner's
    /// stream, and start forwarding. Shared by the invite, open-mode, and
    /// live-approval admission paths.
    /// Returns `true` if the peer was admitted, `false` if the join was denied
    /// (hostname or IP collision). Callers that burned a credential to get here
    /// (an invite) restore it on `false` so the holder isn't locked out.
    #[allow(clippy::too_many_arguments)]
    async fn admit_peer(
        &self,
        conn: Connection,
        mut send: iroh::endpoint::SendStream,
        remote_id: EndpointId,
        _suggested_ip: Ipv4Addr,
        hostname: Option<String>,
        device_cert: Option<control::DeviceCert>,
        // The hostname is coordinator-authoritative (came from an invite binding).
        // Authoritative names are rejected on collision (no silent rename), so no
        // peer can claim another's name (and its Magic-DNS entry).
        authoritative: bool,
    ) -> bool {
        let (peer_ip, collision_index, final_hostname) =
            match self.validate_admission(remote_id, hostname, authoritative) {
                Ok(plan) => plan,
                Err(reason) => {
                    self.deny(&conn, send, reason).await;
                    return false;
                }
            };

        let user_id_opt = device_cert.as_ref().map(|c| c.user_identity);
        let snap_bytes = {
            let mut s = self.state.write().unwrap();
            let _ = s.members.add(Member {
                identity: remote_id,
                ip: peer_ip,
                is_coordinator: false,
                hostname: final_hostname.clone(),
                user_identity: user_id_opt,
                device_cert: device_cert.clone(),
                collision_index,
                last_seen: Some(crate::membership::now_secs()),
            });
            s.bump_generation_and_refresh();
            s.snapshot.as_ref().map(|snap| snap.msgpack_bytes.clone())
        };
        if let Some(bytes) = snap_bytes {
            let _ = self.ctx.blob_store.blobs().add_slice(&bytes).await;
        }

        broadcast_control_msg(
            &self.ctx.peers,
            &ControlMsg::MemberApproved {
                identity: remote_id,
                ip: peer_ip,
                hostname: final_hostname.clone(),
                device_cert: device_cert.clone(),
            },
        )
        .await;

        let (members, approved) = {
            let s = self.state.read().unwrap();
            (s.roster(), s.approved_snapshot())
        };

        tracing::info!(ip = %peer_ip, "new member admitted and joined");
        let _ = control::send_msg(
            &mut send,
            &ControlMsg::Welcome {
                members: members.clone(),
                approved,
            },
        )
        .await;

        if let Some(notify) = &self.dht_notify {
            notify.notify_one();
        }
        broadcast_member_sync(&self.ctx.peers, Some(peer_ip)).await;

        self.spawn_admitted_member_tasks(conn, remote_id, peer_ip);
        true
    }

    /// Decide a joiner's authoritative IP + hostname from the current roster, or
    /// return a denial reason. The IP is the lowest free collision index (not the
    /// peer-suggested address) so two coordinators admitting at index 0 produce a
    /// roster the reconverge tiebreak resolves deterministically. An invite-bound
    /// (`authoritative`) hostname already held by a different identity is rejected
    /// (no silent rename); a joiner-chosen name keeps collision resolution
    /// (`name` → `name-1` → …). An IP collision with a different identity is also
    /// rejected.
    fn validate_admission(
        &self,
        remote_id: EndpointId,
        hostname: Option<String>,
        authoritative: bool,
    ) -> std::result::Result<(Ipv4Addr, u32, Option<String>), String> {
        let (peer_ip, collision_index) = {
            let s = self.state.read().unwrap();
            crate::membership::assign_ip(&s.members, &remote_id, s.subnet)
        };
        let final_hostname = if let Some(desired) = hostname {
            let taken = {
                let s = self.state.read().unwrap();
                s.members
                    .all()
                    .iter()
                    .filter(|m| m.identity != remote_id)
                    .filter_map(|m| m.hostname.clone())
                    .collect::<Vec<String>>()
            };
            let taken_refs: Vec<&str> = taken.iter().map(|s| s.as_str()).collect();
            match crate::hostname::admission_hostname(&desired, &taken_refs, authoritative) {
                Ok(name) => Some(name),
                Err(conflict) => {
                    return Err(format!(
                        "hostname '{conflict}' is already in use on this network"
                    ));
                }
            }
        } else {
            None
        };
        let collision = {
            let s = self.state.read().unwrap();
            if let Some(existing) = s.members.get_by_ip(peer_ip) {
                existing.identity != remote_id
            } else if let Some(existing) = s.approved.get_by_ip(peer_ip) {
                existing.identity != remote_id
            } else {
                false
            }
        };
        if collision {
            return Err(format!("IP collision: {peer_ip} already assigned"));
        }
        // IPV6-002: defense against a *deliberately grinded* IPv6 collision
        // (~2^36 attempts is realistically feasible), not the accidental case
        // (astronomically unlikely at 72 bits, IPV6-001). No stored `ipv6`
        // field exists to look up (never transmitted/signed — always
        // re-derived locally), so recompute it for every existing roster
        // entry and compare.
        let candidate_ipv6 = derive_ipv6(&remote_id, &self.ctx.network_key);
        let ipv6_collision = {
            let s = self.state.read().unwrap();
            s.members.all().iter().any(|m| {
                m.identity != remote_id && derive_ipv6(&m.identity, &self.ctx.network_key) == candidate_ipv6
            }) || s.approved.all().iter().any(|a| {
                a.identity != remote_id && derive_ipv6(&a.identity, &self.ctx.network_key) == candidate_ipv6
            })
        };
        if ipv6_collision {
            return Err(format!("IPv6 collision: {candidate_ipv6} already assigned"));
        }
        Ok((peer_ip, collision_index, final_hostname))
    }

    /// Register an admitted member in the peer table and start its control
    /// reader (answers `Ping`/`Pong`) plus its inbound data-plane reader.
    fn spawn_admitted_member_tasks(
        &self,
        conn: Connection,
        remote_id: EndpointId,
        peer_ip: Ipv4Addr,
    ) {
        let peer_ipv6 = derive_ipv6(&remote_id, &self.ctx.network_key);
        crate::spawn_path_logger(conn.clone(), remote_id.fmt_short().to_string());
        self.ctx.peers.add(
            peer_ip,
            peer_ipv6,
            conn.clone(),
            remote_id,
            &self.network_name,
        );
        spawn_coordinator_control_reader(
            conn.clone(),
            remote_id,
            peer_ip,
            self.network_name.clone(),
            self.token.clone(),
            self.pending_pongs.clone(),
            self.ctx.global_gate.clone(),
        );
        forward::spawn_peer_reader(
            conn,
            remote_id,
            peer_ip,
            peer_ipv6,
            self.network_name.clone(),
            self.ctx
                .forward_ctx(self.disconnect_tx.clone(), self.token.clone()),
        );
    }
}

pub(crate) struct MemberAcceptState {
    pub(crate) ctx: MeshCtx,
    pub(crate) network_name: String,
    pub(crate) state: SharedNetworkState,
    pub(crate) disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    pub(crate) token: CancellationToken,
}

impl MemberAcceptState {
    /// Register a freshly handshaked peer in the peer table and start its
    /// inbound data-plane reader. Shared by the approved-join and known-member
    /// branches of `handle_connection`.
    fn register_peer(&self, conn: Connection, peer_identity: EndpointId, ip: Ipv4Addr) {
        let peer_ipv6 = derive_ipv6(&peer_identity, &self.ctx.network_key);
        self.ctx.peers.add(
            ip,
            peer_ipv6,
            conn.clone(),
            peer_identity,
            &self.network_name,
        );
        forward::spawn_peer_reader(
            conn,
            peer_identity,
            ip,
            peer_ipv6,
            self.network_name.clone(),
            self.ctx
                .forward_ctx(self.disconnect_tx.clone(), self.token.clone()),
        );
    }

    async fn handle_connection(&self, conn: Connection) {
        let Ok((_send, mut recv)) = conn.accept_bi().await else {
            return;
        };
        let transport_id = conn.remote_id();
        let Ok(ControlMsg::MeshHello {
            identity: peer_identity,
            ip,
            hostname,
            device_cert,
            ..
        }) = control::recv_msg(&mut recv).await
        else {
            return;
        };
        // Verify identity: either transport key matches, or a valid device cert is present
        let effective_user_id = if peer_identity == transport_id {
            peer_identity
        } else if let Some(ref cert) = device_cert {
            if !cert.verify()
                || cert.device_key != transport_id
                || cert.user_identity != peer_identity
            {
                tracing::warn!(peer = %transport_id.fmt_short(), "invalid device certificate");
                return;
            }
            cert.user_identity
        } else {
            return;
        };
        let _ = effective_user_id;
        let (is_member, is_approved) = {
            let s = self.state.read().unwrap();
            (
                s.members.is_member(&peer_identity),
                s.approved.is_approved(&peer_identity),
            )
        };
        // Resolve hostname collisions
        let final_hostname = if let Some(desired) = hostname {
            let taken = self.state.read().unwrap().taken_hostnames(peer_identity);
            let taken_refs: Vec<&str> = taken.iter().map(|s| s.as_str()).collect();
            Some(crate::hostname::resolve_collision(&desired, &taken_refs))
        } else {
            None
        };
        if is_approved {
            self.admit_approved_member(conn, peer_identity, ip, final_hostname, device_cert)
                .await;
        } else if is_member {
            if final_hostname.is_some() {
                let mut s = self.state.write().unwrap();
                if let Some(m) = s.members.get_mut(&peer_identity) {
                    m.hostname = final_hostname;
                }
            }
            self.register_peer(conn, peer_identity, ip);
        }
    }

    /// Promote a previously-approved peer to a full member on its `MeshHello`:
    /// seat it with the authoritative IP recorded at approval (not the
    /// peer-supplied one), republish the blob, send `Welcome`, start its reader,
    /// and trigger a `MemberSync` so the rest of the mesh learns the new roster.
    async fn admit_approved_member(
        &self,
        conn: Connection,
        peer_identity: EndpointId,
        ip: Ipv4Addr,
        final_hostname: Option<String>,
        device_cert: Option<control::DeviceCert>,
    ) {
        let (snap_bytes, ip) = {
            let mut s = self.state.write().unwrap();
            let approved_entry = s.approved.remove(&peer_identity);
            let user_id_opt = device_cert.as_ref().map(|c| c.user_identity);
            // Trust the authoritative IP + collision index recorded when the
            // peer was approved, not the peer-supplied MeshHello.ip.
            let (member_ip, member_idx) = approved_entry
                .as_ref()
                .map(|e| (e.ip, e.collision_index))
                .unwrap_or((ip, 0));
            let _ = s.members.add(Member {
                identity: peer_identity,
                ip: member_ip,
                is_coordinator: false,
                hostname: final_hostname.clone(),
                user_identity: user_id_opt,
                device_cert: device_cert.clone(),
                collision_index: member_idx,
                last_seen: Some(crate::membership::now_secs()),
            });
            s.bump_generation_and_refresh();
            (
                s.snapshot.as_ref().map(|snap| snap.msgpack_bytes.clone()),
                member_ip,
            )
        };
        if let Some(bytes) = snap_bytes {
            let _ = self.ctx.blob_store.blobs().add_slice(&bytes).await;
        }
        let (members, approved_list) = {
            let s = self.state.read().unwrap();
            (s.roster(), s.approved_snapshot())
        };
        if let Ok((mut send, _)) = conn.open_bi().await {
            let _ = control::send_msg(
                &mut send,
                &ControlMsg::Welcome {
                    members,
                    approved: approved_list,
                },
            )
            .await;
        }
        self.register_peer(conn, peer_identity, ip);
        broadcast_member_sync(&self.ctx.peers, Some(ip)).await;
    }
}

pub(crate) enum AcceptHandler {
    Coordinator(Arc<CoordinatorAcceptState>),
    Member(Arc<MemberAcceptState>),
}

#[cfg(test)]
impl AcceptHandler {
    pub(crate) fn is_coordinator(&self) -> bool {
        matches!(self, AcceptHandler::Coordinator(_))
    }
}

pub(crate) struct MeshProtocol {
    handler: AcceptHandler,
}

impl std::fmt::Debug for MeshProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshProtocol").finish()
    }
}

impl ProtocolHandler for MeshProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        match &self.handler {
            AcceptHandler::Coordinator(state) => state.handle_connection(conn).await,
            AcceptHandler::Member(state) => state.handle_connection(conn).await,
        }
        Ok(())
    }
}

pub(crate) struct ProtocolRouter {
    blobs: BlobsProtocol,
    handlers: DashMap<Vec<u8>, Arc<MeshProtocol>>,
    /// In-flight `tetron ping` probes, keyed by nonce. The control reader fires the
    /// oneshot when the matching `Pong` arrives so the ping handler can measure
    /// round-trip time. Cloned into both control readers.
    pub(crate) pending_pongs: Arc<DashMap<u64, tokio::sync::oneshot::Sender<()>>>,
}

impl ProtocolRouter {
    pub(crate) fn new(blobs: BlobsProtocol) -> Self {
        Self {
            blobs,
            handlers: DashMap::new(),
            pending_pongs: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn register(&self, alpn: Vec<u8>, handler: AcceptHandler) {
        self.handlers
            .insert(alpn, Arc::new(MeshProtocol { handler }));
    }

    pub(crate) fn unregister(&self, alpn: &[u8]) {
        self.handlers.remove(alpn);
    }

    pub(crate) fn alpns(&self) -> Vec<Vec<u8>> {
        let mut alpns: Vec<Vec<u8>> = self.handlers.iter().map(|r| r.key().clone()).collect();
        alpns.push(iroh_blobs::protocol::ALPN.to_vec());
        alpns
    }

    pub(crate) fn spawn_accept_loop(
        self: &Arc<Self>,
        endpoint: Endpoint,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let router = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    incoming = endpoint.accept() => {
                        let Some(incoming) = incoming else { return };
                        let router = router.clone();
                        tokio::spawn(async move {
                            let conn = match incoming.await {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::debug!(error = ?e, "incoming handshake failed");
                                    return;
                                }
                            };
                            let alpn = conn.alpn().to_vec();
                            match alpn.as_slice() {
                                a if a == iroh_blobs::protocol::ALPN => {
                                    let _ = router.blobs.clone().accept(conn).await;
                                }
                                _ => {
                                    if let Some(handler) = router.handlers.get(&alpn).map(|r| r.clone()) {
                                        let _ = handler.accept(conn).await;
                                    } else {
                                        tracing::warn!(
                                            alpn = %String::from_utf8_lossy(&alpn),
                                            "no handler for ALPN"
                                        );
                                    }
                                }
                            }
                        });
                    }
                }
            }
        })
    }
}


