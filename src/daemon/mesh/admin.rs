//! Admin (co-coordinator) handlers for `MeshManager`: `admin_add` / `admin_list`.
//! Split out of `daemon/mod.rs`.

use super::super::*;

/// GRANT-PUBLISH-001: rounds a grant may take to land its coordinator flag on a
/// DHT record that keeps moving ahead of this node.
const GRANT_PUBLISH_ATTEMPTS: usize = 3;

impl MeshManager {
    /// Coordinator-only: grant the per-network secret key to a member over an
    /// authenticated mesh stream, making it a co-coordinator (can publish the
    /// signed blob / admit joiners). The key is shared (shared-key model), so this is
    /// a transfer of publish capability, not an attributable delegation. The
    /// grant is recorded locally for `tetron admin list`.
    pub(crate) async fn admin_add(&self, network: &str, peer_str: &str) -> IpcMessage {
        let network = match self.resolve_network_name_or_key(network) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        let identity = match self.resolve_peer_name(network, peer_str).await {
            Ok(id) => id,
            Err(message) => return IpcMessage::Error { message },
        };
        match self.grant_admin_key(network, identity).await {
            Ok(()) => IpcMessage::Ok {
                message: format!("granted network key to {}", identity.fmt_short()),
            },
            Err(message) => IpcMessage::Error { message },
        }
    }

    /// Grant the network key to `identity` directly — no name/hostname
    /// resolution, unlike `admin_add`. The mechanism shared between
    /// `admin_add` and `leave_network`'s auto-promotion-before-stranding
    /// (`STRANDED-COORDINATOR-WARN`): send the key over the target's
    /// *existing* mesh connection (opening a fresh one would land the
    /// `AdminGrant` on the member's new-connection handler, which expects a
    /// `MeshHello` first and silently drops anything else), publish the
    /// grantee as a coordinator in the signed group blob, and record the
    /// grant locally for `tetron admin list`. Requires the caller to
    /// already hold the network's secret key.
    pub(crate) async fn grant_admin_key(
        &self,
        network: &str,
        identity: EndpointId,
    ) -> Result<(), String> {
        let (net_pubkey, net_secret_key) = match self.networks.get(network) {
            Some(h) => {
                let key = {
                    let s = h.state.read().unwrap();
                    s.network_secret_key.clone()
                };
                let Some(key) = key else {
                    return Err(
                        "only a coordinator (network key holder) can grant admin".to_string()
                    );
                };
                (h.network_key, key)
            }
            None => {
                return Err(format!("network '{network}' not active"));
            }
        };

        let conn = self
            .networks
            .get(network)
            .map(|h| h.peers.peers_for_network_with_conn(network))
            .unwrap_or_default()
            .into_iter()
            .find(|(id, _, _)| *id == identity)
            .map(|(_, _, c)| c)
            .ok_or_else(|| {
                format!("could not find an active connection to {identity} on '{network}'")
            })?;

        let grant = ControlMsg::AdminGrant {
            network_pubkey: net_pubkey,
            secret_key: net_secret_key.to_bytes(),
        };
        match conn.open_bi().await {
            Ok((mut send, _)) => match control::send_msg(&mut send, &grant).await {
                Ok(()) => {
                    // The grant connection is dropped when this handler returns;
                    // wait for the grantee to read it so it flushes first.
                    let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
                }
                Err(e) => {
                    return Err(format!("failed to send admin grant: {e}"));
                }
            },
            Err(e) => {
                return Err(format!("failed to open stream to {identity}: {e}"));
            }
        }

        // Record the grant locally (coordinator's record; not verifiable). Done
        // before publishing: the key is already delivered and can not be
        // rolled back, so the record must hold even if the publish below fails.
        if let Ok(Some(mut net)) = config::load_network(network)
            && !net.admins.contains(&identity)
        {
            net.admins.push(identity);
            let _ = config::save_network(&net);
        }

        // Publish the grantee as a coordinator in the signed group blob so
        // joiners can discover co-coordinators to dial, and tell members now.
        self.publish_grant(network, identity, net_pubkey).await
    }

