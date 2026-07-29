//! Read-only diagnostics for `MeshManager`: `status` and connection-info
//! helpers. Split out of `daemon/mod.rs`.

use super::super::*;

impl MeshManager {
    /// Part of the embedding API: snapshot the daemon's status (identity,
    /// networks, peers).
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

        // STANDBY-PER-NETWORK: the top-level `active` used to mirror the one
        // daemon-wide flag directly; now that data-plane activation is
        // per-network, it's "is at least one network's data plane up" —
        // matches the pre-existing `tetron status` banner semantics ("up"
        // unless everything is on standby) without a wire-format change.
        let active = statuses.iter().any(|s| s.active);

        IpcMessage::StatusResponse {
            endpoint_id: self.endpoint.id(),
            active,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            networks: statuses,
            packets_rx: self.stats.packets_rx.get(),
            packets_tx: self.stats.packets_tx.get(),
            bytes_rx: self.stats.bytes_rx.get(),
            bytes_tx: self.stats.bytes_tx.get(),
            drops: ipc::DropCounts {
                send_failure: self.stats.drop_count(crate::stats::DropReason::SendFailure),
                no_peer: self.stats.drop_count(crate::stats::DropReason::NoPeer),
                malformed: self.stats.drop_count(crate::stats::DropReason::Malformed),
                backpressure: self.stats.drop_count(crate::stats::DropReason::Backpressure),
                spoof: self.stats.drop_count(crate::stats::DropReason::Spoof),
                fragmentation_failed: self
                    .stats
                    .drop_count(crate::stats::DropReason::FragmentationFailed),
            },
            fragmented_ipv4: self.stats.fragmented_ipv4.get(),
            fragmented_ipv6: self.stats.fragmented_ipv6.get(),
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
        let (members, member_count, nuke_proposals, subnet, subnet_str, nuke_consensus_threshold) = {
            let s = match h.state.read() {
                Ok(s) => s,
                Err(_) => {
                    return NetworkStatus {
                        network: h.name.clone(),
                        role,
                        my_ip: h.my_ip,
                        my_ipv6: Some(derive_ipv6(&my_id, &h.network_key)),
                        my_hostname: None,
                        network_key: Some(h.network_key.to_string()),
                        member_count: 0,
                        peers: vec![],
                        nuke_proposals: vec![],
                        tun_name: h.tun_name.lock().unwrap().clone(),
                        active: h.active.load(Ordering::SeqCst),
                        subnet: {
                            let (base, prefix) = crate::membership::default_subnet();
                            format!("{base}/{prefix}")
                        },
                        nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
                    };
                }
            };
            // STATUS-005: excludes self, matching `peers` below (built from
            // the same roster with the identical filter) and the documented
            // "member count excludes self" behavior -- this used to count
            // every roster entry including self, one too many.
            let count = s.members.all().iter().filter(|m| m.identity != my_id).count();
            let now = now_secs();
            let nuke_ttl = config::load()
                .ok()
                .and_then(|c| c.nuke_proposal_ttl)
                .unwrap_or(crate::membership::NUKE_PROPOSAL_TTL_SECS);
            let proposals =
                crate::membership::active_nuke_proposers(&s.nuke_proposals, now, nuke_ttl)
                    .into_iter()
                .map(|id| ipc::NukeProposalInfo {
                    short_id: id.chars().take(10).collect(),
                    proposed_at: s.nuke_proposals[id],
                })
                .collect();
            let (base, prefix) = s.subnet;
            (
                s.roster(),
                count,
                proposals,
                s.subnet,
                format!("{base}/{prefix}"),
                s.nuke_consensus_threshold,
            )
        };
        // Index live connections by endpoint id for a fast lookup.
        let connected: HashMap<EndpointId, Connection> = h
            .peers
            .peers_for_network_with_conn(&h.name)
            .into_iter()
            .map(|(eid, _, conn)| (eid, conn))
            .collect();
        let peers = members
            .iter()
            .filter(|m| m.identity != my_id)
            .map(|m| {
                let connection = connected
                    .get(&m.identity)
                    .map(|conn| Self::gather_conn_info(conn, subnet, &h.network_key));
                PeerStatus {
                    endpoint_id: m.identity,
                    ip: m.ip,
                    ipv6: Some(derive_ipv6(&m.identity, &h.network_key)),
                    hostname: m.hostname.clone(),
                    connection,
                    is_coordinator: m.is_coordinator,
                }
            })
            .collect();
        // Our own hostname comes from the signed roster (Magic DNS removed).
        let my_hostname = members
            .iter()
            .find(|m| m.identity == my_id)
            .and_then(|m| m.hostname.clone());
        NetworkStatus {
            network: h.name.clone(),
            role,
            my_ip: h.my_ip,
            my_ipv6: Some(derive_ipv6(&self.identity.local_identity(), &h.network_key)),
            my_hostname,
            network_key: Some(h.network_key.to_string()),
            member_count,
            peers,
            nuke_proposals,
            tun_name: h.tun_name.lock().unwrap().clone(),
            active: h.active.load(Ordering::SeqCst),
            subnet: subnet_str,
            nuke_consensus_threshold,
        }
    }

    pub(crate) fn gather_conn_info(
        conn: &iroh::endpoint::Connection,
        network_subnet: crate::membership::Subnet,
        network_key: &EndpointId,
    ) -> ipc::ConnectionInfo {
        let paths = conn.paths();
        // Classify every path, then pick which one to report. iroh only marks a
        // path `is_selected()` once its path-selector has promoted a winner;
        // during establishment, holepunch, or migration no path is selected even
        // though the connection is live and carrying traffic. Reporting only the
        // selected path then renders a working connection as `?`. `choose_path`
        // falls back to the best available (Direct > Relay > Tor) so a live
        // connection always reports a concrete path.
        //
        // PATHBLEED-STATUS-001: `in_subnet` additionally checks each path's own
        // address against *this specific network's own* subnet -- iroh's
        // `RemoteStateActor` shares path-selection state across every network a
        // peer is a member of, so `is_selected()` alone can be poisoned by a
        // path that legitimately belongs to a *different* one of that peer's
        // networks (PATH-BLEED-001). Relay/Tor addresses are not network-scoped
        // (tetron's relay/discovery config is daemon-wide), so there is nothing
        // bleed-shaped to check there -- always `true`.
        //
        // PATHBLEED-STATUS-002: `has_activity` corroborates a selected path with
        // its own real traffic (`stats().udp_tx.bytes`) before trusting it --
        // a freshly-opened, never-actually-used bled candidate reads as zero.
        let classes: Vec<(ipc::ConnType, bool, bool, bool)> = paths
            .iter()
            .map(|p| {
                let addr = p.remote_addr();
                let (ct, in_subnet) = if addr.is_relay() {
                    (ipc::ConnType::Relay, true)
                } else if addr.is_custom() {
                    (ipc::ConnType::Tor, true)
                } else {
                    let in_subnet = match addr {
                        iroh::TransportAddr::Ip(socket_addr) => match socket_addr.ip() {
                            std::net::IpAddr::V4(v4) => {
                                crate::membership::ip_in_subnet(v4, network_subnet)
                            }
                            std::net::IpAddr::V6(v6) => {
                                crate::membership::ipv6_in_network(v6, network_key)
                            }
                        },
                        _ => true,
                    };
                    (ipc::ConnType::Direct, in_subnet)
                };
                let has_activity = p.stats().udp_tx.bytes > 0;
                (ct, p.is_selected(), in_subnet, has_activity)
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
            max_datagram_size: conn.max_datagram_size().map(|sz| sz as u64),
        }
    }
}
