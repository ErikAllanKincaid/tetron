//! Network runtime handlers for `MeshManager`: coordinator restore, nuke,
//! connect-all, activate/deactivate (data plane), teardown, leave. Split out of `daemon/mod.rs`.

use super::super::*;
use std::sync::RwLock;

/// The membership a coordinator restores at startup, sourced from the signed
/// `GroupBlob` (authoritative) or the stale config roster as a fallback.
struct RestoredRoster {
    members: MemberList,
    approved: ApprovedList,
    reusable_keys: BTreeMap<String, crate::membership::ReusableKey>,
    invites: BTreeMap<String, crate::membership::InviteEntry>,
    /// Pending nuke proposals (NUKE-CONSENSUS), restored so a coordinator
    /// restart doesn't silently drop an in-flight proposal.
    nuke_proposals: BTreeMap<String, u64>,
    /// The restored blob's own generation (CONVERGE-005), or 0 if restored from
    /// the config fallback (no signed blob was reachable).
    generation: u64,
}

impl MeshManager {
    /// Rebuild a network's roster for a coordinator restart. Prefers the
    /// published, network-key-signed `GroupBlob` (members + approved +
    /// reusable keys); if the DHT is unreachable, falls back to the
    /// last-persisted config roster (which may be stale). Always ensures this
    /// node is present as a coordinator member.
    async fn restore_member_roster(
        &self,
        name: &str,
        net_public_key: EndpointId,
        net_config: Option<&config::NetworkConfig>,
        my_ip: Ipv4Addr,
        persisted_hostname: &Option<String>,
    ) -> RestoredRoster {
        let mut member_list = MemberList::new();
        let mut approved_list = ApprovedList::new();
        // Reusable join keys and invites are authoritative in the signed blob.
        let mut reusable_keys = BTreeMap::new();
        let mut invites = BTreeMap::new();
        let mut nuke_proposals = BTreeMap::new();
        let mut generation = 0u64;
        match self.restore_roster_from_blob(net_public_key).await {
            Ok(data) => {
                reusable_keys = data.reusable_keys.clone();
                invites = data.invites.clone();
                nuke_proposals = data.nuke_proposals.clone();
                generation = data.generation;
                for m in &data.members {
                    let _ = member_list.add(m.clone());
                }
                for a in &data.approved {
                    let _ = approved_list.approve(a.clone(), &member_list);
                }
                tracing::info!(
                    network = %name,
                    members = member_list.all().len(),
                    "restored roster from published group blob"
                );
            }
            Err(e) => {
                tracing::warn!(
                    network = %name,
                    error = %e,
                    "could not restore roster from DHT blob; falling back to config (may be stale)"
                );
                if let Some(nc) = net_config {
                    for entry in &nc.members {
                        let _ = member_list.add(Member {
                            identity: entry.identity,
                            ip: entry.ip,
                            is_coordinator: entry.is_coordinator,
                            hostname: entry.hostname.clone(),
                            user_identity: None,
                            device_cert: None,
                            collision_index: 0,
                            last_seen: None,
                        });
                    }
                    for entry in &nc.approved {
                        let ae = ApprovedEntry {
                            identity: entry.identity,
                            ip: entry.ip,
                            hostname: entry.hostname.clone(),
                            user_identity: None,
                            device_cert: None,
                            collision_index: 0,
                        };
                        let _ = approved_list.approve(ae, &member_list);
                    }
                }
            }
        }
        if !member_list.is_member(&self.identity.local_identity()) {
            member_list
                .add(Member {
                    identity: self.identity.local_identity(),
                    ip: my_ip,
                    is_coordinator: true,
                    hostname: persisted_hostname.clone(),
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                    last_seen: None,
                })
                .expect("self-add cannot collide");
        }
        RestoredRoster {
            members: member_list,
            approved: approved_list,
            reusable_keys,
            invites,
            nuke_proposals,
            generation,
        }
    }