    /// GRANT-PUBLISH-001: flag `grantee` as a coordinator and publish it without
    /// losing it to a newer DHT record. Each round re-asserts the flag on the
    /// current roster, bumps the generation, and checks the DHT first
    /// ([`grant_publish_action`]): publish if ahead, accept if identical, and
    /// otherwise reconverge from the signed record and go round again. On
    /// success connected members are nudged with `MemberSync` so they learn the
    /// flag now instead of at their next poll (as admit and kick already do).
    async fn publish_grant(
        &self,
        network: &str,
        grantee: EndpointId,
        net_pubkey: EndpointId,
    ) -> Result<(), String> {
        for attempt in 1..=GRANT_PUBLISH_ATTEMPTS {
            let (generation, hash) = {
                let Some(handle) = self.networks.get(network) else {
                    return Err(format!("network '{network}' not active"));
                };
                let mut s = handle.state.write().unwrap();
                crate::membership::mark_coordinator(&mut s.members, &grantee);
                s.bump_generation_and_refresh();
                let Some(snapshot) = s.snapshot.as_ref() else {
                    return Err(format!("network '{network}' has no snapshot to publish"));
                };
                (s.generation, snapshot.hash)
            };
            let dht_state = match dht::create_pkarr_client(&self.endpoint) {
                Ok(client) => dht::resolve_network(&client, net_pubkey)
                    .await
                    .ok()
                    .map(|(dht_hash, dht_generation, _)| (dht_generation, dht_hash)),
                Err(_) => None,
            };
            match grant_publish_action(generation, hash, dht_state) {
                GrantPublish::Publish => {
                    self.store_and_publish_group(network).await;
                    self.nudge_members(network).await;
                    return Ok(());
                }
                GrantPublish::AlreadyPublished => {
                    self.nudge_members(network).await;
                    return Ok(());
                }
                GrantPublish::Reconverge => {
                    tracing::info!(
                        network = %network,
                        attempt,
                        local_generation = generation,
                        dht_generation = ?dht_state.map(|(g, _)| g),
                        "DHT record is ahead of this grant; reconverging before re-asserting the coordinator flag"
                    );
                    self.reconverge_now(network, net_pubkey).await;
                }
            }
        }
        Err(format!(
            "the key was delivered to {}, but its coordinator flag could not be published: \
             the network record kept moving ahead of this node. Run the same `tetron admin \
             add` again; it is safe to repeat (it resends the key and re-asserts the flag)",
            grantee.fmt_short()
        ))
    }

    /// GRANT-PUBLISH-001: adopt the signed record's roster now (the same step
    /// the poller and the `MemberSync` worker run).
    async fn reconverge_now(&self, network: &str, net_pubkey: EndpointId) {
        let (Some(ctx), Some(state)) = (
            self.mesh_ctx_for(network),
            self.networks.get(network).map(|h| h.state.clone()),
        ) else {
            return;
        };
        super::reconverge::reconverge_and_apply(
            &self.endpoint,
            &ctx,
            net_pubkey,
            network,
            &state,
            self.endpoint.id(),
            &[],
            std::net::Ipv4Addr::UNSPECIFIED,
            &self.left_tx,
        )
        .await;
    }

    /// GRANT-PUBLISH-001: tell every connected peer to reconverge now.
    async fn nudge_members(&self, network: &str) {
        if let Some(ctx) = self.mesh_ctx_for(network) {
            broadcast_member_sync(&ctx.peers, None).await;
        }
    }

    /// List this network's key-holders: the local node (if it holds the key) plus
    /// every identity it has granted the key to (`tetron admin add`).
    pub(crate) fn admin_list(&self, network: &str) -> IpcMessage {
        let network = match self.resolve_network_name_or_key(network) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        let self_id = self.endpoint.id();
        let mut admins = Vec::new();
        let self_holds_key = match self.networks.get(network) {
            Some(h) => h.state.read().unwrap().network_secret_key.is_some(),
            None => false,
        };
        if self_holds_key {
            admins.push(ipc::AdminInfo {
                short_id: self_id.fmt_short().to_string(),
                self_node: true,
            });
        }
        if let Ok(cfg) = config::load()
            && let Some(net) = cfg.networks.iter().find(|n| n.name == network)
        {
            for id in &net.admins {
                admins.push(ipc::AdminInfo {
                    short_id: id.fmt_short().to_string(),
                    self_node: false,
                });
            }
        }
        if !self_holds_key && admins.is_empty() {
            return IpcMessage::Error {
                message: format!("network '{network}' not found or not a coordinator"),
            };
        }
        IpcMessage::AdminListResponse { admins }
    }

    // -----------------------------------------------------------------------
    // Direct connections (tetron connect)
    // -----------------------------------------------------------------------
}
