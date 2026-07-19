//! Daemon process bootstrap and the IPC server. Moved out of `daemon/mod.rs`.
//!
//! `run_daemon` is the process entry point (called by the `tetron daemon`
//! command): it builds the shared [`MeshManager`], reconnects saved networks,
//! and runs the IPC accept loop until shutdown. `build_daemon` wires the endpoint
//! / TUN / protocol router / metrics; `serve_ipc` + `handle_ipc_client` answer
//! `tetron` CLI requests over the Unix socket. These live in a `mesh/` submodule
//! (a descendant of `daemon`) so they can still construct `MeshManager` and reach
//! its private fields without widening visibility.

use super::super::*;

pub async fn run_daemon(token: CancellationToken, stats: Arc<ForwardMetrics>) -> Result<()> {
    // This fork runs on a configurable overlay subnet, so instead of the removed
    // hardcoded CGNAT preflight (SUBNET-006) it refuses to start only if the
    // *chosen* overlay subnet would collide with an existing local network
    // (SUBNET-012) — which the safe default (10.88.0.0/24) does not next to
    // Tailscale's 100.64.0.0/10. Runs before anything is built, fail-fast.
    #[cfg(not(target_os = "android"))]
    tun::check_subnet_overlap(config::node_subnet())?;

    // Build the always-on infrastructure. Each saved network gets its own OS
    // TUN device now (MULTISEG-003), created inside `connect_all_networks`'s
    // per-network create/join/restore paths rather than once here — there is
    // no daemon-wide packet interface left to attach at this point. On Android
    // the packet interface is a `VpnService` fd attached later by the embedder
    // via `attach_tun`, per network, so this was already skipped there.
    let daemon = build_daemon(token.clone(), stats).await?;

    // Connect the control plane (mesh connections) once, for the daemon's
    // whole lifetime, then bring the data plane up. `tetron resume`/`tetron
    // standby` toggle only the data plane after this; connections persist
    // across `standby` so the node stays online to peers. Each network's TUN
    // is created as part of this call (MULTISEG-003), not before it.
    daemon.connect_all_networks().await;

    // Seed the peer address cache from the live connections we just established.
    // Subsequent reconnects (after an all-offline gap) will skip DHT lookup and
    // dial cached addresses directly (CACHE-001).
    daemon.refresh_peer_cache();

    daemon.activate(None, None).await;

    // Spawn a periodic task that saves the peer address cache to disk every
    // 5 minutes. The task exits when the shutdown token fires, saving one
    // final snapshot.
    crate::peercache::spawn_periodic_save(token.clone());

    // The promotion receiver was stashed on the daemon by the builder; take it
    // back to drive the IPC loop.
    let promote_rx = daemon
        .promote_rx
        .lock()
        .unwrap()
        .take()
        .expect("promote_rx present after build");
    let left_rx = daemon
        .left_rx
        .lock()
        .unwrap()
        .take()
        .expect("left_rx present after build");

    let result = serve_ipc(&daemon, promote_rx, left_rx, token).await;

    // Save one final cache snapshot before tearing down connections, so the
    // most recent peer addresses survive a restart (CACHE-001).
    daemon.refresh_peer_cache();
    crate::peercache::save();

    // Close the iroh endpoint before returning. Dropping it on return logs
    // "Endpoint dropped without calling `Endpoint::close`. Aborting
    // ungracefully." and can leave the process lingering until the service
    // manager escalates to SIGKILL — which delays the relaunch on
    // `tetron restart` past the client's reachability probe. Closing
    // it here lets QUIC connections terminate cleanly and the process exit
    // promptly so the new daemon comes up fast.
    daemon.endpoint.close().await;

    result
}

/// Construct all always-on daemon infrastructure: identity, iroh endpoint, blob
/// store, TUN device, forwarding loop, DNS resolver, protocol
/// router, and metrics server. Returns the shared [`MeshManager`] — still on
/// standby, so the caller is expected to run [`MeshManager::activate`] — and the
/// metrics-server guard, which must outlive the process.
/// The ALPNs the endpoint advertises at boot: one per saved network plus the
/// network-independent blobs ALPN. Mirrors `ProtocolRouter::alpns()`.
fn initial_alpns(app_config: &config::AppConfig) -> Vec<Vec<u8>> {
    let mut alpns: Vec<Vec<u8>> = app_config
        .networks
        .iter()
        .filter_map(|net| net.network_public_key.as_ref().map(transport::network_alpn))
        .collect();
    alpns.push(iroh_blobs::protocol::ALPN.to_vec());
    alpns
}

