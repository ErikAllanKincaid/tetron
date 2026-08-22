//! Read-only diagnostics for `MeshManager`: `status` and connection-info
//! helpers. Split out of `daemon/mod.rs`.

use super::super::*;

/// STATUS-CACHE-001: the cached half of a status response -- everything whose
/// construction is expensive. The scalar counters are deliberately absent:
/// they are atomic reads, so caching them would only make traffic and drop
/// figures stale for no saving.
pub struct StatusSnapshot {
    networks: Vec<NetworkStatus>,
    built_at: std::time::Instant,
}

impl MeshManager {
    /// Drop the cached status snapshot, forcing the next read to rebuild.
    /// Called for every non-read-only IPC message (see
    /// `invalidates_status_snapshot`) and therefore also by `Sync`, which is
    /// what makes tetron-webui's existing per-network `sync` button double as
    /// a manual status refresh with no new wire message.
    ///
    /// STATUS-CACHE-001 (embedder gap, 2026-08-22): `pub`, not
    /// `pub(crate)` -- `handle_request`'s call above only covers the
    /// desktop Unix-socket IPC dispatch loop. An embedder built on
    /// `build_headless()` (no IPC socket, e.g. `tetron-mobile`'s `Node`)
    /// calls `MeshManager` methods directly and never passes through
    /// that loop, so it must be able to call this itself after its own
    /// mutating calls (join/leave/activate/deactivate/...) or its status
    /// reads stay stale for a full `status-cache.interval` after
    /// anything the embedder's own user just did.
    pub fn invalidate_status_snapshot(&self) {
        if let Ok(mut g) = self.status_snapshot.write() {
            *g = None;
        }
    }

    /// The per-network status list, from cache when it is younger than
    /// `status-cache.interval`, rebuilt otherwise.
    ///
    /// The rebuild is lazy: a daemon nobody queries never does this work at
    /// all, while any number of polling clients -- systray every 8s, a webui
    /// tab every 10s, several tabs at once -- together cost at most one
    /// rebuild per interval. Daemon work is bounded by daemon policy rather
    /// than by client behavior it does not control.
    fn network_statuses_cached(&self) -> Vec<NetworkStatus> {
        let interval = status_cache_interval();
        let hit = self.status_snapshot.read().ok().and_then(|g| {
            g.as_ref()
                .filter(|s| !status_snapshot_is_stale(Some(s.built_at.elapsed()), interval))
                .map(|s| s.networks.clone())
        });
        if let Some(networks) = hit {
            return networks;
        }
        let networks = self.build_network_statuses();
        if let Ok(mut g) = self.status_snapshot.write() {
            *g = Some(StatusSnapshot {
                networks: networks.clone(),
                built_at: std::time::Instant::now(),
            });
        }
        networks
    }

    /// Build the per-network status list from scratch. This is the expensive
    /// half -- `network_status` walks every peer and `gather_conn_info` calls
    /// `conn.paths()` plus `p.stats()` per path -- and is what
    /// `network_statuses_cached` exists to keep off the per-request path.
    fn build_network_statuses(&self) -> Vec<NetworkStatus> {
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
        // PATHBLEED-STATUS-003 (corrected): every overlay subnet/network this
        // daemon manages, not just the one being queried -- gather_conn_info
        // needs the full set to recognize a candidate address that's
        // actually one of THIS daemon's own overlay addresses on a
        // *different* network (a self-captured/bled candidate,
        // PATH-BLEED-001), which the currently-queried network's own subnet
        // alone can't catch. Recovers a poisoned lock's data instead of
        // dropping that network from the exclusion set (same idiom as
        // logdir.rs/identity.rs's ENV_LOCK) -- this is a trust boundary, so
        // it must fail closed (keep checking) rather than open (silently
        // trust more).
        let managed_subnets: Vec<crate::membership::Subnet> = self
            .networks
            .iter()
            .map(|h| match h.state.read() {
                Ok(s) => s.subnet,
                Err(poisoned) => poisoned.into_inner().subnet,
            })
            .collect();
        let managed_network_keys: Vec<EndpointId> =
            self.networks.iter().map(|h| h.network_key).collect();
        self.networks
            .iter()
            .map(|h| {
                self.network_status(
                    &h,
                    my_id,
                    &direct_names,
                    &managed_subnets,
                    &managed_network_keys,
                )
            })
            .collect()
    }

