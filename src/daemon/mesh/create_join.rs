//! Network create + join handlers for `MeshManager`: `create_network*`, the join
//! handshake (`join_network*`, dial/fetch/restore-roster helpers). Split out of `daemon/mod.rs`.

use super::super::*;
use crate::config::TransportMode;

/// Upper bound on a single proactive full-mesh dial in `dial_all_members`
/// (DIAL-001). An offline peer's `connect` fails on its own (fast when it has
/// no fresh discovery record, but up to iroh's internal handshake timeout —
/// tens of seconds — when a stale record still points at it). Capped so a
/// restart/reconnect never blocks that long on a dead peer: the dial is
/// best-effort and the peer's own reconnect loop re-establishes the link once
/// it comes back online.
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Borrowed bundle of the per-join inputs threaded through the dial + finalize
/// phases of `join_network_inner`, so each phase takes one argument instead of a
/// dozen. The references point at locals that live for the whole join.
struct JoinContext<'a> {
    display_name: &'a str,
    my_hostname: &'a str,
    alpn: &'a [u8],
    my_ip: Ipv4Addr,
    net_pubkey: EndpointId,
    /// Invite secret to redeem at admission, if any. Cloned per dial attempt (a
    /// fresh join may try several coordinators).
    invite: Option<Vec<u8>>,
    /// Per-network transport preference (none = default, Some(Tor) = route over Tor).
    transport: Option<TransportMode>,
    /// This network's own subnet (MULTISEG-001/004), resolved from the
    /// fetched `GroupBlob` before dialing. Used both to derive `my_ip` and,
    /// after a successful join, to create this network's own TUN device.
    network_subnet: crate::membership::Subnet,
    /// This network's own `MeshCtx` (MULTISEG-002), built from a fresh
    /// `peers`/`tun_tx` pair before the `NetworkHandle` exists — threaded
    /// through the dial phase instead of `self.mesh_ctx()`.
    mesh_ctx: MeshCtx,
}

/// A live mesh connection produced by the dial phase: the per-network state cell
/// plus the cancellation token, disconnect channel, and background tasks that
/// `finalize_join` folds into the `NetworkHandle`.
struct EstablishedMesh {
    state: SharedNetworkState,
    cancel: CancellationToken,
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Tear down a failed dial attempt: cancel the token and abort every spawned
/// task. Used on each unreachable/denied coordinator before trying the next.
fn abort_join_tasks(cancel: &CancellationToken, tasks: Vec<tokio::task::JoinHandle<()>>) {
    cancel.cancel();
    for t in tasks {
        t.abort();
    }
}

impl MeshManager {
    /// Refresh the network's blob snapshot, store its bytes in the local blob
    /// store, and publish the network-key-signed pkarr record (blob hash + this
    /// endpoint as the seed peer). Shared by network creation and coordinator
    /// restore — both seal a freshly built `NetworkState` and announce it.
    ///
    /// **Goes through the same read-before-write guard as the periodic
    /// publishers** (found via live testing, 2026-07-17): a restore whose
    /// `NetworkState` came from a stale-config fallback (DHT/blob
    /// unreachable at restart) must not unconditionally overwrite whatever
    /// is actually live on the DHT — that could resurrect superseded, or
    /// even already-nuked, state. For a genuinely brand-new network there's
    /// nothing to compare against yet, so the guard passes harmlessly (one
    /// extra resolve attempt, same as any other coordinator's first-ever
    /// publish now goes through). If the guard defers, the group poller
    /// picks up the actually-current state on its next tick — the daemon's
    /// in-memory view is briefly the restored one, not the DHT's, until then.
    pub(crate) async fn seal_and_publish(
        &self,
        net_state: &mut NetworkState,
        net_secret_key: &SecretKey,
    ) {
        net_state.refresh_snapshot();
        if let Some(snap) = &net_state.snapshot
            && let Err(e) = self.blob_store.blobs().add_slice(&snap.msgpack_bytes).await
        {
            tracing::error!(error = %e, "seal_and_publish: add_slice failed");
        }
        if let Ok(pkarr_client) = dht::create_pkarr_client(&self.endpoint) {
            let blob_hash = net_state
                .snapshot
                .as_ref()
                .map(|s| s.hash)
                .expect("snapshot set");
            let net_pubkey = net_secret_key.public();
            if !dht_read_before_write(&pkarr_client, net_pubkey, net_state.generation, blob_hash)
                .await
            {
                tracing::info!(
                    "seal_and_publish: DHT already at current/newer state; skipping publish"
                );
                return;
            }
            if let Err(e) = dht::publish_network(
                &pkarr_client,
                net_secret_key,
                &blob_hash,
                net_state.generation,
                &[self.endpoint.id()],
            )
            .await
            {
                tracing::warn!(error = %e, "failed to publish network record");
            }
        }
    }