    /// Restores a coordinator network from saved config (uses the existing name).
    pub(crate) async fn restore_coordinator_network(
        &self,
        name: &str,
        mode: GroupMode,
    ) -> Result<IpcMessage> {
        {
            if self.networks.contains_key(name) {
                return Ok(IpcMessage::Error {
                    message: format!("network '{name}' already active"),
                });
            }
        }

        let my_ip = self.identity.local_ip();

        // Load persisted network secret key from config
        let app_config = config::load()?;
        let net_config = app_config.networks.iter().find(|n| n.name == name);
        let net_secret_key = net_config
            .and_then(|nc| nc.network_secret_key.clone())
            .context("no network secret key in config — cannot restore as coordinator")?;
        let net_public_key = net_secret_key.public();
        let persisted_hostname = net_config.and_then(|nc| nc.my_hostname.clone());

        // Restore membership from the authoritative published GroupBlob. The blob
        // (members + approved) is signed by the per-network key and published
        // to DHT, so it is the source of truth and survives a daemon restart. The
        // local blob store still holds the bytes we published before going down, so
        // we read them back by the hash in the pkarr record (falling back to a seed
        // peer, then to the stale config roster only if the DHT is unreachable).
        // Restoring from the blob is also what prevents a clobber: the rebuilt
        // snapshot hashes identical to the published record, so the periodic
        // re-publish becomes a no-op instead of overwriting the roster with a
        // coordinator-only stub.
        let RestoredRoster {
            members: member_list,
            approved: approved_list,
            reusable_keys,
            invites,
            nuke_proposals,
            generation,
        } = self
            .restore_member_roster(name, net_public_key, net_config, my_ip, &persisted_hostname)
            .await;

        let mut net_state = NetworkState {
            generation,
            members: member_list,
            approved: approved_list,
            snapshot: None,
            network_secret_key: Some(net_secret_key.clone()),
            network_public_key: net_public_key,
            network_name: Some(name.to_string()),
            mode,
            // SUBNET-010: single-TUN node — use the provider's operative subnet
            // (built from the persisted node cache at bootstrap).
            subnet: self.identity.subnet(),
            reusable_keys,
            invites,
            nuke_proposals,
        };

        self.seal_and_publish(&mut net_state, &net_secret_key).await;

        // Update config
        let member_entries = to_member_entries(net_state.members.all());
        let approved_entries = to_approved_entries(net_state.approved.all());
        config::save_network(&config::NetworkConfig {
            name: name.to_string(),
            group_mode: mode,
            my_ip: Some(my_ip),
            my_hostname: persisted_hostname.clone(),
            // Coordinators publish renames directly, so they never carry a
            // pending intent.
            members: member_entries,
            approved: approved_entries,
            network_secret_key: Some(net_secret_key.clone()),
            network_public_key: Some(net_public_key),
            transport: net_config.and_then(|nc| nc.transport.clone()),
            // Preserve the persisted admin roster across a restart; only the
            // roster (members/approved) is authoritative from the blob.
            admins: net_config.map(|nc| nc.admins.clone()).unwrap_or_default(),
            direct: net_config.map(|nc| nc.direct).unwrap_or(false),
        })?;

        let cancel = self.shutdown_token.child_token();
        let state = Arc::new(RwLock::new(net_state));
        let dht_notify = Arc::new(tokio::sync::Notify::new());
        let (tasks, disconnect_tx) = self.spawn_coordinator_background_tasks(
            name,
            &net_secret_key,
            &state,
            &dht_notify,
            &cancel,
        );

        self.register_coordinator_handler(
            name,
            state.clone(),
            Some(dht_notify.clone()),
            net_public_key,
            disconnect_tx.clone(),
            cancel.clone(),
        );

        // Insert the network before dialing its members (DIAL-001), not after:
        // `dial_all_members` used to run first, so a `tetron status` in the
        // ~150ms-or-more window before it finished (routinely hit right after
        // `sudo tetron restart`) reported no active networks at all, even
        // though the local restore (and config on disk) was already complete.
        // The accept handler is already registered above, so return traffic
        // is handled regardless of dial order.
        let handle = NetworkHandle {
            name: name.to_string(),
            network_key: net_public_key,
            role: NetworkRole::Coordinator,
            my_ip,
            state: state.clone(),
            dht_notify: Some(dht_notify),
            cancel: cancel.clone(),
            tasks,
            disconnect_tx: disconnect_tx.clone(),
        };
        self.networks.insert(name.to_string(), handle);
        self.refresh_alpns().await;

        tracing::info!(name = %name, key = %net_public_key, ip = %my_ip, "network restored (coordinator)");

        // Full mesh: proactively dial every known member so a restarting
        // coordinator/co-coordinator reconnects to peers that haven't (yet)
        // dialed in. Without this, a co-coordinator that comes back up only
        // learns about peers that connect *to it*; it never dials out, so two
        // co-coordinators restarting together can each show the other as
        // offline until one is manually disturbed. Now concurrent and
        // timeout-bounded (DIAL-001), so this never scales with roster size or
        // hangs on a single dead peer.
        let members_to_dial: Vec<Member> = state
            .read()
            .unwrap()
            .members
            .all()
            .into_iter()
            .cloned()
            .collect();
        let alpn = transport::network_alpn(&net_public_key);
        self.dial_all_members(
            &members_to_dial,
            &alpn,
            name,
            self.identity.local_identity(),
            my_ip,
            persisted_hostname.clone(),
            disconnect_tx,
            cancel,
        )
        .await;

        Ok(IpcMessage::Created {
            name: name.to_string(),
            network_key: net_public_key,
            my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity())),
            warning: crate::membership::subnet_change_warning(
                crate::config::node_subnet(),
                self.identity.subnet(),
            ),
            initial_invite_key: None,
        })
    }

    /// Destroy a network (NUKE-CONSENSUS). A solo coordinator (no one to
    /// second) nukes immediately, unchanged from the original behavior. With
    /// two or more coordinators, this adds the caller's own proposal to the
    /// signed blob instead of nuking outright; if that action itself brings
    /// the count of distinct, unexpired proposers to two or more, this same
    /// call executes the nuke immediately. `cancel` withdraws the caller's own
    /// proposal. `second` optionally names (by short id) the specific proposal
    /// being seconded, for an explicit error if it doesn't match an active one
    /// rather than silently proposing fresh.
    #[tracing::instrument(skip(self), fields(net = short_id))]
    pub(crate) async fn nuke_network(
        &self,
        short_id: &str,
        force: bool,
        cancel: bool,
        second: Option<&str>,
    ) -> IpcMessage {
        let name = match self.resolve_network_short_id(short_id) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let name = name.as_str();
        let my_id = self.endpoint.id();
        let (is_coordinator, has_other_members, coordinator_count) = {
            let handle = match self.networks.get(name) {
                Some(h) => h,
                None => {
                    return IpcMessage::Error {
                        message: format!("not in network '{name}'"),
                    };
                }
            };
            let state = handle.state.read().unwrap();
            let is_coord = state
                .members
                .get(&my_id)
                .map(|m| m.is_coordinator)
                .unwrap_or(false);
            let others = state.members.all().len() > 1;
            let roster = state.roster();
            (
                is_coord,
                others,
                crate::membership::coordinator_count(&roster),
            )
        };

        if !is_coordinator {
            return IpcMessage::Error {
                message: "only the coordinator can nuke a network".to_string(),
            };
        }

        if coordinator_count <= 1 {
            if cancel || second.is_some() {
                return IpcMessage::Error {
                    message: "no consensus needed with a single coordinator; nuke runs immediately, nothing to cancel or second".to_string(),
                };
            }
            if has_other_members && !force {
                return IpcMessage::Error {
                    message: "network has other members — use --force to destroy, or transfer ownership first".to_string(),
                };
            }
            let (net_secret_key, tombstone_generation) = {
                let handle = self.networks.get(name).unwrap();
                let state = handle.state.read().unwrap();
                (state.network_secret_key.clone(), state.generation + 1)
            };
            self.publish_nuke_tombstone(net_secret_key, tombstone_generation)
                .await;
            return self.leave_network(name).await;
        }

        // Two or more coordinators: consensus required.
        if cancel {
            let (dht_notify, snap_bytes) = {
                let handle = self.networks.get(name).unwrap();
                let mut state = handle.state.write().unwrap();
                if state.nuke_proposals.remove(&my_id.to_string()).is_none() {
                    return IpcMessage::Error {
                        message: "you have no active nuke proposal on this network".to_string(),
                    };
                }
                state.bump_generation_and_refresh();
                (
                    handle.dht_notify.clone(),
                    state.snapshot.as_ref().map(|s| s.msgpack_bytes.clone()),
                )
            };
            if let Some(bytes) = snap_bytes
                && let Err(e) = self.blob_store.blobs().add_slice(&bytes).await
            {
                tracing::error!(error = %e, "nuke --cancel: add_slice failed");
            }
            if let Some(notify) = dht_notify {
                notify.notify_one();
            }
            return IpcMessage::Ok {
                message: format!("nuke proposal for '{name}' cancelled"),
            };
        }

        if has_other_members && !force {
            return IpcMessage::Error {
                message: "network has other members — use --force to destroy, or transfer ownership first".to_string(),
            };
        }

        let now = now_secs();
        if let Some(short) = second {
            let handle = self.networks.get(name).unwrap();
            let state = handle.state.read().unwrap();
            if let Err(e) =
                crate::membership::resolve_nuke_proposer(&state.nuke_proposals, now, short)
            {
                return IpcMessage::Error {
                    message: format!("{e:#}"),
                };
            }
        }

        let (dht_notify, net_secret_key, generation, active_count, snap_bytes, short_id) = {
            let handle = self.networks.get(name).unwrap();
            let mut state = handle.state.write().unwrap();
            state.nuke_proposals.insert(my_id.to_string(), now);
            state.bump_generation_and_refresh();
            let active = crate::membership::active_nuke_proposers(&state.nuke_proposals, now).len();
            (
                handle.dht_notify.clone(),
                state.network_secret_key.clone(),
                state.generation,
                active,
                state.snapshot.as_ref().map(|s| s.msgpack_bytes.clone()),
                handle.network_key.fmt_short().to_string(),
            )
        };

        if active_count < 2 {
            // Not enough seconds yet: persist + publish the proposal itself,
            // same as any other blob mutation (invite create, admin grant, ...).
            if let Some(bytes) = snap_bytes
                && let Err(e) = self.blob_store.blobs().add_slice(&bytes).await
            {
                tracing::error!(error = %e, "nuke propose: add_slice failed");
            }
            if let Some(notify) = dht_notify {
                notify.notify_one();
            }
            return IpcMessage::Ok {
                message: format!(
                    "nuke proposed for '{name}' — {active_count}/2 coordinators required; have another coordinator run `tetron nuke {short_id}` to second"
                ),
            };
        }

        // This call itself reached consensus: execute immediately rather than
        // waiting for the proposal-blob publish + a reconverge cycle.
        self.publish_nuke_tombstone(net_secret_key, generation + 1)
            .await;
        self.leave_network(name).await
    }

    /// Publish an empty (tombstone) pkarr record for a network, poisoning the
    /// record so no one resolves it again. The tombstone must carry a strictly
    /// higher generation than whatever's live (CONVERGE-005) — otherwise a
    /// generation-aware reader would treat this erase as stale and ignore it.
    ///
    /// **Persists the empty blob's bytes to the local store before publishing
    /// the DHT pointer** (found via live testing 2026-07-17): without this,
    /// `resolve_network` still correctly signals the new (higher) generation
    /// to remaining members, but `fetch_verified_blob` can never actually
    /// fetch+verify content matching that hash from anywhere — the executing
    /// coordinator is the only node that ever held it, in memory, and it's
    /// gone the moment this function returns and the caller leaves. Remaining
    /// members' `member_removed` (CONVERGE-003) check never fires, so they
    /// never self-remove; they're left polling a generation bump they can
    /// never resolve. This bug predates NUKE-CONSENSUS (the original
    /// single-coordinator nuke had the same gap) but only surfaces when other
    /// members are actually still present to notice — which is exactly
    /// NUKE-CONSENSUS's normal case, unlike the original's typical
    /// already-abandoned-network use.
    ///
    /// No-op if `net_secret_key` is `None` (shouldn't happen — callers only
    /// reach this after an `is_coordinator` check — but publishing requires
    /// the key, so this stays defensive rather than panicking).
    async fn publish_nuke_tombstone(&self, net_secret_key: Option<SecretKey>, generation: u64) {
        let Some(key) = net_secret_key else {
            tracing::warn!("nuke: no network secret key available to publish tombstone");
            return;
        };
        let Ok(client) = dht::create_pkarr_client(&self.endpoint) else {
            tracing::warn!("nuke: failed to create pkarr client for tombstone publish");
            return;
        };
        let empty_bytes = canonical_group_bytes(
            generation,
            &MemberList::new(),
            &ApprovedList::new(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        if let Err(e) = self.blob_store.blobs().add_slice(&empty_bytes).await {
            tracing::error!(error = %e, "failed to persist empty tombstone blob");
        }
        let empty_hash = blake3::hash(&empty_bytes);
        if let Err(e) = dht::publish_network(&client, &key, &empty_hash, generation, &[]).await {
            tracing::warn!(error = %e, "failed to publish empty network record on nuke");
        } else {
            tracing::info!(generation, "published empty (tombstone) network record");
        }
    }

    /// Resolve a peer argument (bare hostname, or a short-id / endpoint-id
    /// prefix) to its endpoint id. Backs `tetron admin add`. Hostname
    /// resolution is deliberately available here: `admin add` is additive
    /// (grants trust to whoever the name currently resolves to), so a
    /// friendlier identifier is an acceptable convenience. Destructive
    /// commands (`tetron kick`, `tetron nuke --second`) intentionally do
    /// NOT use this — they resolve by short id / endpoint id only via
    /// [`Self::resolve_short_id_any_network`], since removing the wrong
    /// peer needs a cryptographic identity, not a spoofable one.
    pub(crate) async fn resolve_peer_name(&self, name: &str) -> Result<EndpointId, String> {
        for entry in self.networks.iter() {
            let state = entry.value().state.read().unwrap();
            if let Some(m) = state
                .members
                .all()
                .iter()
                .find(|m| m.hostname.as_deref() == Some(name))
            {
                return Ok(m.identity);
            }
        }
        self.resolve_short_id_any_network(name)
    }

    /// Remove a member from a closed network. Coordinator-only (any network-key
    /// holder). Prunes the target from the roster + approved list, republishes the
    /// signed blob, and broadcasts a `MemberSync` so every member reconverges and
    /// drops the target mesh-wide (`prune_departed_peers`); the coordinator also
    /// closes its own link to the target immediately. Refused on open networks
    /// (the target would auto-re-join) and against coordinators / self.
    pub(crate) async fn kick_member(&self, network_short_id: &str, peer: &str) -> IpcMessage {
        let network = match self.resolve_network_short_id(network_short_id) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        let (state, dht_notify, has_key, mode) = match self.networks.get(network) {
            Some(h) => {
                let (has_key, mode) = {
                    let s = h.state.read().unwrap();
                    (s.network_secret_key.is_some(), s.mode)
                };
                (h.state.clone(), h.dht_notify.clone(), has_key, mode)
            }
            None => {
                return IpcMessage::Error {
                    message: format!("network '{network}' not found"),
                };
            }
        };
        if !has_key {
            return IpcMessage::Error {
                message: "only a coordinator (network key holder) can kick a member".to_string(),
            };
        }
        if mode == GroupMode::Open {
            return IpcMessage::Error {
                message: format!(
                    "'{network}' is an open network — a kicked peer can re-join immediately. \
                     Kicking only takes effect on a closed network."
                ),
            };
        }

        // Resolve the argument to a roster member by endpoint id only (no
        // hostname or IP resolution — kick requires a cryptographic identity).
        let candidate = match self.resolve_short_id_any_network(peer) {
            Ok(id) => id,
            Err(message) => {
                return IpcMessage::Error { message };
            }
        };
        let (member_id, is_coord, display) = {
            let s = state.read().unwrap();
            match s
                .members
                .all()
                .into_iter()
                .find(|m| m.identity == candidate)
            {
                Some(m) => (
                    m.identity,
                    m.is_coordinator,
                    m.hostname
                        .clone()
                        .unwrap_or_else(|| m.identity.fmt_short().to_string()),
                ),
                None => {
                    return IpcMessage::Error {
                        message: format!("'{peer}' is not a member of '{network}'"),
                    };
                }
            }
        };
        if member_id == self.endpoint.id() {
            return IpcMessage::Error {
                message: "cannot kick yourself — use `tetron leave` or `tetron nuke`".to_string(),
            };
        }
        if is_coord {
            return IpcMessage::Error {
                message: format!(
                    "'{display}' is a coordinator (holds the network key); kicking can't remove \
                     its access. Revoke the key instead."
                ),
            };
        }

        // Prune the roster, then publish + broadcast + sever the link.
        let ctx = self.mesh_ctx();
        remove_member_roster_only(&state, member_id);
        finalize_removal(&ctx, network, &state, &dht_notify, &[member_id]).await;

        tracing::info!(peer = %member_id.fmt_short(), network = %network, "kicked member");
        IpcMessage::Ok {
            message: format!("kicked '{display}' from '{network}'"),
        }
    }

    /// Connect to every saved network (control plane). Run once at daemon
    /// startup so mesh connections follow the daemon lifecycle, not the data
    /// plane: `tetron down` keeps these connected so the node stays online to
    /// peers. Connections are dropped only on leave/nuke/shutdown.
    pub(crate) async fn connect_all_networks(self: &Arc<Self>) {
        let app_config = match config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load config during connect");
                return;
            }
        };
        let mut count = 0;
        for net in &app_config.networks {
            count += 1;
            if net.network_secret_key.is_some() {
                // We hold the secret key, restore as coordinator.
                let name = net.name.clone();
                let mode = net.group_mode;
                let daemon_c = Arc::clone(self);
                tokio::spawn(async move {
                    match daemon_c.restore_coordinator_network(&name, mode).await {
                        Ok(IpcMessage::Created { name, .. }) => {
                            tracing::info!(network = %name, "restored coordinator network");
                        }
                        Ok(IpcMessage::Error { message }) => {
                            tracing::warn!(network = %name, error = %message, "failed to restore network");
                        }
                        Err(e) => {
                            tracing::warn!(network = %name, error = %e, "failed to restore network");
                        }
                        _ => {}
                    }
                });
            } else {
                // We're a member, rejoin via DHT lookup.
                let name = net.name.clone();
                let persisted_hostname = net.my_hostname.clone();
                let net_pubkey = match &net.network_public_key {
                    Some(k) => k.to_string(),
                    None => {
                        tracing::warn!(network = %name, "no network public key in config, skipping restore");
                        continue;
                    }
                };
                let net_transport = net.transport.clone();
                let daemon_c = Arc::clone(self);
                tokio::spawn(async move {
                    match daemon_c
                        .join_network_inner(
                            &net_pubkey,
                            Some(&name),
                            persisted_hostname,
                            net_transport,
                            None,
                            false,
                        )
                        .await
                    {
                        Ok(TryJoin::Joined(IpcMessage::Joined { name, my_ip, .. })) => {
                            tracing::info!(network = %name, ip = %my_ip, "restored member network");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(network = %name, error = %e, "failed to restore network");
                        }
                    }
                });
            }
        }

        // LIVE-001 removed the pending-join queue; nothing to resume.

        tracing::info!(networks = count, "control plane connected");
    }

    /// Activate the VPN: bring the TUN interface up. Idempotent — a no-op if
    /// already active. Runs entirely inside the (root) daemon, so the IPC
    /// client needs no privileges. Part of the embedding API: bring the data
    /// plane up (mark active, configure routes). On Android the packet
    /// interface + routes are the `VpnService`'s job, so those desktop route
    /// calls are skipped.
    pub async fn activate(self: &Arc<Self>, hostname: Option<String>) -> IpcMessage {
        // Persist the personal default hostname first (before the already-active
        // short-circuit) so `tetron up --hostname X` records the new default even
        // when the VPN is already up. Used as the fallback for future
        // creates/joins; doesn't rename networks already joined.
        if let Some(h) = hostname {
            let h = h.to_ascii_lowercase();
            if !crate::hostname::is_valid_hostname(&h) {
                return IpcMessage::Error {
                    message: format!(
                        "invalid hostname '{h}': use 1-63 ASCII letters, digits, or hyphens (no leading/trailing hyphen)"
                    ),
                };
            }
            match config::load() {
                Ok(mut app_config) => {
                    app_config.default_hostname = Some(h);
                    if let Err(e) = config::save_settings(&app_config) {
                        tracing::warn!(error = %e, "failed to persist default hostname");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to load config to set default hostname")
                }
            }
        }

        if self.active.swap(true, Ordering::SeqCst) {
            return IpcMessage::Ok {
                message: "already up".into(),
            };
        }

        // Non-fatal problems hit while activating. The daemon stays up, but we
        // return these to the client so `tetron up` can tell the user something is
        // wrong instead of silently reporting success on a degraded VPN.
        let mut warnings: Vec<String> = Vec::new();

        // The TUN device/routes are managed by the OS on desktop. On Android the
        // packet interface is a `VpnService` fd whose routes are configured on the
        // Kotlin side, so these desktop route calls don't apply.
        #[cfg(not(target_os = "android"))]
        {
            let tun_name = self.tun_name.lock().unwrap().clone();
            if let Err(e) = tun::set_link_up(&tun_name) {
                tracing::warn!(error = %e, "failed to bring TUN interface up");
                warnings.push(format!("failed to bring TUN interface up: {e}"));
            }

            // Route the 200::/7 peer range into the TUN. Must happen after
            // link-up: on Linux the kernel won't install an IPv6 connected route
            // while the link is down, so without this peer traffic leaks out the
            // default route.
            if let Err(e) = tun::route_peer_range(&tun_name).await {
                tracing::warn!(error = %e, "failed to route 200::/7 into TUN");
                warnings.push(format!("failed to route IPv6 peer range into TUN: {e}"));
            }

            // Loop our own addresses back through lo0 so self-traffic (e.g.
            // pinging our own mesh IP) is answered locally instead of leaving via
            // the TUN, where the forwarding loop would drop it as "no peer for
            // dst". No-op on Linux (kernel installs the `local` route
            // automatically).
            let my_v4 = self.identity.local_ip();
            let my_v6 = derive_ipv6(&self.identity.local_identity());
            if let Err(e) = tun::route_self_loopback(my_v4, my_v6).await {
                tracing::warn!(error = %e, "failed to install loopback self-route");
                warnings.push(format!("failed to install loopback self-route: {e}"));
            }
        }

        tracing::info!("data plane activated");
        if warnings.is_empty() {
            IpcMessage::Ok {
                message: "VPN up".into(),
            }
        } else {
            let mut message = String::from("VPN up, but some things need attention:");
            for w in &warnings {
                message.push_str("\n  - ");
                message.push_str(w);
            }
            IpcMessage::Ok { message }
        }
    }

    /// Put the daemon on standby: take the data plane offline (bring the TUN
    /// link down, stop forwarding) while keeping the control plane connected.
    /// Network connections, control readers, and pollers stay live so the node
    /// remains online to peers and keeps receiving roster/blob updates.
    /// Connections are dropped only on leave/nuke/shutdown. Idempotent.
    pub(crate) async fn deactivate(&self) -> IpcMessage {
        if !self.active.swap(false, Ordering::SeqCst) {
            return IpcMessage::Ok {
                message: "already on standby".into(),
            };
        }

        let tun_name = self.tun_name.lock().unwrap().clone();

        #[cfg(not(target_os = "android"))]
        if let Err(e) = tun::set_link_down(&tun_name) {
            tracing::warn!(error = %e, "failed to bring TUN interface down");
        }

        tracing::info!("VPN on standby");
        IpcMessage::Ok {
            message: "VPN on standby (still connected to peers)".into(),
        }
    }

    /// Tear down a network's runtime state (connections, ALPN, background tasks)
    /// without touching its persisted config. Returns whether the network was
    /// active. Used by `leave_network` (which also forgets the config); standby
    /// (`deactivate`) no longer tears connections down.
    pub(crate) async fn teardown_network_runtime(&self, name: &str) -> bool {
        let Some(handle) = self.networks.remove(name).map(|(_, v)| v) else {
            return false;
        };
        handle.cancel.cancel();
        for task in handle.tasks {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }

        self.peers.remove_by_network(name);
        self.protocol_router
            .unregister(&transport::network_alpn(&handle.network_key));
        self.refresh_alpns().await;
        true
    }

    /// Leave `name` locally after a network's poller or reconverge worker
    /// (CONVERGE-003) determined the local node is no longer in the
    /// authoritative roster — kicked, or dropped by a stale publish race.
    /// Runs the same teardown as a manual `tetron leave`: without it the
    /// reconnect loop would keep redialing coordinators that now correctly
    /// deny us, forever, while `tetron status` kept reporting a healthy
    /// membership.
    #[tracing::instrument(skip(self), fields(net = name))]
    pub(crate) async fn handle_removed_from_network(&self, name: &str) {
        tracing::warn!(network = %name, "no longer a member of this network; leaving locally");
        self.leave_network(name).await;
    }

    /// Part of the embedding API.
    #[tracing::instrument(skip(self), fields(net = network))]
    pub async fn leave_network(&self, network: &str) -> IpcMessage {
        // Gracefully close our connections with the leave code BEFORE teardown
        // drops them, so each peer's reader sees an intentional close and the
        // coordinator prunes us from the roster (rather than waiting for an
        // idle timeout that only ever clears the green dot).
        for (_eid, _ip, conn) in self.peers.peers_for_network_with_conn(network) {
            conn.close(VarInt::from_u32(forward::LEAVE_CODE), b"leave");
        }

        let was_active = self.teardown_network_runtime(network).await;

        // Remove from config even if the network wasn't active
        let removed_from_config = config::delete_network(network).unwrap_or(false);

        if was_active || removed_from_config {
            tracing::info!(network = %network, "left network");
            IpcMessage::Ok {
                message: format!("left network '{}'", network),
            }
        } else {
            IpcMessage::Error {
                message: format!("network '{}' not found", network),
            }
        }
    }
}
