//! iroh endpoint setup and peer connection management.
//!
//! Each network gets its own ALPN (`tetron/net/<version>/<prefix>`) for isolation
//! and mesh-protocol version gating (see `MESH_PROTOCOL_VERSION`).
//! A single shared iroh [`Endpoint`] handles all networks, filtering by ALPN on accept.

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    address_lookup::{PkarrPublisher, PkarrResolver},
    endpoint::Connection,
    endpoint::presets,
    endpoint::{Builder, QuicTransportConfig},
};

use std::sync::Arc;

use crate::config::ServerOverride;
use crate::path_selector::{PathPreferenceSlot, TetronPathSelector};

/// TOR-DIAL-001: a type-erased handle to Tor's own peer-address resolution
/// (`TorCustomTransport::discovery()`), kept regardless of the `tor` cargo
/// feature (an empty `None` slot when not built/enabled) so callers like
/// [`connect_to_peer_with_alpn`] and [`crate::daemon::MeshManager`] don't need
/// their own `#[cfg(feature = "tor")]` gating just to hold this value --
/// mirrors `veilid_node_id`'s "always-present slot" pattern just below.
/// Unlike `veilid_node_id`, this never changes after `bind_endpoint` returns:
/// a Tor onion address is a pure, instant function of the peer's own
/// `EndpointId` (`TorAddressLookup::resolve`, `iroh-tor-transport`'s own
/// source), not an asynchronously-resolved value, so no `ArcSwap` is needed.
pub type TorAddrLookup = Arc<dyn iroh::address_lookup::AddressLookup>;

/// Compiled-default fixed UDP port the endpoint binds so users can
/// port-forward a stable, known port for guaranteed direct reachability
/// (Tailscale-style). Unlike an ephemeral port, this stays the same across
/// daemon restarts, so a manual router forward keeps working and the external
/// NAT mapping doesn't churn. iroh still does automatic NAT traversal
/// (UPnP/NAT-PMP/PCP), discovery, and relay fallback on top of this. If the
/// port is already taken, the endpoint falls back to an ephemeral port (see
/// `create_endpoint_with_alpns`). Overridable via `tetron config set
/// listen-port <port>` (CONFIG-AUDIT-002) — daemon-wide, not per-network, since
/// one shared iroh `Endpoint`/UDP socket serves every joined network.
pub const TETRON_LISTEN_PORT: u16 = 43737;

/// Mesh wire-protocol version, embedded in the per-network ALPN. Bump this on any
/// breaking change to the mesh control/forwarding protocol. Because iroh negotiates
/// the ALPN during the QUIC handshake, two peers on different mesh versions share no
/// common ALPN and simply cannot connect — the version gate is enforced by the
/// transport, with no in-band handshake. Per-network discovery still keys on the
/// pubkey prefix; the version is an independent leading segment.
pub const MESH_PROTOCOL_VERSION: u32 = 1;

pub fn network_alpn(network_pubkey: &EndpointId) -> Vec<u8> {
    let full = network_pubkey.to_string();
    let prefix = &full[..full.len().min(16)];
    format!("tetron/net/{MESH_PROTOCOL_VERSION}/{prefix}").into_bytes()
}

/// TOR-DIAL-001, Fix 7: the Tor transport's own build step (below) is a
/// real, stateful side effect on the Tor daemon (`ADD_ONION` + the
/// HSDir-quorum wait, TOR-DIAL-001) — not something safe to redo silently.
/// This alias lets `create_endpoint_with_alpns` build it exactly once
/// regardless of the `tor` cargo feature (`()` when not compiled in, so
/// callers don't need their own `#[cfg]` gating), and thread the *same*
/// already-built instance into every `bind_endpoint` attempt below.
#[cfg(feature = "tor")]
pub type TorTransportHandle = Option<Arc<iroh_tor_transport::TorCustomTransport>>;
#[cfg(not(feature = "tor"))]
pub type TorTransportHandle = ();