/// Construct a headless [`MeshManager`] for an embedder. Builds the same
/// infrastructure as `run_daemon` minus
/// the OS TUN device and the Unix-socket IPC server: the caller supplies a
/// packet interface via [`MeshManager::attach_tun`]. The returned daemon is on
/// standby (no data plane), with its saved networks' control plane connected.
pub async fn build_headless() -> Result<Arc<MeshManager>> {
    let token = CancellationToken::new();
    let stats = Arc::new(ForwardMetrics::default());
    let daemon = build_daemon(token, stats).await?;
    // Bring the saved networks' control plane up, matching `run_daemon`.
    daemon.connect_all_networks().await;
    Ok(daemon)
}

/// Build all always-on daemon infrastructure WITHOUT a packet interface or the
/// Unix-socket IPC server. The returned [`MeshManager`] is on standby (no data
/// plane); attach a TUN with [`MeshManager::attach_tun`], connect saved networks,
/// then bring the data plane up with [`MeshManager::activate`]. The promotion
/// receiver and metrics-server guard are stashed on the state for the caller.
///
/// Shared by [`run_daemon`] (desktop) and [`build_headless`] (embedders).
async fn build_daemon(
    token: CancellationToken,
    stats: Arc<ForwardMetrics>,
) -> Result<Arc<MeshManager>> {
    // Relocate a pre-/etc config tree into /etc/tetron (Linux upgrade path)
    // before anything reads identity or config. No-op on macOS / once migrated.
    config::migrate_location();

    // Initialise the peer address cache from disk. Must happen before any
    // connections are made so `connect_to_peer_with_alpn` can use cached
    // addresses on the very first dial (CACHE-001).
    crate::peercache::init(&config::config_dir()?);

    // --- Identity (persistent transport key) ---
    let key = identity::load_or_create()?;
    let public_key = key.public();
    let collision_index = identity::load_collision_index()?;
    // The node runs a single overlay subnet / TUN. Read the operative subnet
    // (cache of the active network's signed GroupBlob value) so the identity and
    // TUN are built in the right range at bootstrap, before any network is up.
    let node_subnet = config::node_subnet();
    let identity = IrohIdentityProvider::new(public_key, collision_index, node_subnet);
    let my_ip = identity.local_ip();

    // --- iroh endpoint (one ALPN per saved network + the blobs ALPN) ---
    let app_config = config::load()?;
    // Point the pkarr client at the configured discovery-DNS server (if any)
    // before any record publish/resolve happens.
    dht::set_discovery_override(&app_config.discovery_dns);
    let alpns = initial_alpns(&app_config);
    let use_tor = app_config
        .networks
        .iter()
        .any(|net| net.transport.as_ref().is_some_and(|t| t.is_tor()));
    let ep = transport::create_endpoint_with_alpns(
        key.clone(),
        alpns,
        use_tor,
        &app_config.relay,
        &app_config.discovery_dns,
    )
    .await?;

    // --- Content-addressed blob store (membership/file transfer) ---
    let blobs_dir = config::config_dir()?.join("blobs");
    std::fs::create_dir_all(&blobs_dir)?;
    let blob_store = FsStore::load(&blobs_dir)
        .await
        .context("failed to open blob store")?;
    let blobs_proto = BlobsProtocol::new(&blob_store, None);

    // --- Packet interface: deferred, per-network ---
    // No OS TUN device or forwarding loop is created here. MULTISEG-003 moved
    // TUN creation off the daemon entirely: each network's own `peers`/
    // `tun_name`/`tun_tx`/`tun_tasks` live on its `NetworkHandle`, created
    // inside that network's own create/join/restore path (desktop) or attached
    // there by the embedder via `attach_tun(network, ...)` (mobile).
    let active = Arc::new(AtomicBool::new(false));
    // --- Protocol router + the shared MeshManager ---
    let protocol_router = Arc::new(ProtocolRouter::new(blobs_proto));
    // Promotion channel: a co-coordinator's control reader signals the main
    // daemon loop to swap in the coordinator accept handler on `AdminGrant`.
    let (promote_tx, promote_rx) = mpsc::channel::<String>(16);
    // Self-removal channel (CONVERGE-003): mirrors the promotion channel above.
    let (left_tx, left_rx) = mpsc::channel::<String>(16);
    let daemon = Arc::new(MeshManager {
        endpoint: ep,
        identity,
        stats: stats.clone(),
        networks: Arc::new(DashMap::new()),
        shutdown_token: token.clone(),
        blob_store,
        protocol_router: protocol_router.clone(),
        promote_rx: std::sync::Mutex::new(Some(promote_rx)),
        pruned_peers: Arc::new(DashSet::new()),
        active: active.clone(),

        promote_tx,
        left_rx: std::sync::Mutex::new(Some(left_rx)),
        left_tx,
    });

    // --- Accept loop (ALPN dispatch) ---
    protocol_router.spawn_accept_loop(daemon.endpoint.clone(), token.clone());

    tracing::info!(ip = %my_ip, id = %daemon.endpoint.id().fmt_short(), "daemon started");
    Ok(daemon)
}

