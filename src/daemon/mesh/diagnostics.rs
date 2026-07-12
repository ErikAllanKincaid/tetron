//! Read-only diagnostics for `MeshManager`: `status` and connection-info
//! helpers. Split out of `daemon/mod.rs`.

use super::super::*;

impl MeshManager {
    /// Part of the embedding API (used by `ray-mobile` and future embedders):
    /// snapshot the daemon's status (identity, networks, peers).
    pub fn status(&self) -> IpcMessage {
        let hostname_snapshot = self.dns.hostname_table.try_read().ok();
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
            .map(|h| self.network_status(&h, my_id, hostname_snapshot.as_deref(), &direct_names))
            .collect();
        // Persisted pending-join markers, minus any network that has since
        // become active (admitted while we were retrying in the background).
        let pending_networks: Vec<String> = config::load()
            .map(|c| {
                c.pending_joins
                    .into_iter()
                    .filter(|p| !self.networks.contains_key(&p.network_key))
                    .map(|p| p.name.unwrap_or(p.network_key))
                    .collect()
            })
            .unwrap_or_default();

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

    /// Build one network's `NetworkStatus` for `torpedo status`. The peer list comes
    /// from the *roster* (every known member, not just live connections) so
    /// offline peers still show (Tailscale-style) with `connection: None`.
    fn network_status(
        &self,
        h: &NetworkHandle,
        my_id: EndpointId,
        hostname_snapshot: Option<&HashMap<String, HashMap<String, dns::HostnameEntry>>>,
        direct_names: &HashSet<String>,
    ) -> NetworkStatus {
        // Direct-connection networks are tagged `[direct]` regardless of role.
        let role = if direct_names.contains(&h.name) {
            NetworkRole::Direct
        } else {
            h.role.clone()
        };
        // Node-local aliases (display-only) come straight from config; status is
        // not a hot path, so a per-network read is fine.
        let net_cfg = config::load_network(&h.name).ok().flatten();
        let aliases = net_cfg
            .as_ref()
            .map(|n| n.aliases.clone())
            .unwrap_or_default();
        let ephemeral_ttl_secs = net_cfg.as_ref().and_then(|n| n.ephemeral_ttl_secs);
        // Resolve a mesh IPv4 back to its `.ray` hostname via the DNS snapshot.
        let lookup_hostname = |ip| {
            hostname_snapshot.and_then(|table| {
                table.get(&h.name).and_then(|hosts| {
                    hosts
                        .iter()
                        .find(|(_, v)| v.0 == ip)
                        .map(|(k, _)| k.clone())
                })
            })
        };

        let (members, member_count, pending_suggestions, pending_requests) = {
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
                        pending_suggestions: 0,
                        pending_requests: 0,
                        aliases,
                        ephemeral_ttl_secs,
                    };
                }
            };
            let count = s.members.all().len();
            (
                s.roster(),
                count,
                s.pending_suggestions.len(),
                s.pending.len(),
            )
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
                let hostname = m.hostname.clone().or_else(|| lookup_hostname(m.ip));
                let connection = connected.get(&m.identity).map(Self::gather_conn_info);
                PeerStatus {
                    endpoint_id: m.identity,
                    ip: m.ip,
                    ipv6: Some(derive_ipv6(&m.identity)),
                    hostname,
                    connection,
                }
            })
            .collect();
        NetworkStatus {
            name: h.name.clone(),
            role,
            my_ip: h.my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity())),
            my_hostname: lookup_hostname(h.my_ip),
            network_key: Some(h.network_key.to_string()),
            member_count,
            peers,
            pending_suggestions,
            pending_requests,
            aliases,
            ephemeral_ttl_secs,
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