/// Creates an iroh endpoint with the N0 preset (NAT traversal + relay fallback).
/// When `tor`/`veilid` is true and the matching cargo feature is enabled, adds
/// that custom transport alongside the default relay transport. `listen_port`
/// overrides [`TETRON_LISTEN_PORT`] (CONFIG-AUDIT-002); pass the constant
/// itself for the compiled-default behavior.
/// Returns the bound endpoint and a live-updating slot for this node's own
/// Veilid `NodeId` (string form, `None` until resolved) -- the caller
/// stores the `Arc` on [`crate::daemon::MeshManager`] for the
/// own-roster-entry/join-handshake population VEILID-002/003/004 build on.
/// Always present, even when built without `--features veilid` or no
/// joined network requested it (in which case it just stays empty). Kept
/// live rather than a one-shot snapshot because identity resolution has
/// been observed taking several minutes (VEILID-005/006, see
/// `spec/core.py`) -- far too long to block on here, but also too long for
/// a value captured once at boot to be useful in practice.
// `TorTransportHandle` is `()` without the `tor` feature (see its own doc
// comment) -- `let_unit_value`/`clone_on_copy` fire only in that build and
// are the deliberate cost of keeping this function's body identical across
// both feature configurations rather than duplicating it.
#[allow(
    clippy::too_many_arguments,
    clippy::let_unit_value,
    clippy::clone_on_copy,
    clippy::unit_arg
)]
pub async fn create_endpoint_with_alpns(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
    tor: bool,
    veilid: bool,
    relay: &ServerOverride,
    discovery: &ServerOverride,
    listen_port: u16,
    path_preference: PathPreferenceSlot,
) -> Result<(
    Endpoint,
    Arc<arc_swap::ArcSwapOption<String>>,
    Option<TorAddrLookup>,
    TorTransportHandle,
)> {
    // TOR-DIAL-001, Fix 7: build the Tor transport exactly once, before
    // either bind attempt below, and reuse this same instance for both.
    // Building it fresh inside a *retried* bind_endpoint() call (the
    // pre-Fix-7 shape) created a second ephemeral onion service every time
    // the fixed port was unavailable, and — since the first attempt's
    // whole successful result (control connection included) was discarded
    // when only the later, unrelated QUIC `.bind()` step failed — tore the
    // first one down mid-flight (Tor's `hs_service_del_ephemeral` +
    // `circuit_mark_for_close_`, confirmed live). See spec/core.py's
    // `TorDialPathWiring`, Fix 7, for the full investigation.
    #[cfg(feature = "tor")]
    let tor_transport: TorTransportHandle = if tor {
        Some(
            iroh_tor_transport::TorCustomTransport::builder()
                .build(secret_key.clone())
                .await
                .context(
                    "failed to create Tor transport — is Tor running with ControlPort 9051?",
                )?,
        )
    } else {
        None
    };
    #[cfg(not(feature = "tor"))]
    let tor_transport: TorTransportHandle = {
        if tor {
            anyhow::bail!("Tor support requires building with --features tor");
        }
    };

    // TOR-DIAL-001, Fix 12: `TorCustomTransport` owns the live Tor control
    // connection that its own `ADD_ONION` (non-detached) is scoped to --
    // its doc comment says so explicitly: "the hidden service is removed
    // when this connection is dropped". Neither `bind_endpoint` below nor
    // its caller previously kept any owning reference to this value once
    // endpoint setup finished (only the separate, stateless
    // `TorAddrLookup` was threaded onward) -- live-verified via Tor's own
    // `hs_service_del_ephemeral()` firing in the same second as "Tor
    // transport enabled" on both sides of a real cross-machine test,
    // regardless of whether iroh's own custom-transport registration keeps
    // a clone alive internally. Clone the keepalive handle now, before
    // `tor_transport` is consumed below, and return it so the caller can
    // store it for the life of the daemon (mirrors `tor_addr_lookup`'s own
    // "always-present slot on MeshManager" pattern). See
    // `TorDialPathWiring`, Fix 12, for the full investigation.
    let tor_transport_keepalive: TorTransportHandle = tor_transport.clone();

    // Bind the fixed port so the daemon is reachable on a known, forwardable UDP
    // port across restarts. The builder is consumed by `.bind()`, so we rebuild
    // it for the ephemeral fallback. Falling back keeps the `0.0.0.0:0` guarantee
    // that the daemon always starts even if the fixed port is already in use.
    let fixed = format!("0.0.0.0:{listen_port}");
    let (ep, veilid_node_id, tor_addr_lookup) = match bind_endpoint(
        &secret_key,
        &alpns,
        tor_transport.clone(),
        veilid,
        &fixed,
        relay,
        discovery,
        path_preference.clone(),
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(
                port = listen_port,
                error = %e,
                "fixed UDP port unavailable; falling back to an ephemeral port"
            );
            bind_endpoint(
                &secret_key,
                &alpns,
                tor_transport,
                veilid,
                "0.0.0.0:0",
                relay,
                discovery,
                path_preference,
            )
            .await
            .context("failed to bind iroh endpoint")?
        }
    };

    tracing::info!(id = %ep.id().fmt_short(), "iroh endpoint ready");

    Ok((ep, veilid_node_id, tor_addr_lookup, tor_transport_keepalive))
}