/// Bind the IPC Unix socket and serve client requests until the daemon-wide
/// `token` is cancelled. On shutdown, put the VPN on standby (revert DNS, drop
/// connections, bring the TUN down) and remove the socket file. Each request is
/// handled on its own task so a slow client can't block the accept loop.
async fn serve_ipc(
    daemon: &Arc<MeshManager>,
    mut promote_rx: mpsc::Receiver<String>,
    mut left_rx: mpsc::Receiver<String>,
    token: CancellationToken,
) -> Result<()> {
    let socket_path = ipc::socket_path();
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path).context("failed to bind IPC socket")?;
    set_socket_permissions(&socket_path);
    tracing::info!(path = %socket_path.display(), "IPC socket listening");

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("daemon shutting down");
                daemon.deactivate(None).await;
                let _ = std::fs::remove_file(&socket_path);
                return Ok(());
            }
            // A co-coordinator just persisted an `AdminGrant` key: swap its
            // accept handler to coordinator so it can admit fresh joiners.
            // Idempotent and quick (a synchronous handler swap), so running it
            // inline in the loop is fine.
            Some(net) = promote_rx.recv() => {
                daemon.promote_to_coordinator(&net).await;
            }
            // A network's poller or reconverge worker detected that the local
            // node is no longer in the authoritative roster (CONVERGE-003):
            // leave it locally instead of redialing forever.
            Some(net) = left_rx.recv() => {
                daemon.handle_removed_from_network(&net).await;
            }
            result = listener.accept() => match result {
                Ok((stream, _)) => {
                    let daemon = daemon.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_ipc_client(stream, &daemon).await {
                            tracing::debug!(error = %e, "IPC client error");
                        }
                    });
                }
                Err(e) => tracing::warn!(error = %e, "IPC accept error"),
            }
        }
    }
}

/// Make the IPC socket connectable by any local user. Authority is not granted
/// by reaching the socket — every mutating request is authorized per-connection
/// in `check_authorized` via `SO_PEERCRED` (root or the configured operator
/// UID), Tailscale's model — so the file mode only has to permit the connect().
fn set_socket_permissions(path: &std::path::Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    if let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) {
        unsafe { libc::chmod(c_path.as_ptr(), 0o666) };
        tracing::info!("IPC socket mode 0666 (per-request authorization via peer creds)");
    }
}

async fn handle_ipc_client(stream: UnixStream, daemon: &Arc<MeshManager>) -> Result<()> {
    let peer_cred = stream.peer_cred().ok().map(|c| (c.uid(), c.gid()));
    let mut framed = ipc::framed(stream);
    let req = ipc::recv(&mut framed).await?;
    let resp = daemon.handle_request(req, peer_cred).await;
    ipc::send(&mut framed, resp).await?;
    Ok(())
}

