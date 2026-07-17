//! Read-only diagnostics for `MeshManager`: `status` and connection-info
//! helpers. Split out of `daemon/mod.rs`.

use super::super::*;

impl MeshManager {
    /// Part of the embedding API (used by `ray-mobile` and future embedders):
    /// snapshot the daemon's status (identity, networks, peers).
    pub fn status(&self) -> IpcMessage {
        let my_id = self.endpoint.id();
        // Direct-connection networks are flagged in config; collect their names
        // so each NetworkStatus can be tagged `[direct]` in the CLI.
        let direct_names: HashSet<String> = config::load()
            .map(|c| {
                c.networks
                    .iter()
                    .filter(|n| n.direct)
                    .map(|n| n.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let statuses: Vec<NetworkStatus> = self
            .networks
            .iter()
            .map(|h| self.network_status(&h, my_id, &direct_names))
            .collect();
        // LIVE-001 removed the pending-join queue; always empty.
        let pending_networks: Vec<String> = Vec::new();

        IpcMessage::StatusResponse {
            endpoint_id: self.endpoint.id(),
            active: self.active.load(Ordering::SeqCst),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            networks: statuses,
            packets_rx: self.stats.packets_rx.get(),
            packets_tx: self.stats.packets_tx.get(),
            bytes_rx: self.stats.bytes_rx.get(),
            bytes_tx: self.stats.bytes_tx.get(),
            pending_networks,
        }
    }

    /// Build one network's `NetworkStatus` for `tetron status`. The peer list comes
    /// from the *roster* (every known member, not just live connections) so
    /// offline peers still show (Tailscale-style) with `connection: None`.
    fn network_status(
        &self,
        h: &NetworkHandle,
        my_id: EndpointId,
        direct_names: &HashSet<String>,
    ) -> NetworkStatus {
        // Direct-connection networks are tagged `[direct]` regardless of role.
        let role = if direct_names.contains(&h.name) {
            NetworkRole::Direct
        } else {
            h.role.clone()
        };
        let (members, member_count, pending_requests, nuke_proposals) = {
            let s = match h.state.read() {
                Ok(s) => s,
                Err(_) => {
                    return NetworkStatus {
                        name: h.name.clone(),
                        role,
                        my_ip: h.my_ip,
                        my_ipv6: Some(derive_ipv6(&my_id)),
                        my_hostname: None,
                        network_key: Some(h.network_key.to_string()),
                        member_count: 0,
                        peers: vec![],
                        pending_requests: 0,
                        nuke_proposals: vec![],
                    };
                }
            };
            let count = s.members.all().len();
            let now = now_secs();
            let proposals = crate::membership::active_nuke_proposers(&s.nuke_proposals, now)
                .into_iter()
                .map(|id| ipc::NukeProposalInfo {
                    short_id: id.chars().take(10).collect(),
                    proposed_at: s.nuke_proposals[id],
                })
                .collect();
            (s.roster(), count, 0, proposals)
        };
        // Index live connections by endpoint id for a fast lookup.
        let connected: HashMap<EndpointId, Connection> = self
            .peers
            .peers_for_network_with_conn(&h.name)
            .into_iter()
            .map(|(eid, _, conn)| (eid, conn))
            .collect();
        let peers = members
            .iter()
            .filter(|m| m.identity != my_id)
            .map(|m| {
                let connection = connected.get(&m.identity).map(Self::gather_conn_info);
                PeerStatus {
                    endpoint_id: m.identity,
                    ip: m.ip,
                    ipv6: Some(derive_ipv6(&m.identity)),
                    hostname: m.hostname.clone(),
                    connection,
                }
            })
            .collect();
        // Our own hostname comes from the signed roster (Magic DNS removed).
        let my_hostname = members
            .iter()
            .find(|m| m.identity == my_id)
            .and_then(|m| m.hostname.clone());
        NetworkStatus {
            name: h.name.clone(),
            role,
            my_ip: h.my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity())),
            my_hostname,
            network_key: Some(h.network_key.to_string()),
            member_count,
            peers,
            pending_requests,
            nuke_proposals,
        }
    }

    pub(crate) fn gather_conn_info(conn: &iroh::endpoint::Connection) -> ipc::ConnectionInfo {
        let paths = conn.paths();
        // Classify every path, then pick which one to report. iroh only marks a
        // path `is_selected()` once its path-selector has promoted a winner;
        // during establishment, holepunch, or migration no path is selected even
        // though the connection is live and carrying traffic. Reporting only the
        // selected path then renders a working connection as `?`. `choose_path`
        // falls back to the best available (Direct > Relay > Tor) so a live
        // connection always reports a concrete path.
        let classes: Vec<(ipc::ConnType, bool)> = paths
            .iter()
            .map(|p| {
                let addr = p.remote_addr();
                let ct = if addr.is_relay() {
                    ipc::ConnType::Relay
                } else if addr.is_custom() {
                    ipc::ConnType::Tor
                } else {
                    ipc::ConnType::Direct
                };
                (ct, p.is_selected())
            })
            .collect();

        let (conn_type, remote_addr, rtt_ms) = match choose_path_index(&classes)
            .and_then(|idx| paths.iter().nth(idx).map(|p| (idx, p)))
        {
            Some((idx, path)) => {
                let rtt = path.rtt().as_secs_f64() * 1000.0;
                (
                    classes[idx].0.clone(),
                    Some(path.remote_addr().to_string()),
                    Some(rtt),
                )
            }
            None => (ipc::ConnType::Unknown, None, None),
        };

        let stats = conn.stats();
        ipc::ConnectionInfo {
            conn_type,
            remote_addr,
            rtt_ms,
            bytes_tx: stats.udp_tx.bytes,
            bytes_rx: stats.udp_rx.bytes,
            datagrams_tx: stats.udp_tx.datagrams,
            datagrams_rx: stats.udp_rx.datagrams,
            lost_packets: stats.lost_packets,
        }
    }
}