/// Builds and binds an iroh endpoint at `bind` with the N0 preset and (when
/// requested + compiled in) the Tor and/or Veilid custom transports. Factored
/// out so the caller can retry with a different bind address after a port
/// collision -- safe to call more than once because every side-effecting
/// build step (currently: Tor's `ADD_ONION`) already happened once in the
/// caller and is only *wired onto* the builder here (Fix 7); Veilid's own
/// `build()` call still happens inside this function on each attempt, since
/// (unlike Tor's) it has no comparable externally-visible, non-idempotent
/// side effect to worry about duplicating.
// See `create_endpoint_with_alpns`'s identical allow for why.
#[allow(clippy::too_many_arguments, clippy::let_unit_value)]
async fn bind_endpoint(
    secret_key: &SecretKey,
    alpns: &[Vec<u8>],
    tor_transport: TorTransportHandle,
    veilid: bool,
    bind: &str,
    relay: &ServerOverride,
    discovery: &ServerOverride,
    path_preference: PathPreferenceSlot,
) -> Result<(
    Endpoint,
    Arc<arc_swap::ArcSwapOption<String>>,
    Option<TorAddrLookup>,
)> {
    #[allow(unused_mut)]
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(secret_key.clone())
        .alpns(alpns.to_vec())
        .clear_ip_transports()
        .bind_addr(bind)
        .context("invalid bind address")?
        // PATHPREF-001: overrides iroh's own default `BiasedRttPathSelector`
        // with tetron's own, which delegates to that same default logic
        // (re-exported for exactly this, `vendor/iroh-1.0.3/PATCH.md` Patch
        // 6) whenever no preference is set -- so `auto` behavior is
        // unchanged from before this feature existed.
        .path_selector(
            Arc::new(TetronPathSelector::new(path_preference)) as Arc<dyn iroh::PathSelector>
        )
        // Rayfish's data plane is a single stream of QUIC datagrams per peer
        // (TUN packets → `send_datagram`), with a few reliable control streams per
        // connection. Tune the transport config for that shape:
        //   - `send_fairness(false)`: no competing data streams of equal priority
        //     to round-robin, so fairness scheduling is pure overhead. (Affects
        //     stream scheduling only, not datagrams, but is the correct setting and
        //     removes a small amount of per-packet work.)
        //   - GSO on (default): confirmed explicit so a future change can't silently
        //     regress it. GSO coalesces same-destination segments into one sendmsg,
        //     cutting syscalls under burst.
        //   - Datagrams enabled (iroh/noq default `Some` receive buffer); the send
        //     buffer stays at the 1 MiB default, sized via `datagram_send_buffer_space`
        //     on the hot path (see `forward::run_mesh`).
        // The congestion controller stays at the noq default (Cubic). Switching to
        // BBR3 would help on lossy/shallow-buffer consumer uplinks but requires a
        // `noq-proto` dependency to reach the config type — deferred to a measured
        // follow-up (see iroh-audit BASELINE.md, cross-parameter sweep).
        .transport_config(quic_transport_config());

    // Override the N0 preset's relay / discovery defaults when configured.
    if let Some(mode) = build_relay_mode(relay)? {
        builder = builder.relay_mode(mode);
    }
    builder = apply_discovery(builder, discovery)?;

    // TOR-DIAL-001: `tor_addr_lookup` is `connect_to_peer_with_alpn`'s own
    // source of a Tor candidate address at dial time -- see that function
    // for why `.address_lookup(...)` alone (registered on the endpoint just
    // below) is not sufficient. Always present as an empty slot, mirroring
    // `veilid_node_id` below, so the caller doesn't need its own
    // `#[cfg(feature = "tor")]` gating just to hold this value.
    //
    // Fix 7: `tor_transport` is already built (by `create_endpoint_with_alpns`,
    // once, before either bind attempt) -- registering it here is purely
    // wiring onto this specific builder instance, with no side effect of
    // its own, so it's safe for this whole function to be retried.
    #[cfg(feature = "tor")]
    let tor_addr_lookup: Option<TorAddrLookup> = if let Some(tor_transport) = &tor_transport {
        let lookup: TorAddrLookup = Arc::new(tor_transport.discovery());
        builder = builder
            .add_custom_transport(
                tor_transport.clone() as Arc<dyn iroh::endpoint::transports::CustomTransport>
            )
            .address_lookup(tor_transport.discovery());
        tracing::info!("Tor transport enabled");
        Some(lookup)
    } else {
        None
    };

    #[cfg(not(feature = "tor"))]
    let tor_addr_lookup: Option<TorAddrLookup> = {
        let _ = tor_transport;
        None
    };

    // Unlike Tor, no `.address_lookup(...)` is registered here: a custom
    // transport's local address is NOT automatically included in what
    // `Endpoint`'s own pkarr publisher sends (`socket.rs::publish_my_addr`
    // only ever builds its address list from direct/relay addrs, by design —
    // the same isolation boundary that makes Tor need its own separate
    // discovery mechanism applies here). VEILID-003 (see `spec/core.py`)
    // resolves peers instead by injecting a `TransportAddr::Custom` built
    // from the roster's `Member.veilid_node_id` directly into the
    // per-peer `EndpointAddr` at dial time (`connect_to_peer_with_alpn`),
    // rather than through iroh's generic discovery hook.
    // Always present (an empty slot when Veilid isn't used or isn't
    // compiled in), so `MeshManager` can hold one field regardless of the
    // `veilid` feature. VEILID-006: unlike an `Option<String>` snapshot,
    // this is a live cell the background task below keeps updated for as
    // long as it takes -- identity resolution has been observed taking
    // several minutes even with the IPv4 fix, well past anything sane to
    // block `bind_endpoint`'s own return on (see VEILID-005). Every
    // consumer reads it fresh at the point of use rather than capturing a
    // boot-time value, so a late-arriving identity still reaches the
    // roster once resolved, not just on the next restart.
    let veilid_node_id: Arc<arc_swap::ArcSwapOption<String>> =
        Arc::new(arc_swap::ArcSwapOption::empty());

    #[cfg(feature = "veilid")]
    if veilid {
        let veilid_transport = veilid_transport::VeilidTransportBuilder::new()
            .build()
            .await
            .context("failed to start embedded Veilid node")?;
        tracing::info!("Veilid transport enabled; own identity resolving in the background");
        let slot = veilid_node_id.clone();
        let transport_for_wait = veilid_transport.clone();
        tokio::spawn(async move {
            let node_id = transport_for_wait.wait_for_own_node_id().await;
            tracing::info!(%node_id, "Veilid: own identity now available to the daemon");
            slot.store(Some(Arc::new(node_id.to_string())));
        });
        builder =
            builder
                .add_custom_transport(Arc::new(veilid_transport)
                    as Arc<dyn iroh::endpoint::transports::CustomTransport>);
    }

    #[cfg(not(feature = "veilid"))]
    if veilid {
        anyhow::bail!("Veilid support requires building with --features veilid");
    }

    let ep = builder
        .bind()
        .await
        .context("failed to bind iroh endpoint")?;
    Ok((ep, veilid_node_id, tor_addr_lookup))
}