    /// Part of the embedding API: snapshot the daemon's status (identity,
    /// networks, peers).
    ///
    /// STATUS-CACHE-001: the per-network list comes from the daemon-paced
    /// cache, while every scalar counter below is read live, so traffic and
    /// drop figures are always current even when the peer/path detail is up
    /// to `status-cache.interval` old.
    pub fn status(&self) -> IpcMessage {
        let statuses = self.network_statuses_cached();
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
                backpressure: self
                    .stats
                    .drop_count(crate::stats::DropReason::Backpressure),
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
        managed_subnets: &[crate::membership::Subnet],
        managed_network_keys: &[EndpointId],
    ) -> NetworkStatus {
        // Direct-connection networks are tagged `[direct]` regardless of role.
        let role = if direct_names.contains(&h.name) {
            NetworkRole::Direct
        } else {
            h.role.clone()
        };
        let (members, member_count, nuke_proposals, subnet_str, nuke_consensus_threshold) = {
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
                        nuke_consensus_threshold:
                            crate::membership::default_nuke_consensus_threshold(),
                    };
                }
            };
            // STATUS-005: excludes self, matching `peers` below (built from
            // the same roster with the identical filter) and the documented
            // "member count excludes self" behavior -- this used to count
            // every roster entry including self, one too many.
            let count = s
                .members
                .all()
                .iter()
                .filter(|m| m.identity != my_id)
                .count();
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
                let connection = connected.get(&m.identity).map(|conn| {
                    Self::gather_conn_info(conn, managed_subnets, managed_network_keys)
                });
                PeerStatus {
                    endpoint_id: m.identity,
                    ip: m.ip,
                    ipv6: Some(derive_ipv6(&m.identity, &h.network_key)),
                    hostname: m.hostname.clone(),
                    connection,
                    is_coordinator: m.is_coordinator,
                    // STATUS-006: roster-stamped, so an offline peer's age
                    // ("zombie" spotting) survives daemon restarts and
                    // replicates to every member via the signed blob.
                    last_seen: m.last_seen,
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
        managed_subnets: &[crate::membership::Subnet],
        managed_network_keys: &[EndpointId],
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
        // PATHBLEED-STATUS-003 (corrected): classify_candidate_addr checks a
        // Direct candidate's address against every overlay subnet/network
        // this daemon manages, not just this one -- a self-captured/bled
        // overlay address (a peer's own address on a *different* one of the
        // daemon's networks) is caught even though it isn't inside this
        // specific network's own subnet, while a genuine real address (never
        // inside any of them) is trusted. See that requirement's own
        // docstring for the full account, including the first (wrong) cut
        // this replaced.
        //
        // PATHBLEED-STATUS-002 (corrected): `has_activity` corroborates a
        // selected path with real *received* traffic (`stats().udp_rx.bytes`)
        // before trusting it -- transmitted-only (`udp_tx`) already counts a
        // path's own unvalidated `PATH_CHALLENGE` probe, so it doesn't
        // actually prove the path works; a freshly-opened, never-actually-
        // validated bled candidate reads real activity as zero.
        // PATH-DIAG-002: build the full per-candidate detail once, up front --
        // `choose_path_index`'s existing `classes` shape is derived from it
        // below rather than computed separately, so there is exactly one
        // classification pass, not two.
        let candidates: Vec<ipc::PathCandidateInfo> = paths
            .iter()
            .map(|p| {
                let addr = p.remote_addr();
                let (ct, in_subnet) =
                    classify_candidate_addr(addr, managed_subnets, managed_network_keys);
                // MTU-DIAG-002: one `stats()` call, read for everything it
                // already carries. `has_activity` needed it anyway
                // (PATHBLEED-STATUS-002); the MTU/probe fields below are
                // siblings of `udp_rx` on the same materialized struct, so
                // surfacing them costs nothing at runtime.
                let stats = p.stats();
                let has_activity = stats.udp_rx.bytes > 0;
                ipc::PathCandidateInfo {
                    conn_type: ct,
                    remote_addr: addr.to_string(),
                    is_selected: p.is_selected(),
                    in_subnet,
                    has_activity,
                    rtt_ms: Some(p.rtt().as_secs_f64() * 1000.0),
                    current_mtu: Some(stats.current_mtu),
                    black_holes_detected: Some(stats.black_holes_detected),
                    sent_plpmtud_probes: Some(stats.sent_plpmtud_probes),
                    lost_plpmtud_probes: Some(stats.lost_plpmtud_probes),
                }
            })
            .collect();

        let classes: Vec<(ipc::ConnType, bool, bool, bool)> = candidates
            .iter()
            .map(|c| {
                (
                    c.conn_type.clone(),
                    c.is_selected,
                    c.in_subnet,
                    c.has_activity,
                )
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
        let via_detail = classify_via_detail(&classes, &conn_type);

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
            paths: candidates,
            via_detail,
        }
    }
}