    /// Spawn the two background tasks every coordinator network needs: the pkarr
    /// record publisher and the peer-disconnect cleanup (which republishes the
    /// blob when a member drops). Returns the task handles plus the
    /// `disconnect_tx` the accept handlers feed. Shared by create + restore.
    /// **MULTISEG-002:** `ctx` is this network's own [`MeshCtx`] (its own
    /// `peers`/`tun_tx`, built by the caller before the `NetworkHandle` exists
    /// — see `register_coordinator_handler`'s doc comment for why this can't
    /// be looked up here instead).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_coordinator_background_tasks(
        &self,
        name: &str,
        net_secret_key: &SecretKey,
        state: &SharedNetworkState,
        dht_notify: &Arc<tokio::sync::Notify>,
        poller_notify: &Arc<tokio::sync::Notify>,
        cancel: &CancellationToken,
        ctx: &MeshCtx,
    ) -> (
        Vec<tokio::task::JoinHandle<()>>,
        mpsc::Sender<forward::DisconnectEvent>,
    ) {
        let mut tasks = Vec::new();

        // Network publisher (single pkarr record: blob hash + seed peers)
        if let Ok(pkarr_client) = dht::create_pkarr_client(&self.endpoint) {
            tasks.push(spawn_network_publisher(
                pkarr_client,
                net_secret_key.clone(),
                state.clone(),
                self.endpoint.id(),
                ctx.peers.clone(),
                name.to_string(),
                dht_notify.clone(),
                cancel.clone(),
            ));
        }

        // Group poller: discover blob updates published by co-coordinators.
        // Without this, the coordinator never learns about changes it did not
        // originate itself (CONVERGE-001 follow-up).
        let net_pubkey = net_secret_key.public();
        if let Ok(poller_client) = dht::create_pkarr_client(&self.endpoint) {
            tasks.push(spawn_group_poller(
                poller_client,
                net_pubkey,
                state.clone(),
                self.endpoint.clone(),
                ctx.clone(),
                name.to_string(),
                cancel.clone(),
                self.left_tx.clone(),
                poller_notify.clone(),
            ));
        }

        // Disconnect handler (coordinator removes dead peers, republishes blob)
        let (disconnect_tx, disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(64);
        tasks.push(spawn_peer_cleanup(
            disconnect_rx,
            ctx.peers.clone(),
            cancel.clone(),
            Some(CoordinatorCleanup {
                state: state.clone(),
                blob_store: self.blob_store.clone(),
                dht_notify: Some(dht_notify.clone()),
                network_name: name.to_string(),
            }),
        ));

        (tasks, disconnect_tx)
    }

    /// Part of the embedding API: create a new network and register this
    /// node as its coordinator.
    #[tracing::instrument(skip(self, hostname), fields(mode = ?mode))]
    #[allow(clippy::too_many_arguments)]
    pub async fn create_network(
        self: &Arc<Self>,
        mode: GroupMode,
        network_name: Option<String>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        subnet: Option<crate::membership::Subnet>,
        nuke_consensus: Option<u32>,
    ) -> IpcMessage {
        match self
            .create_network_inner(
                mode,
                network_name,
                hostname,
                transport,
                subnet,
                nuke_consensus,
                false,
                None,
            )
            .await
        {
            Ok(resp) => resp,
            Err(e) => IpcMessage::Error {
                message: format!("{e:#}"),
            },
        }
    }

    /// Create a network and register it as coordinator.
    ///
    /// `direct` marks an auto-minted 2-peer `tetron connect` network (persisted so
    /// `tetron status` can tag it). `pre_approve` adds a peer to the `ApprovedList`
    /// before the blob is signed/published, so the named peer can be welcomed
    /// without a separate `tetron accept` round-trip — used by `approve_connection`.
    /// Build the initial [`NetworkState`] for a freshly created network: the
    /// creator as sole coordinator, plus any `pre_approve` peer (a `tetron connect`
    /// requester) admitted up front so the published blob already carries the
    /// approval and the peer is welcomed on its join without a separate
    /// `tetron accept`.
    #[allow(clippy::too_many_arguments)]
    fn build_initial_roster(
        &self,
        name: &str,
        my_ip: Ipv4Addr,
        my_hostname: &str,
        mode: GroupMode,
        net_secret_key: &SecretKey,
        subnet: crate::membership::Subnet,
        nuke_consensus_threshold: u32,
        pre_approve: Option<(EndpointId, Option<String>)>,
    ) -> Result<NetworkState> {
        let mut member_list = MemberList::new();
        member_list
            .add(Member {
                identity: self.identity.local_identity(),
                ip: my_ip,
                is_coordinator: true,
                hostname: Some(my_hostname.to_string()),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .expect("self-add cannot collide");

        let mut approved = ApprovedList::new();
        if let Some((peer_id, peer_hostname)) = pre_approve {
            // Derive in the network's chosen subnet, which may differ from the
            // provider's cached (node) subnet on a fresh `create --subnet`.
            let peer_ip = crate::membership::derive_ip(&peer_id, subnet);
            approved
                .approve(
                    ApprovedEntry {
                        identity: peer_id,
                        ip: peer_ip,
                        hostname: peer_hostname,
                        user_identity: None,
                        device_cert: None,
                        collision_index: 0,
                    },
                    &member_list,
                )
                .map_err(|e| anyhow::anyhow!("failed to pre-approve peer: {e:?}"))?;
        }

        Ok(NetworkState {
            generation: 0,
            members: member_list,
            approved,
            snapshot: None,
            network_secret_key: Some(net_secret_key.clone()),
            network_public_key: net_secret_key.public(),
            network_name: Some(name.to_string()),
            mode,
            subnet,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
            nuke_proposals: BTreeMap::new(),
            nuke_consensus_threshold,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_network_inner(
        self: &Arc<Self>,
        mode: GroupMode,
        custom_name: Option<String>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        subnet: Option<crate::membership::Subnet>,
        nuke_consensus: Option<u32>,
        direct: bool,
        pre_approve: Option<(EndpointId, Option<String>)>,
    ) -> Result<IpcMessage> {
        // NUKE-CONSENSUS-THRESHOLD-001: a threshold of 0 or 1 would let a
        // single coordinator (among 2+) execute a nuke unilaterally once
        // consensus is required at all -- exactly the risk NUKE-CONSENSUS
        // exists to close (see the requirement's "Found" note: a compromised
        // or malicious single key holder should not be able to unilaterally
        // destroy the network). A solo coordinator can already always nuke
        // immediately regardless of this value (that path never consults
        // it), so this floor only ever bites once a second coordinator
        // exists -- exactly where it matters.
        let nuke_consensus_threshold = match nuke_consensus {
            Some(n) => {
                anyhow::ensure!(
                    n >= 2,
                    "--nuke-consensus must be at least 2 (a value of 0 or 1 would let a single \
                     coordinator nuke unilaterally once a second coordinator exists, defeating \
                     the point of requiring consensus at all)"
                );
                n
            }
            None => crate::membership::default_nuke_consensus_threshold(),
        };
        let name = match custom_name {
            Some(n) => {
                anyhow::ensure!(
                    crate::hostname::is_valid_hostname(&n),
                    "invalid network name '{n}': use 1-63 lowercase ASCII letters, digits, or hyphens (no leading/trailing hyphen)"
                );
                n
            }
            None => network_name::generate_name(),
        };

        // Generate per-network keypair
        let net_secret_key = SecretKey::generate();
        let net_public_key = net_secret_key.public();

        if self.networks.contains_key(&name) {
            return Ok(IpcMessage::Error {
                message: format!("network '{name}' already active"),
            });
        }

        // MULTISEG-004: each network now gets its own TUN, so there is no
        // longer a node-wide single TUN for `--subnet` to disagree with — the
        // former SUBNET-010 rejection (a `--subnet` that disagreed with an
        // already-persisted node-wide value) is gone; a brand-new network name
        // has nothing to conflict with (the `already active` check above
        // already covers reusing an existing name). `AppConfig.subnet`'s only
        // remaining job is seeding the *default* subnet for a create with no
        // explicit `--subnet` and nothing persisted yet; an explicit `--subnet`
        // still updates that default for the node's next unspecified create.
        let persisted = config::load().ok().and_then(|c| c.subnet);
        // Every network on this node must land on a genuinely distinct
        // subnet -- otherwise two networks created without an explicit
        // `--subnet` both silently get the node's one default (found live-
        // testing: this node's own address ended up identical, 10.77.0.200,
        // on two supposedly-independent networks). Explicit `--subnet` is
        // still honored exactly, but rejected outright if it collides with
        // a network this node already has -- an explicit request deserves a
        // clear error, not a silent substitution. An unspecified `--subnet`
        // instead auto-advances past any collision, silently as far as the
        // resolution logic is concerned, but the actual chosen value is
        // always reported back in the command's own success output so it's
        // never a surprise later.
        let existing_subnets: Vec<crate::membership::Subnet> = config::load()
            .map(|c| c.networks.iter().filter_map(|n| n.subnet).collect())
            .unwrap_or_default();
        let subnet = match subnet {
            Some(requested) => {
                anyhow::ensure!(
                    !existing_subnets
                        .iter()
                        .any(|&e| crate::membership::subnets_overlap(e, requested)),
                    "--subnet {}/{} overlaps a network this node already has -- pick a distinct \
                     range (see `tetron status`'s `subnet` line for what's already in use)",
                    requested.0,
                    requested.1,
                );
                if let Err(e) = config::set_node_subnet(requested) {
                    tracing::warn!(error = %e, "failed to persist node subnet");
                }
                requested
            }
            None => {
                let candidate = persisted.unwrap_or_else(crate::membership::default_subnet);
                crate::membership::next_available_subnet(candidate, existing_subnets.into_iter())
            }
        };
        // The creator's own IP must land in the chosen subnet. When it matches the
        // provider's (node) subnet the cached local_ip is correct; otherwise it is
        // re-derived at collision index 0 (matching the self-member the roster adds).
        let my_ip = if subnet == self.identity.subnet() {
            self.identity.local_ip()
        } else {
            crate::membership::derive_ip(&self.identity.local_identity(), subnet)
        };

        let my_hostname = match hostname {
            Some(h) => {
                let h = h.to_ascii_lowercase();
                anyhow::ensure!(
                    crate::hostname::is_valid_hostname(&h),
                    "invalid hostname '{h}': use 1-63 ASCII letters, digits, or hyphens (no leading/trailing hyphen)"
                );
                h
            }
            None => config::load()
                .ok()
                .and_then(|c| c.default_hostname)
                .unwrap_or_else(crate::hostname::generate_hostname),
        };

        let mut net_state = self.build_initial_roster(
            &name,
            my_ip,
            &my_hostname,
            mode,
            &net_secret_key,
            subnet,
            nuke_consensus_threshold,
            pre_approve,
        )?;

        self.seal_and_publish(&mut net_state, &net_secret_key).await;

        // Save to config
        let member_entries = to_member_entries(net_state.members.all());
        let approved_entries = to_approved_entries(net_state.approved.all());
        config::save_network(&config::NetworkConfig {
            name: name.clone(),
            group_mode: mode,
            my_ip: Some(my_ip),
            my_hostname: Some(my_hostname.clone()),
            members: member_entries,
            approved: approved_entries,
            network_secret_key: Some(net_secret_key.clone()),
            network_public_key: Some(net_public_key),
            transport,
            admins: vec![],
            direct,
            // SUBNET-DRIFT-001: always explicit, never omitted -- MULTISEG-001's
            // original "None when it matches the node default, to keep configs
            // byte-identical" scheme meant `None` could no longer be told apart
            // from "the node's default subnet has since changed" on a later
            // restore, which silently mis-resolved this exact value. Persisting
            // it explicitly removes the ambiguity at the source.
            subnet: Some(subnet),
            nuke_consensus_threshold,
        })?;

        let cancel = self.shutdown_token.child_token();
        let state = Arc::new(std::sync::RwLock::new(net_state));
        let dht_notify = Arc::new(tokio::sync::Notify::new());
        let poller_notify = Arc::new(tokio::sync::Notify::new());
        let (peers, tun_tx) = self.new_network_data_plane();
        let ctx = MeshCtx {
            identity: self.identity.clone(),
            network_key: net_public_key,
            peers: peers.clone(),
            tun_tx: tun_tx.clone(),
            stats: self.stats.clone(),
            blob_store: self.blob_store.clone(),
            pruned_peers: self.pruned_peers.clone(),
            global_gate: self.global_gate.clone(),
        };
        let (tasks, disconnect_tx) = self.spawn_coordinator_background_tasks(
            &name,
            &net_secret_key,
            &state,
            &dht_notify,
            &poller_notify,
            &cancel,
            &ctx,
        );

        // Insert the handle first so register_coordinator_handler can update the role.
        let handle = NetworkHandle {
            name: name.clone(),
            network_key: net_public_key,
            role: NetworkRole::Coordinator,
            my_ip,
            state: state.clone(),
            dht_notify: Some(dht_notify.clone()),
            poller_notify,
            cancel: cancel.clone(),
            tasks,
            disconnect_tx: disconnect_tx.clone(),
            peers,
            tun_name: std::sync::Mutex::new(String::from("pending")),
            tun_tx,
            tun_tasks: std::sync::Mutex::new(None),
            active: Arc::new(AtomicBool::new(false)),
        };
        self.networks.insert(name.clone(), handle);

        // Register protocol handler for this network
        self.register_coordinator_handler(
            &name,
            state.clone(),
            Some(dht_notify),
            net_public_key,
            disconnect_tx,
            cancel,
            ctx,
        );
        self.refresh_alpns().await;

        // MULTISEG-003: this network's own TUN device, created (and, if the
        // VPN is already active, brought up) now rather than at daemon boot.
        #[cfg(not(target_os = "android"))]
        self.create_and_attach_network_tun(&name, my_ip, subnet)
            .await;

        tracing::info!(name = %name, key = %net_public_key, ip = %my_ip, "network created");

        // Mint an initial invite so the creator can share it immediately.
        let initial_invite_key = {
            let secret: [u8; crate::invite::SECRET_LEN] = rand::random();
            // Add invite to blob state, refresh snapshot, then drop lock.
            let snapshot_data = {
                let mut s = state.write().unwrap();
                // CONFIG-AUDIT-002: honor a configured invite-default-expiry
                // for the initial invite too, matching `invite_create`'s own
                // default resolution instead of a separate hardcoded literal.
                let default_ttl = config::load()
                    .ok()
                    .and_then(|c| c.invite_default_expiry)
                    .unwrap_or(7 * 24 * 3600);
                let (key, _entry) =
                    crate::membership::InviteEntry::from_secret(&secret, now_secs(), default_ttl);
                s.invites.insert(key, _entry);
                s.bump_generation_and_refresh();
                s.snapshot
                    .as_ref()
                    .map(|sn| (sn.msgpack_bytes.clone(), sn.hash, s.generation))
            };
            // Publish the updated blob (lock-free).
            if let Some((snap_bytes, snap_hash, snap_generation)) = snapshot_data {
                let _ = self.blob_store.blobs().add_slice(&snap_bytes).await;
                if let Ok(pkarr_client) = crate::dht::create_pkarr_client(&self.endpoint) {
                    let _ = crate::dht::publish_network(
                        &pkarr_client,
                        &net_secret_key,
                        &snap_hash,
                        snap_generation,
                        &[self.endpoint.id()],
                    )
                    .await;
                }
            }
            crate::invite::encode_invite_code(&net_public_key, &secret)
        };

        Ok(IpcMessage::Created {
            network: name,
            network_key: net_public_key,
            my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity(), &net_public_key)),
            // MULTISEG-003: this network's TUN is created fresh, in its own
            // subnet, right above — SUBNET-014's warning existed only because
            // a subnet mismatch used to require a full daemon restart to take
            // effect on the one shared TUN. That scenario no longer exists.
            warning: None,
            initial_invite_key: Some(initial_invite_key),
            subnet: format!("{}/{}", subnet.0, subnet.1),
        })
    }

    /// Part of the embedding API: join an existing network by key
    /// (optionally with an invite secret).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(self, hostname), fields(net = alias.unwrap_or(network_key)))]
    pub async fn join_network(
        self: &Arc<Self>,
        network_key: &str,
        alias: Option<&str>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        invite: Option<Vec<u8>>,
    ) -> IpcMessage {
        match self
            .join_network_inner(
                network_key,
                alias,
                hostname.clone(),
                transport,
                invite.clone(),
                true,
            )
            .await
        {
            Ok(TryJoin::Joined(resp)) => resp,
            Ok(TryJoin::Pending) => {
                // The coordinator queued us for live approval — this is a
                // full-tetron or legacy peer that still runs live admission.
                // tetron (LIVE-001) does not support `tetron accept`; the
                // caller must obtain an invite key from a coordinator.
                IpcMessage::Error {
                    message: "this network uses live approval, which tetron does not support; "
                        .to_string(),
                }
            }
            Err(e) => IpcMessage::Error {
                message: format!("{e:#}"),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn join_network_inner(
        self: &Arc<Self>,
        network_key: &str,
        alias: Option<&str>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        invite: Option<Vec<u8>>,
        // True for a fresh join (we send a JoinRequest first); false when
        // restoring a network we're already a member of (legacy handshake where
        // the coordinator speaks first).
        initial: bool,
    ) -> Result<TryJoin> {
        let net_pubkey: EndpointId = network_key.parse().context("invalid network key")?;

        if let Some(a) = alias
            && self.networks.contains_key(a)
        {
            anyhow::bail!("already in network '{a}'");
        }

        let data = match self.resolve_and_fetch_blob(net_pubkey).await {
            Ok(data) => data,
            // Boot-time restore only (CONVERGE-006): degrade to the persisted
            // config roster rather than dropping the network for this
            // daemon's entire runtime on a transient DHT/network hiccup at
            // boot. A fresh `tetron join` (initial=true) still fails loudly —
            // there is no prior membership to fall back to.
            Err(e) if !initial => {
                let Some(name) = alias else { return Err(e) };
                let Some(fallback) = self.fallback_blob_from_config(name) else {
                    return Err(e);
                };
                tracing::warn!(
                    network = %name,
                    error = %e,
                    "could not resolve/fetch network blob on restore; falling back to persisted config"
                );
                fallback
            }
            Err(e) => return Err(e),
        };

        let alpn = transport::network_alpn(&net_pubkey);
        // MULTISEG-004: this network gets its own TUN in its own subnet, so
        // (unlike the removed SUBNET-BUG-001 guard) there is no shared TUN for
        // it to disagree with — `my_ip` is derived directly from the
        // network's own blob-carried subnet, matching `create_network_inner`'s
        // existing derive-if-different pattern.
        let network_subnet = crate::membership::resolve_subnet(data.subnet);
        let my_ip = if network_subnet == self.identity.subnet() {
            self.identity.local_ip()
        } else {
            crate::membership::derive_ip(&self.identity.local_identity(), network_subnet)
        };
        // SUBNET-DRIFT-001: on a reconnect/restore (not a fresh join -- the
        // roster won't contain us yet in that case, so this is a no-op),
        // this must agree with what the roster already says our address is.
        // Same defense-in-depth as `restore_coordinator_network`'s check,
        // for the member-side restore path.
        if let Err(message) = crate::membership::validate_subnet_matches_roster(
            network_subnet,
            &data.members,
            &self.identity.local_identity(),
        ) {
            anyhow::bail!(message);
        }
        // Use coordinator's network name from GroupBlob, or user alias, or truncated key as fallback
        let blob_name = data
            .name
            .clone()
            .unwrap_or_else(|| network_key[..network_key.len().min(8)].to_string());
        let display_name_owned = alias.map(|a| a.to_string()).unwrap_or(blob_name);
        let display_name = display_name_owned.as_str();

        if self.networks.contains_key(display_name) {
            anyhow::bail!("already in network '{display_name}'");
        }

        let my_hostname = match hostname {
            Some(h) => {
                let h = h.to_ascii_lowercase();
                anyhow::ensure!(
                    crate::hostname::is_valid_hostname(&h),
                    "invalid hostname '{h}': use 1-63 ASCII letters, digits, or hyphens (no leading/trailing hyphen)"
                );
                h
            }
            None => config::load()
                .ok()
                .and_then(|c| c.default_hostname)
                .unwrap_or_else(crate::hostname::generate_hostname),
        };

        let (peers, tun_tx) = self.new_network_data_plane();
        let mesh_ctx = MeshCtx {
            identity: self.identity.clone(),
            network_key: net_pubkey,
            peers,
            tun_tx,
            stats: self.stats.clone(),
            blob_store: self.blob_store.clone(),
            pruned_peers: self.pruned_peers.clone(),
            global_gate: self.global_gate.clone(),
        };
        let ctx = JoinContext {
            display_name,
            my_hostname: &my_hostname,
            alpn: &alpn,
            my_ip,
            net_pubkey,
            invite,
            transport,
            network_subnet,
            mesh_ctx,
        };

        // Establish the mesh link. A fresh join tries each coordinator in the
        // blob's dial order (minter first) until one welcomes us; a reconnect/
        // restore uses the legacy single-coordinator handshake where the
        // coordinator speaks first. Either may return `None` (closed network,
        // queued for `tetron accept`) — propagate that to the caller as `Pending`.
        let established = if initial {
            self.dial_fresh_join(&ctx, &data).await?
        } else {
            self.dial_reconnect(&ctx, &data).await?
        };
        let Some(mesh) = established else {
            return Ok(TryJoin::Pending);
        };

        self.finalize_join(ctx, mesh).await
    }

    /// Resolve a network's signed pkarr record, gate on mesh-protocol version,
    /// and fetch + verify its `GroupBlob` from a seed peer. The version check is
    /// a pre-dial courtesy: the versioned ALPN is the hard gate but fails
    /// opaquely, so comparing the network-key-signed record up front yields a
    /// precise, actionable error instead.
    async fn resolve_and_fetch_blob(
        &self,
        net_pubkey: EndpointId,
    ) -> Result<crate::membership::GroupBlob> {
        let pkarr_client = dht::create_pkarr_client(&self.endpoint)?;
        let record = dht::resolve_network_packet(&pkarr_client, net_pubkey)
            .await
            .context("failed to resolve network record")?;

        // Absent version (older record) ⇒ skip and let the ALPN gate decide.
        if let Some(net_ver) = dht::mesh_version_from_record(&record) {
            let mine = transport::MESH_PROTOCOL_VERSION;
            anyhow::ensure!(
                net_ver == mine,
                "incompatible mesh protocol: this network runs v{net_ver}, this build speaks v{mine} \
                 - upgrade the older node so both sides match"
            );
        }

        let (expected_hash, _generation, peer_ids) =
            dht::decode_network_record(&record).context("invalid network record")?;
        if peer_ids.is_empty() {
            anyhow::bail!("no peers found in network record");
        }
        let blob_hash = iroh_blobs::Hash::from_bytes(*expected_hash.as_bytes());

        for peer_id in &peer_ids {
            match self.try_fetch_group_blob(*peer_id, blob_hash).await {
                Ok(data) => return Ok(data),
                Err(e) => {
                    tracing::warn!(peer = %peer_id.fmt_short(), error = %e, "failed to fetch blob");
                }
            }
        }
        anyhow::bail!("could not fetch group blob from any peer")
    }

    /// Build a `GroupBlob`-shaped fallback from `name`'s persisted config
    /// roster (CONVERGE-006), for when `resolve_and_fetch_blob` fails on a
    /// boot-time restore. Mirrors `restore_member_roster`'s config-fallback
    /// branch (the coordinator-restore counterpart), reusing data
    /// `persist_join_config` already writes on every successful join/
    /// reconnect. `generation: 0` is purely informational here — a member
    /// never publishes, and the next successful reconverge replaces it (and
    /// the empty firewall/reusable-key/invite fields) with the real, current
    /// blob. Returns `None` if there is no config entry or it has no members
    /// (nothing to fall back to).
    fn fallback_blob_from_config(&self, name: &str) -> Option<crate::membership::GroupBlob> {
        let nc = config::load_network(name).ok().flatten()?;
        if nc.members.is_empty() {
            return None;
        }
        let members = nc
            .members
            .iter()
            .map(|entry| crate::membership::Member {
                identity: entry.identity,
                ip: entry.ip,
                is_coordinator: entry.is_coordinator,
                hostname: entry.hostname.clone(),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .collect();
        let approved = nc
            .approved
            .iter()
            .map(|entry| ApprovedEntry {
                identity: entry.identity,
                ip: entry.ip,
                hostname: entry.hostname.clone(),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
            })
            .collect();
        Some(crate::membership::GroupBlob {
            generation: 0,
            members,
            approved,
            name: Some(name.to_string()),
            // SUBNET-DRIFT-001: this network's own persisted subnet, not the
            // node's current global default. The previous version used
            // `config::node_subnet()` here, reasoning it was "safe per the
            // SUBNET-BUG-001 invariant: an already-joined member's node subnet
            // already matches its network's" -- true only in the pre-multi-
            // segment world where a node ran one shared TUN/subnet for
            // everything. Since MULTISEG-002..007 let each network keep its
            // own independent subnet, and `finalize_join`/`create_network_inner`
            // both mutate this same node-wide default as a side effect on
            // every join/create, that invariant no longer holds the moment a
            // node's second network uses a different subnet -- this fallback
            // was then silently substituting an unrelated network's subnet
            // for this one's. `nc.subnet` is `None` only for a legacy config
            // predating the fix that made this field always explicit; the
            // caller's `resolve_subnet` falls back to the compiled default in
            // that case, and `validate_subnet_matches_roster` catches a real
            // mismatch rather than silently accepting it.
            subnet: nc.subnet,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
            nuke_proposals: BTreeMap::new(),
            nuke_consensus_threshold: nc.nuke_consensus_threshold,
        })
    }

    /// Fresh-join dial: try each coordinator in `coordinator_dial_order` (minter
    /// first) until one welcomes us. `Ok(None)` means a coordinator queued the
    /// request (`JoinPending`) and we stop there; the caller retries with backoff
    /// until `tetron accept` admits us.
    async fn dial_fresh_join(
        self: &Arc<Self>,
        ctx: &JoinContext<'_>,
        data: &crate::membership::GroupBlob,
    ) -> Result<Option<EstablishedMesh>> {
        let my_id = self.identity.local_identity();
        // In the invite-in-blob model (BLOB-001) there is no pinned coordinator
        // in the invite code. Use our own id as the nominal minter so the dial
        // order includes all blob coordinators.
        let order = coordinator_dial_order(my_id, &data.members, my_id);
        if order.is_empty() {
            anyhow::bail!("no coordinator found in network record");
        }

        let mut last_err = anyhow::anyhow!("no coordinators tried");
        for coordinator_id in &order {
            let cancel = self.shutdown_token.child_token();
            let (disconnect_tx, disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(64);
            // Oneshot channels deliver the live_state/reconverge_notify to the
            // reconnect loop after join_mesh_shared creates them.
            let (live_state_tx, live_state_rx) = tokio::sync::oneshot::channel();
            let (reconverge_notify_tx, reconverge_notify_rx) = tokio::sync::oneshot::channel();
            let tasks = vec![self.spawn_join_reconnect(
                ctx,
                my_id,
                &disconnect_tx,
                disconnect_rx,
                &cancel,
                live_state_rx,
                reconverge_notify_rx,
            )];

            tracing::info!(coordinator = %coordinator_id.fmt_short(), "connecting to coordinator");
            let conn = match transport::connect_to_peer_with_alpn(
                &self.endpoint,
                *coordinator_id,
                ctx.alpn,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(coordinator = %coordinator_id.fmt_short(), error = %e, "coordinator unreachable, trying next");
                    abort_join_tasks(&cancel, tasks);
                    last_err = anyhow::anyhow!("coordinator offline: {e}");
                    continue;
                }
            };

            match self
                .run_join_handshake(
                    ctx,
                    data,
                    conn,
                    true,
                    &disconnect_tx,
                    &cancel,
                    ctx.invite.clone(),
                )
                .await
            {
                Ok(JoinResult::Joined(state, reconverge_notify)) => {
                    // Deliver the resources to the reconnect loop (which blocks
                    // on the receivers until now; forward readers — and thus
                    // disconnect events — are spawned inside join_mesh_shared,
                    // so no race is possible).
                    // Best-effort: if the reconnect task was already aborted
                    // (another coordinator succeeded first), the receivers are
                    // dropped and the sends silently fail.
                    let _ = live_state_tx.send(state.clone());
                    let _ = reconverge_notify_tx.send(reconverge_notify);
                    return Ok(Some(EstablishedMesh {
                        state,
                        cancel,
                        disconnect_tx,
                        tasks,
                    }));
                }
                Ok(JoinResult::Pending) => {
                    // This coordinator queued the request — don't try the next;
                    // let the caller retry with backoff until accepted.
                    abort_join_tasks(&cancel, tasks);
                    return Ok(None);
                }
                Err(e) => {
                    tracing::warn!(coordinator = %coordinator_id.fmt_short(), error = %e, "coordinator denied or unreachable, trying next");
                    abort_join_tasks(&cancel, tasks);
                    last_err = e;
                }
            }
        }

        anyhow::bail!(
            "no coordinator admitted the join (tried {}): {last_err:#}",
            order.len()
        )
    }

    /// Reconnect/restore dial: the coordinator speaks first, so pick the single
    /// coordinator from the blob and run the legacy handshake. `Ok(None)` when
    /// queued for live approval (caller retries on backoff).
    async fn dial_reconnect(
        self: &Arc<Self>,
        ctx: &JoinContext<'_>,
        data: &crate::membership::GroupBlob,
    ) -> Result<Option<EstablishedMesh>> {
        let coordinator_id = data
            .members
            .iter()
            .find(|m| m.is_coordinator)
            .map(|m| m.identity)
            .context("no coordinator found in network record")?;

        // The reconnect loop is spawned unconditionally and up front. A member
        // already holds the verified blob, so being *in* the network does not
        // depend on the coordinator answering right now: if it is offline at
        // restore we still register the network from the blob and let this loop
        // dial it back when it returns. Without this a member that reboots while
        // its coordinator is down silently drops the network from its running
        // state until a lucky restart.
        let cancel = self.shutdown_token.child_token();
        let (disconnect_tx, disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(64);
        let my_id = self.identity.local_identity();
        let (live_state_tx, live_state_rx) = tokio::sync::oneshot::channel();
        let (reconverge_notify_tx, reconverge_notify_rx) = tokio::sync::oneshot::channel();
        let tasks = vec![self.spawn_join_reconnect(
            ctx,
            my_id,
            &disconnect_tx,
            disconnect_rx,
            &cancel,
            live_state_rx,
            reconverge_notify_rx,
        )];

        // Fallback state built straight from the verified blob so registration
        // never blocks on (or dies with) the coordinator handshake.
        let state_from_blob = || {
            let mut ns = NetworkState {
                generation: data.generation,
                members: MemberList::from_members(data.members.clone()),
                approved: ApprovedList::from_entries(data.approved.clone()),
                snapshot: None,
                network_secret_key: None,
                network_public_key: ctx.net_pubkey,
                network_name: Some(ctx.display_name.to_string()),
                mode: GroupMode::Restricted,
                subnet: crate::membership::resolve_subnet(data.subnet),
                reusable_keys: data.reusable_keys.clone(),
                invites: data.invites.clone(),
                nuke_proposals: data.nuke_proposals.clone(),
                nuke_consensus_threshold: data.nuke_consensus_threshold,
            };
            ns.refresh_snapshot();
            Arc::new(std::sync::RwLock::new(ns))
        };

        tracing::info!(coordinator = %coordinator_id.fmt_short(), "connecting to coordinator");
        let mut seed_from_blob = false;
        let (state, state_notify) = match transport::connect_to_peer_with_alpn(
            &self.endpoint,
            coordinator_id,
            ctx.alpn,
        )
        .await
        {
            Ok(conn) => {
                let mut joined_state: Option<(SharedNetworkState, Arc<tokio::sync::Notify>)> = None;
                match self
                    .run_join_handshake(
                        ctx,
                        data,
                        conn,
                        false,
                        &disconnect_tx,
                        &cancel,
                        ctx.invite.clone(),
                    )
                    .await
                {
                    Ok(JoinResult::Joined(state, reconverge_notify)) => {
                        joined_state = Some((state, reconverge_notify));
                    }
                    Ok(JoinResult::Pending) => {
                        // Closed network: queued for live approval. Stop the just-
                        // spawned reconnect loop (nothing connected yet); caller
                        // retries on a backoff until `tetron accept` lets us in.
                        abort_join_tasks(&cancel, tasks);
                        return Ok(None);
                    }
                    Err(e) => {
                        // Dialed the coordinator but the handshake failed. We still
                        // hold the verified blob, so register from it and let the
                        // reconnect loop recover rather than dropping the network.
                        tracing::warn!(coordinator = %coordinator_id.fmt_short(), error = %e, "coordinator handshake failed on restore; registering from blob, reconnect loop will retry");
                        seed_from_blob = true;
                    }
                }
                joined_state.unwrap_or_else(|| {
                    let fb = state_from_blob();
                    (fb, Arc::new(tokio::sync::Notify::new()))
                })
            }
            Err(e) => {
                // Coordinator offline at restore: register from the blob so the
                // network stays live; the reconnect loop dials it back once it
                // returns.
                tracing::warn!(coordinator = %coordinator_id.fmt_short(), error = %e, "coordinator offline on restore; registering from blob, reconnect loop will retry");
                seed_from_blob = true;
                (state_from_blob(), Arc::new(tokio::sync::Notify::new()))
            }
        };
        // Deliver state to the reconnect loop (it blocks until now). Must happen
        // once, outside the match, because oneshot::Sender::send takes ownership.
        let _ = live_state_tx.send(state.clone());
        let _ = reconverge_notify_tx.send(state_notify);

        // The reconnect loop is edge-triggered on disconnect events, so a cold
        // registration (no live connection yet) needs a synthetic kick per member
        // to start the backoff-retry dial. Only fires when we registered from the
        // blob without a live handshake.
        if seed_from_blob {
            let me = self.identity.local_identity();
            for m in &data.members {
                if m.identity == me {
                    continue;
                }
                let _ = disconnect_tx
                    .send(forward::DisconnectEvent {
                        endpoint_id: m.identity,
                        ip: m.ip,
                        ipv6: derive_ipv6(&m.identity, &ctx.net_pubkey),
                        network: ctx.display_name.to_string(),
                        // Synthetic cold-restore kick-start: not a leave or a
                        // kick, just a trigger to force the reconnect dial. No
                        // live connection backs it, so it must always proceed.
                        reason: forward::CloseReason::Other,
                        conn_stable_id: None,
                    })
                    .await;
            }
        }

        Ok(Some(EstablishedMesh {
            state,
            cancel,
            disconnect_tx,
            tasks,
        }))
    }

    /// Spawn the per-network reconnect loop used by both dial paths.
    #[allow(clippy::too_many_arguments)]
    fn spawn_join_reconnect(
        &self,
        ctx: &JoinContext<'_>,
        my_id: EndpointId,
        disconnect_tx: &mpsc::Sender<forward::DisconnectEvent>,
        disconnect_rx: mpsc::Receiver<forward::DisconnectEvent>,
        cancel: &CancellationToken,
        // The reconnect loop blocks on these receivers until the caller delivers
        // live_state + reconverge_notify via the corresponding senders after
        // join_mesh_shared completes. No disconnect can arrive before then (forward
        // readers are spawned inside join_mesh_shared), so the block is safe.
        live_state_rx: tokio::sync::oneshot::Receiver<SharedNetworkState>,
        reconverge_notify_rx: tokio::sync::oneshot::Receiver<Arc<tokio::sync::Notify>>,
    ) -> tokio::task::JoinHandle<()> {
        spawn_reconnect_loop(
            disconnect_rx,
            self.endpoint.clone(),
            ctx.alpn.to_vec(),
            ctx.display_name.to_string(),
            my_id,
            ctx.my_ip,
            ctx.mesh_ctx.clone(),
            disconnect_tx.clone(),
            cancel.clone(),
            live_state_rx,
            reconverge_notify_rx,
            self.promote_tx.clone(),
            self.protocol_router.pending_pongs.clone(),
        )
    }

    /// Run the mesh handshake over an established connection (shared by both dial
    /// paths). `initial` distinguishes a fresh join (we speak first) from a
    /// reconnect/restore (coordinator speaks first).
    #[allow(clippy::too_many_arguments)]
    async fn run_join_handshake(
        &self,
        ctx: &JoinContext<'_>,
        data: &crate::membership::GroupBlob,
        conn: iroh::endpoint::Connection,
        initial: bool,
        disconnect_tx: &mpsc::Sender<forward::DisconnectEvent>,
        cancel: &CancellationToken,
        invite_secret: Option<Vec<u8>>,
    ) -> Result<JoinResult> {
        join_mesh_shared(
            conn,
            &self.endpoint,
            ctx.display_name,
            ctx.alpn,
            ctx.mesh_ctx.clone(),
            JoinParams {
                my_hostname: Some(ctx.my_hostname.to_string()),
                net_pubkey: ctx.net_pubkey,
                invite_secret,
                reusable_keys: data.reusable_keys.clone(),
                invites: data.invites.clone(),
                nuke_proposals: data.nuke_proposals.clone(),
                nuke_consensus_threshold: data.nuke_consensus_threshold,
                generation: data.generation,
                transport: ctx.transport.clone(),
                initial,
                my_ip: ctx.my_ip,
                network_subnet: ctx.network_subnet,
            },
            disconnect_tx.clone(),
            cancel.clone(),
            self.promote_tx.clone(),
            self.left_tx.clone(),
            self.protocol_router.pending_pongs.clone(),
        )
        .await
    }

    /// Register the accept handler, persist the network public key, seed the blob
    /// store, spawn the membership poller, and install the `NetworkHandle`. Runs
    /// once the dial phase produced a live mesh connection.
    async fn finalize_join(
        self: &Arc<Self>,
        ctx: JoinContext<'_>,
        mesh: EstablishedMesh,
    ) -> Result<TryJoin> {
        let EstablishedMesh {
            state,
            cancel,
            disconnect_tx,
            mut tasks,
        } = mesh;
        let JoinContext {
            display_name,
            alpn,
            my_ip,
            net_pubkey,
            transport,
            network_subnet,
            mesh_ctx,
            ..
        } = ctx;

        // A node that already holds the network secret key (e.g. a
        // co-coordinator joining after a config-only restore) should run as
        // Coordinator so it can admit future peers immediately — even though
        // it arrived here via join rather than restore.
        let held_key = state.read().unwrap().network_secret_key.clone();
        match role_for_key_holder(held_key.is_some()) {
            NetworkRole::Coordinator => {
                let net_public_key = state.read().unwrap().network_public_key;
                self.register_coordinator_handler(
                    display_name,
                    state.clone(),
                    None,
                    net_public_key,
                    disconnect_tx.clone(),
                    cancel.clone(),
                    mesh_ctx.clone(),
                );
            }
            // `Direct` is a display-only role (set in `status`), never produced by
            // `role_for_key_holder`; a non-key-holder runs as a plain member.
            NetworkRole::Member | NetworkRole::Direct => {
                self.protocol_router.register(
                    alpn.to_vec(),
                    AcceptHandler::Member(Arc::new(MemberAcceptState {
                        ctx: mesh_ctx.clone(),
                        network_name: display_name.to_string(),
                        state: state.clone(),
                        disconnect_tx: disconnect_tx.clone(),
                        token: cancel.clone(),
                    })),
                );
            }
        }

        // Set the network public key on the state
        {
            let mut s = state.write().unwrap();
            s.network_public_key = net_pubkey;
            s.refresh_snapshot();
        }
        let snap_bytes = state
            .read()
            .unwrap()
            .snapshot
            .as_ref()
            .map(|s| s.msgpack_bytes.clone());
        if let Some(bytes) = snap_bytes {
            let _ = self.blob_store.blobs().add_slice(&bytes).await;
        }

        // Save config with network public key and transport preference
        if let Ok(Some(mut net)) = config::load_network(display_name) {
            net.network_public_key = Some(net_pubkey);
            net.transport = transport;
            let _ = config::save_network(&net);
        }

        // Membership poller
        let poller_notify = Arc::new(tokio::sync::Notify::new());
        if let Ok(poller_client) = dht::create_pkarr_client(&self.endpoint) {
            tasks.push(spawn_group_poller(
                poller_client,
                net_pubkey,
                state.clone(),
                self.endpoint.clone(),
                mesh_ctx.clone(),
                display_name.to_string(),
                cancel.clone(),
                self.left_tx.clone(),
                poller_notify.clone(),
            ));
        }

        let handle = NetworkHandle {
            name: display_name.to_string(),
            network_key: net_pubkey,
            role: NetworkRole::Member,
            my_ip,
            state,
            dht_notify: None,
            poller_notify,
            cancel,
            tasks,
            disconnect_tx,
            peers: mesh_ctx.peers.clone(),
            tun_name: std::sync::Mutex::new(String::from("pending")),
            tun_tx: mesh_ctx.tun_tx.clone(),
            tun_tasks: std::sync::Mutex::new(None),
            active: Arc::new(AtomicBool::new(false)),
        };
        self.networks.insert(display_name.to_string(), handle);
        self.refresh_alpns().await;

        tracing::info!(network = %display_name, key = %net_pubkey, ip = %my_ip, "joined network");

        // MULTISEG-003: this network's own TUN device, created (and, if the
        // VPN is already active, brought up) now rather than at daemon boot.
        #[cfg(not(target_os = "android"))]
        self.create_and_attach_network_tun(display_name, my_ip, network_subnet)
            .await;

        Ok(TryJoin::Joined(IpcMessage::Joined {
            network: display_name.to_string(),
            my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity(), &net_pubkey)),
            // MULTISEG-003: this network's TUN is created fresh, in its own
            // subnet, right above — see the identical note in
            // `create_network_inner`'s `Created` response.
            warning: None,
        }))
    }

    /// Fetch the authoritative GroupBlob for a network we coordinate, used to
    /// restore the roster across a daemon restart. Resolves the pkarr record to
    /// get the blob hash, reads the bytes back from the local blob store (where
    /// we stored them before publishing — no network round-trip), and verifies +
    /// decodes. Falls back to fetching from a seed peer if the local store
    /// doesn't have them (e.g. blobs dir was wiped). Returns an error if the DHT
    /// is unreachable, so the caller can fall back to the (possibly stale)
    /// config roster rather than booting empty.
    pub(crate) async fn restore_roster_from_blob(
        &self,
        net_pubkey: EndpointId,
    ) -> Result<crate::membership::GroupBlob> {
        let pkarr_client = dht::create_pkarr_client(&self.endpoint)?;
        let (expected_hash, _generation, seed_peers) =
            dht::resolve_network(&pkarr_client, net_pubkey)
                .await
                .context("resolve pkarr record for roster restore")?;
        let blob_hash = iroh_blobs::Hash::from_bytes(*expected_hash.as_bytes());

        // Local blob store first: the coordinator stored these bytes before
        // publishing, so they're on disk.
        if let Ok(bytes) = self.blob_store.blobs().get_bytes(blob_hash).await
            && let Ok(data) = verify_group_blob(&bytes, &expected_hash)
        {
            return Ok(data);
        }

        // Fall back to fetching from a seed peer.
        for peer_id in &seed_peers {
            if *peer_id == self.endpoint.id() {
                continue;
            }
            let conn = match transport::connect_to_peer_with_alpn(
                &self.endpoint,
                *peer_id,
                iroh_blobs::protocol::ALPN,
            )
            .await
            {
                Ok(c) => c,
                Err(_) => continue,
            };
            if self
                .blob_store
                .remote()
                .fetch(conn, HashAndFormat::raw(blob_hash))
                .await
                .is_err()
            {
                continue;
            }
            if let Ok(bytes) = self.blob_store.blobs().get_bytes(blob_hash).await
                && let Ok(data) = verify_group_blob(&bytes, &expected_hash)
            {
                return Ok(data);
            }
        }
        anyhow::bail!("group blob not found locally or at any seed peer");
    }

    pub(crate) async fn try_fetch_group_blob(
        &self,
        peer_id: EndpointId,
        blob_hash: iroh_blobs::Hash,
    ) -> Result<crate::membership::GroupBlob> {
        let conn = transport::connect_to_peer_with_alpn(
            &self.endpoint,
            peer_id,
            iroh_blobs::protocol::ALPN,
        )
        .await?;
        self.blob_store
            .remote()
            .fetch(conn, HashAndFormat::raw(blob_hash))
            .await
            .map_err(|e| anyhow::anyhow!("blob fetch failed: {e}"))?;
        let bytes = self
            .blob_store
            .blobs()
            .get_bytes(blob_hash)
            .await
            .map_err(|e| anyhow::anyhow!("blob read failed: {e}"))?;
        crate::membership::decode_group_blob(&bytes)
    }

    #[allow(dead_code)]
    pub(crate) async fn try_dht_fallback_join(
        &self,
        network_name: &str,
        net_pubkey: EndpointId,
        alpn: &[u8],
    ) -> Result<IpcMessage> {
        tracing::info!(network = %network_name, "trying DHT fallback");

        let pkarr_client = dht::create_pkarr_client(&self.endpoint)?;
        let (expected_hash, _generation, _peer_ids) =
            dht::resolve_network(&pkarr_client, net_pubkey).await?;

        let my_identity = self.identity.local_identity();
        let blob_hash = iroh_blobs::Hash::from_bytes(*expected_hash.as_bytes());

        let app_config = config::load()?;
        let net_config = app_config
            .networks
            .iter()
            .find(|n| n.name == network_name)
            .context("network not in config")?;

        for member in &net_config.members {
            if member.identity == my_identity {
                continue;
            }

            let blobs_conn = match transport::connect_to_peer_with_alpn(
                &self.endpoint,
                member.identity,
                iroh_blobs::protocol::ALPN,
            )
            .await
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            if self
                .blob_store
                .remote()
                .fetch(blobs_conn, HashAndFormat::raw(blob_hash))
                .await
                .is_err()
            {
                continue;
            }

            let blob_bytes = match self.blob_store.blobs().get_bytes(blob_hash).await {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };

            let data = verify_group_blob(&blob_bytes, &expected_hash)?;
            tracing::info!(network = %network_name, members = data.members.len(), "group blob resolved via DHT fallback");

            // MULTISEG-002: this dead-code path (`#[allow(dead_code)]`, no
            // caller anywhere in the crate) is given the same fresh
            // per-network `peers`/`tun_tx` pair as the live paths purely so it
            // keeps compiling against the current `NetworkHandle`/`MeshCtx`
            // shape; it does not create a real TUN device (nothing exercises
            // it, so there is nothing to attach one for).
            let (peers, tun_tx) = self.new_network_data_plane();
            let mesh_ctx = MeshCtx {
                identity: self.identity.clone(),
                network_key: net_pubkey,
                peers: peers.clone(),
                tun_tx: tun_tx.clone(),
                stats: self.stats.clone(),
                blob_store: self.blob_store.clone(),
                pruned_peers: self.pruned_peers.clone(),
                global_gate: self.global_gate.clone(),
            };
            let my_ip = self.identity.local_ip();
            let my_hostname = net_config.my_hostname.clone();
            let cancel = self.shutdown_token.child_token();
            let (disconnect_tx, disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(64);
            // DHT fallback has no run_join_handshake, so deliver dummy state/notify
            // to the reconnect loop immediately (the reconnect loop only uses them
            // for the control listener, which is optional).
            let (live_state_tx, live_state_rx) = tokio::sync::oneshot::channel();
            let (reconverge_notify_tx, reconverge_notify_rx) = tokio::sync::oneshot::channel();
            let dummy_state = Arc::new(std::sync::RwLock::new(NetworkState {
                generation: 0,
                members: crate::membership::MemberList::new(),
                approved: crate::membership::ApprovedList::new(),
                snapshot: None,
                network_secret_key: None,
                network_public_key: net_pubkey,
                network_name: Some(network_name.to_string()),
                mode: crate::membership::GroupMode::Restricted,
                subnet: crate::membership::resolve_subnet(None),
                reusable_keys: Default::default(),
                invites: Default::default(),
                nuke_proposals: Default::default(),
                nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
            }));
            let _ = live_state_tx.send(dummy_state);
            let _ = reconverge_notify_tx.send(Arc::new(tokio::sync::Notify::new()));

            let tasks = vec![spawn_reconnect_loop(
                disconnect_rx,
                self.endpoint.clone(),
                alpn.to_vec(),
                network_name.to_string(),
                my_identity,
                my_ip,
                mesh_ctx.clone(),
                disconnect_tx.clone(),
                cancel.clone(),
                live_state_rx,
                reconverge_notify_rx,
                self.promote_tx.clone(),
                self.protocol_router.pending_pongs.clone(),
            )];

            self.dial_all_members(
                &data.members,
                alpn,
                network_name,
                net_pubkey,
                my_identity,
                my_ip,
                my_hostname.clone(),
                disconnect_tx.clone(),
                cancel.clone(),
                &mesh_ctx,
            )
            .await;

            // Persist as the node's default-seed subnet (MULTISEG-004 narrowed
            // this to seeding an unspecified future `create`, not rebuilding a
            // single shared TUN — this network's own TUN would use
            // `joined_subnet` directly if this path were ever live).
            let joined_subnet = crate::membership::resolve_subnet(data.subnet);
            if let Err(e) = config::set_node_subnet(joined_subnet) {
                tracing::warn!(error = %e, "failed to persist node subnet on join");
            }
            let mut ns = NetworkState {
                generation: data.generation,
                members: MemberList::from_members(data.members),
                approved: ApprovedList::from_entries(data.approved),
                snapshot: None,
                network_secret_key: None,
                network_public_key: net_pubkey,
                network_name: data.name.clone(),
                mode: GroupMode::Restricted,
                subnet: joined_subnet,
                reusable_keys: data.reusable_keys.clone(),
                invites: data.invites.clone(),
                nuke_proposals: data.nuke_proposals.clone(),
                nuke_consensus_threshold: data.nuke_consensus_threshold,
            };
            ns.refresh_snapshot();
            let live_state = Arc::new(std::sync::RwLock::new(ns));

            let handle = NetworkHandle {
                name: network_name.to_string(),
                network_key: net_pubkey,
                role: NetworkRole::Member,
                my_ip,
                state: live_state,
                dht_notify: None,
                // No group poller runs on this DHT-fallback join path (see
                // its call site above -- only `spawn_reconnect_loop` is
                // spawned here), so this Notify has no receiver yet; a
                // `tetron sync` against this network is a harmless no-op
                // until/unless that changes.
                poller_notify: Arc::new(tokio::sync::Notify::new()),
                cancel,
                tasks,
                disconnect_tx,
                peers,
                tun_name: std::sync::Mutex::new(String::from("pending")),
                tun_tx,
                tun_tasks: std::sync::Mutex::new(None),
                active: Arc::new(AtomicBool::new(false)),
            };
            self.networks.insert(network_name.to_string(), handle);
            self.refresh_alpns().await;

            return Ok(IpcMessage::Joined {
                network: network_name.to_string(),
                my_ip,
                my_ipv6: Some(derive_ipv6(&self.identity.local_identity(), &net_pubkey)),
                warning: None,
            });
        }

        anyhow::bail!("no peers reachable for DHT fallback")
    }

    /// Dial every known member of a network: open a QUIC connection on the
    /// network ALPN, send `MeshHello`, register the peer in the PeerTable, and
    /// spawn a peer reader for each. Shared by the join path and coordinator
    /// restore so a restarting coordinator/co-coordinator proactively
    /// reconnects to **all** known members (full mesh), not just the peers
    /// that happen to dial in. Failures per-peer are logged at debug and
    /// skipped (the reconnect loop + group poller are the backstop).
    #[allow(clippy::too_many_arguments)]
    /// Dial every member concurrently (DIAL-001). Each `connect_to_peer_with_alpn`
    /// awaits an iroh handshake (hundreds of ms, or the full internal handshake
    /// timeout for an offline peer), so a serial loop made restore/reconnect scale
    /// linearly with the roster and stall on the first unreachable peer. Driving
    /// the dials as a `FuturesUnordered`, each bounded by [`DIAL_TIMEOUT`], caps
    /// the total wait at the timeout regardless of roster size: total time is the
    /// slowest single dial, not their sum, and a dead peer can't hang this call —
    /// the per-peer reconnect loop is the real recovery path either way.
    /// **MULTISEG-002:** `ctx` is `network_name`'s own [`MeshCtx`] — dialed
    /// peers register into that network's own `peers` table and forward into
    /// its own `tun_tx`, not a daemon-wide one.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dial_all_members(
        &self,
        members: &[Member],
        alpn: &[u8],
        network_name: &str,
        net_pubkey: EndpointId,
        my_identity: EndpointId,
        my_ip: Ipv4Addr,
        my_hostname: Option<String>,
        disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
        cancel: CancellationToken,
        ctx: &MeshCtx,
    ) {
        use futures::StreamExt;
        // Announce the current name (a pending rename or the confirmed one),
        // read fresh from config, rather than a value captured before a rename.
        let my_hostname = outgoing_hostname(network_name).or(my_hostname);
        let mut dials = futures::stream::FuturesUnordered::new();
        for m in members {
            if m.identity == my_identity {
                continue;
            }
            let my_hostname = my_hostname.clone();
            let disconnect_tx = disconnect_tx.clone();
            let cancel = cancel.clone();
            let peers = ctx.peers.clone();
            let tun_tx = ctx.tun_tx.clone();
            let stats = ctx.stats.clone();
            dials.push(async move {
                // Bound the dial and honor cancellation: an unreachable peer
                // would otherwise sit in iroh's internal handshake timeout,
                // keeping this call alive (and deaf to leave/down/shutdown)
                // far longer than the dial is worth.
                let conn = tokio::select! {
                    _ = cancel.cancelled() => return,
                    r = tokio::time::timeout(
                        DIAL_TIMEOUT,
                        transport::connect_to_peer_with_alpn(&self.endpoint, m.identity, alpn),
                    ) => r,
                };
                match conn {
                    Ok(Ok(peer_conn)) => {
                        if let Ok((mut s, _)) = peer_conn.open_bi().await {
                            let _ = control::send_msg(
                                &mut s,
                                &ControlMsg::MeshHello {
                                    identity: my_identity,
                                    ip: my_ip,
                                    hostname: my_hostname,
                                    device_cert: None,
                                },
                            )
                            .await;
                        }
                        crate::spawn_path_logger(
                            peer_conn.clone(),
                            m.identity.fmt_short().to_string(),
                        );
                        peers.add(
                            m.ip,
                            derive_ipv6(&m.identity, &net_pubkey),
                            peer_conn.clone(),
                            m.identity,
                            network_name,
                        );
                        forward::spawn_peer_reader(
                            peer_conn,
                            m.identity,
                            m.ip,
                            derive_ipv6(&m.identity, &net_pubkey),
                            network_name.to_string(),
                            forward::ForwardCtx {
                                tun_tx,
                                disconnect_tx,
                                token: cancel,
                                stats,
                            },
                        );
                        tracing::info!(
                            network = %network_name,
                            peer = %m.identity.fmt_short(),
                            "dialed known member on restore/join (full mesh)"
                        );
                    }
                    Ok(Err(e)) => {
                        tracing::debug!(
                            network = %network_name,
                            peer = %m.identity.fmt_short(),
                            error = %e,
                            "could not dial member yet; reconnect loop will retry"
                        );
                    }
                    Err(_elapsed) => {
                        tracing::debug!(
                            network = %network_name,
                            peer = %m.identity.fmt_short(),
                            timeout_secs = DIAL_TIMEOUT.as_secs(),
                            "dial timed out; reconnect loop will retry"
                        );
                    }
                }
            });
        }
        while dials.next().await.is_some() {}
    }
}