/// Builds the [`QuicTransportConfig`] for tetron's data-plane shape (one stream
/// of QUIC datagrams per peer, plus a few reliable control streams).
///
/// Starts from iroh's builder defaults (which carry the multipath / NAT-traversal
/// / heartbeat settings required for holepunching) and only overrides the
/// datagram-relevant knobs. See `bind_endpoint` for the rationale.
fn quic_transport_config() -> QuicTransportConfig {
    QuicTransportConfig::builder()
        // No competing data streams of equal priority → disable round-robin
        // fairness scheduling (removes overhead; correct for a single datagram
        // stream per peer).
        .send_fairness(false)
        // Keep GSO on (default) explicitly so a future change can't silently
        // regress it.
        .enable_segmentation_offload(true)
        // No max_idle_timeout override: stays at iroh/quinn's own 30s default
        // (CONN-STABILITY-001, reverting HARDEN-007's 10s tightening). A
        // relay-tunneled connection between two admitted mesh peers has no
        // reliable activity within a 10s window on its own — the relay
        // protocol's own ping/pong heartbeat runs on a ~15s+jitter cadence,
        // so a 10s ceiling guarantees the connection dies before that
        // heartbeat ever gets a chance to fire even once. 30s gives it room
        // to engage and then sustains the connection indefinitely
        // (empirically confirmed: 0 reconnects over 250s at 30s vs. 14
        // reconnects in 180s at 10s, identical relay-forced-idle setup).
        .build()
}