// --- STATUS-CACHE-001: the daemon paces status rebuilds, not its clients ---

/// Compiled default for `status-cache.interval`, in seconds.
pub const STATUS_CACHE_INTERVAL_SECS: u64 = 12;

/// Resolve the refresh floor: `status-cache.interval` if set and non-zero,
/// otherwise the compiled default. Zero is treated as "unset" (the same
/// convention the other knobs use) rather than as "rebuild every time",
/// because a client-driven unbounded rebuild rate is the exact thing this
/// requirement exists to prevent.
pub fn status_cache_interval() -> std::time::Duration {
    let secs = config::load()
        .ok()
        .and_then(|c| c.status_cache.interval_secs)
        .filter(|s| *s > 0)
        .unwrap_or(STATUS_CACHE_INTERVAL_SECS);
    std::time::Duration::from_secs(secs)
}

/// Whether the cached per-peer snapshot must be rebuilt. Pure, same shape as
/// `path_flap_decision` (PATH-DIAG-006) and `dial_retry_decision`
/// (CONVERGE-010). `age` is `None` when there is no snapshot at all.
///
/// This is a *floor* on rebuild frequency, evaluated lazily on read: a daemon
/// nobody queries never rebuilds, and any number of polling clients cost at
/// most one rebuild per interval between them.
pub(crate) fn status_snapshot_is_stale(
    age: Option<std::time::Duration>,
    interval: std::time::Duration,
) -> bool {
    match age {
        None => true,
        Some(a) => a >= interval,
    }
}

/// Whether handling `msg` should drop the cached snapshot.
///
/// The read-only queries are named explicitly and everything else
/// invalidates, so a mutating message added later fails *safe* (an extra
/// rebuild) rather than silently serving stale state. `Sync` is deliberately
/// in the invalidating set: its meaning is already "stop waiting for
/// intervals, get current state now", which is exactly the semantics a
/// manual refresh needs -- and `tetron-webui` already has a button wired to
/// it, so no new message and no addon change is required.
pub(crate) fn invalidates_status_snapshot(msg: &IpcMessage) -> bool {
    !matches!(
        msg,
        IpcMessage::Status | IpcMessage::AdminList { .. } | IpcMessage::InviteList { .. }
    )
}

#[cfg(test)]
mod status_cache_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn absent_snapshot_is_always_stale() {
        assert!(status_snapshot_is_stale(None, Duration::from_secs(12)));
    }

    #[test]
    fn fresh_snapshot_is_reused() {
        assert!(!status_snapshot_is_stale(
            Some(Duration::from_secs(3)),
            Duration::from_secs(12)
        ));
    }

    #[test]
    fn snapshot_at_exactly_the_interval_is_stale() {
        assert!(status_snapshot_is_stale(
            Some(Duration::from_secs(12)),
            Duration::from_secs(12)
        ));
    }

    #[test]
    fn older_snapshot_is_stale() {
        assert!(status_snapshot_is_stale(
            Some(Duration::from_secs(60)),
            Duration::from_secs(12)
        ));
    }

    #[test]
    fn status_query_does_not_invalidate() {
        assert!(!invalidates_status_snapshot(&IpcMessage::Status));
    }

    #[test]
    fn read_only_queries_do_not_invalidate() {
        assert!(!invalidates_status_snapshot(&IpcMessage::AdminList {
            network: "n".into()
        }));
        assert!(!invalidates_status_snapshot(&IpcMessage::InviteList {
            network: "n".into()
        }));
    }

    #[test]
    fn sync_invalidates_so_the_webui_button_forces_freshness() {
        assert!(invalidates_status_snapshot(&IpcMessage::Sync {
            network: None
        }));
        assert!(invalidates_status_snapshot(&IpcMessage::Sync {
            network: Some("n".into())
        }));
    }

    #[test]
    fn mutations_invalidate() {
        assert!(invalidates_status_snapshot(&IpcMessage::Leave {
            network: "n".into(),
            force: false,
        }));
        assert!(invalidates_status_snapshot(&IpcMessage::Standby {
            network: None
        }));
        assert!(invalidates_status_snapshot(&IpcMessage::Resume {
            hostname: None,
            network: None,
        }));
        assert!(invalidates_status_snapshot(&IpcMessage::Kick {
            network_key: "k".into(),
            endpoint_id: "e".into(),
        }));
    }
}