/// Build a custom [`RelayMode`] from a relay override, or `None` when unset (in
/// which case the N0 preset's default relays are kept). Replace mode uses only
/// the configured relays; augment mode appends n0's default relay URLs so the
/// node keeps the n0 fallback.
///
/// RELAY-001: `off` is a distinct third case from unset -- maps to iroh's own
/// `RelayMode::Disabled` ("disable relay servers completely") rather than
/// falling through to URL parsing, which would reject `off` as an invalid URL.
pub fn build_relay_mode(o: &ServerOverride) -> Result<Option<RelayMode>> {
    if o.servers == ["off"] {
        return Ok(Some(RelayMode::Disabled));
    }
    let urls = crate::config::relay_urls(o)?;
    if urls.is_empty() {
        return Ok(None);
    }
    let mut parsed: Vec<RelayUrl> = urls
        .iter()
        .map(|u| u.parse().with_context(|| format!("invalid relay URL: {u}")))
        .collect::<Result<_>>()?;
    if !o.replace {
        parsed.extend(RelayMode::Default.relay_map().urls::<Vec<RelayUrl>>());
    }
    Ok(Some(RelayMode::custom(parsed)))
}

/// Apply a discovery-DNS override to the endpoint builder. Each configured URL
/// is registered as a pkarr publisher + resolver. Replace mode first clears the
/// preset's address-lookup services (n0 pkarr/DNS); augment mode stacks on top.
fn apply_discovery(mut builder: Builder, o: &ServerOverride) -> Result<Builder> {
    let urls = crate::config::discovery_urls(o)?;
    if urls.is_empty() {
        return Ok(builder);
    }
    if o.replace {
        builder = builder.clear_address_lookup();
    }
    for u in urls {
        let url: url::Url = u
            .parse()
            .with_context(|| format!("invalid discovery URL: {u}"))?;
        builder = builder
            .address_lookup(PkarrPublisher::builder(url.clone()))
            .address_lookup(PkarrResolver::builder(url));
    }
    Ok(builder)
}

/// Connects to a peer by EndpointId with a specific ALPN. iroh handles
/// NAT traversal and falls back to relay if direct connection fails.
///
/// If the peer address cache (see `peercache.rs`) has known addresses for
/// this peer, they are included in the [`EndpointAddr`] so iroh tries them
/// directly before falling back to DHT lookup — this enables reconnection
/// after an all-offline gap without a functioning pkarr relay.
///
/// `veilid_node_id` (VEILID-003), when `Some`, is the target peer's own
/// Veilid `NodeId` (string form, from their roster `Member.veilid_node_id`)
/// -- resolved here into a `TransportAddr::Custom` and appended, since a
/// custom transport's address is not discoverable through iroh's normal
/// discovery path (see `bind_endpoint`'s doc comment on the same point).
///
/// `tor_addr_lookup` (TOR-DIAL-001), when `Some`, is queried the same way for
/// a Tor onion candidate. Unlike Veilid's, this doesn't need a per-peer
/// roster field: a peer's onion address is a pure function of `id` itself
/// (`TorAddressLookup::resolve`, `iroh-tor-transport`), so any node can
/// derive any other node's Tor candidate unconditionally, the moment Tor is
/// enabled locally -- the lookup is only ever consulted here (not via
/// `.address_lookup(...)` on the endpoint) because `ep.connect()` never
/// invokes the registered discovery hook when the caller already supplies
/// known addresses via `EndpointAddr`, which the peercache above almost
/// always does for an already-admitted mesh peer.
pub async fn connect_to_peer_with_alpn(
    ep: &Endpoint,
    id: EndpointId,
    veilid_node_id: Option<&str>,
    tor_addr_lookup: Option<&TorAddrLookup>,
    alpn: &[u8],
) -> Result<Connection> {
    #[cfg_attr(not(feature = "veilid"), allow(unused_mut))]
    let mut addrs = crate::peercache::lookup(&id);
    #[cfg(feature = "veilid")]
    match veilid_node_id {
        Some(vnid) => match vnid.parse::<veilid_transport::NodeId>() {
            Ok(node_id) => {
                tracing::debug!(peer = %id.fmt_short(), veilid_node_id = %node_id, "dialing with a Veilid custom-transport candidate address");
                addrs
                    .get_or_insert_with(Vec::new)
                    .push(iroh::TransportAddr::Custom(
                        veilid_transport::node_id_to_custom_addr(&node_id),
                    ));
            }
            Err(e) => {
                tracing::warn!(
                    peer = %id.fmt_short(),
                    error = %e,
                    "invalid veilid_node_id in roster entry, skipping"
                );
            }
        },
        None => {
            tracing::debug!(peer = %id.fmt_short(), "dialing with no Veilid candidate address (peer's veilid_node_id not yet known)");
        }
    }
    #[cfg(not(feature = "veilid"))]
    let _ = veilid_node_id;

    if let Some(lookup) = tor_addr_lookup
        && let Some(mut stream) = lookup.resolve(id)
    {
        use futures::StreamExt;
        match stream.next().await {
            Some(Ok(item)) => {
                let tor_addr = item.into_endpoint_addr();
                tracing::debug!(peer = %id.fmt_short(), "dialing with a Tor custom-transport candidate address");
                addrs.get_or_insert_with(Vec::new).extend(tor_addr.addrs);
            }
            Some(Err(e)) => {
                tracing::warn!(peer = %id.fmt_short(), error = %e, "Tor address lookup failed, dialing without a Tor candidate");
            }
            None => {
                tracing::debug!(peer = %id.fmt_short(), "Tor address lookup returned no candidate");
            }
        }
    }

    let addr: EndpointAddr = match addrs {
        Some(addrs) => EndpointAddr::from_parts(id, addrs),
        None => id.into(),
    };
    let conn = match ep.connect(addr, alpn).await {
        Ok(conn) => conn,
        // An ALPN mismatch fails the QUIC/TLS handshake opaquely. Map that one
        // case to an actionable hint (it's a heuristic — a peer that isn't
        // running tetron at all looks similar — hence "may be").
        Err(e) if is_alpn_mismatch(&e.to_string()) => {
            return Err(e).context(
                "no shared protocol with peer — it may be running an incompatible \
                 tetron version (upgrade the older node)",
            );
        }
        Err(e) => return Err(e).context("failed to connect to peer"),
    };
    tracing::info!(
        peer = %conn.remote_id().fmt_short(),
        alpn = %String::from_utf8_lossy(alpn),
        "connected to peer"
    );
    Ok(conn)
}

/// Heuristic: does a connect error look like an ALPN mismatch (no protocol the
/// two peers share)? iroh/quinn surfaces this as "peer doesn't support any known
/// protocol" / a TLS `no_application_protocol` alert. Matching the message keeps
/// us robust across iroh patch releases without depending on exact error enums.
pub(crate) fn is_alpn_mismatch(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("known protocol") || e.contains("application protocol")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn test_network_alpn() {
        let key = SecretKey::generate().public();
        let alpn = network_alpn(&key);
        let key_str = key.to_string();
        let expected = format!("tetron/net/{MESH_PROTOCOL_VERSION}/{}", &key_str[..16]);
        assert_eq!(alpn, expected.as_bytes());
    }

    #[test]
    fn relay_mode_augment_vs_replace() {
        // Unset: keep the preset default (None).
        assert!(
            build_relay_mode(&ServerOverride::default())
                .unwrap()
                .is_none()
        );

        // A parseable relay URL (iroh RelayUrl requires a host).
        let custom = "https://relay.example.com".to_string();

        // Replace: only the custom relay.
        let rep = ServerOverride {
            servers: vec![custom.clone()],
            replace: true,
        };
        let mode = build_relay_mode(&rep).unwrap().expect("some mode");
        assert_eq!(mode.relay_map().urls::<Vec<RelayUrl>>().len(), 1);

        // Augment: custom + n0 defaults (more than one).
        let aug = ServerOverride {
            servers: vec![custom],
            replace: false,
        };
        let mode = build_relay_mode(&aug).unwrap().expect("some mode");
        assert!(mode.relay_map().urls::<Vec<RelayUrl>>().len() > 1);
    }

    #[test]
    fn relay_mode_off_disables_relay_entirely() {
        // RELAY-001: distinct from unset (`None`, keeps the n0 default) --
        // `off` maps to iroh's own `RelayMode::Disabled`.
        let off = ServerOverride {
            servers: vec!["off".to_string()],
            replace: true,
        };
        let mode = build_relay_mode(&off).unwrap().expect("some mode");
        assert_eq!(mode, RelayMode::Disabled);
    }

    #[test]
    fn alpn_mismatch_classifier() {
        // iroh/quinn phrasings for "no shared ALPN".
        assert!(is_alpn_mismatch(
            "connection closed: peer doesn't support any known protocol"
        ));
        assert!(is_alpn_mismatch(
            "the cryptographic handshake failed: no application protocol"
        ));
        // Unrelated failures must not be misclassified as version mismatches.
        assert!(!is_alpn_mismatch("connection timed out"));
        assert!(!is_alpn_mismatch("connection refused"));
    }
}
