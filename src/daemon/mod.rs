//! The tetron daemon: a long-lived, root-owned process that holds the iroh
//! [`Endpoint`], the TUN device, the [`PeerTable`], and the [`ProtocolRouter`],
//! and serves the unprivileged CLI over a Unix-socket IPC channel.
//!
//! # Two lifecycles
//!
//! The daemon deliberately separates two concepts that are easy to conflate:
//!
//! - **Process / infrastructure lifecycle** — the iroh endpoint, IPC socket,
//!   accept loop, blob store, DNS resolver, metrics server, and the TUN *file
//!   descriptor*. These are built once in [`run_daemon`] and live for the whole
//!   process. They are torn down only by the daemon-wide `shutdown_token`
//!   (real shutdown / `IpcMessage::Shutdown`).
//! - **Active VPN state** — the TUN link being *up*, system DNS being
//!   configured, and the saved networks being connected. This is toggled at
//!   runtime by [`MeshManager::activate`] / [`MeshManager::deactivate`], driven
//!   by the `Resume` / `Standby` IPC commands. Each network tracks its own
//!   activation on its own `NetworkHandle.active` (STANDBY-PER-NETWORK), so
//!   `--network` can toggle one network's data plane without touching the
//!   others; [`MeshManager::active`] now only seeds a brand-new network's
//!   initial state and is what an *unscoped* `Resume`/`Standby` sets.
//!
//! This mirrors Tailscale's split between the always-running `tailscaled`
//! daemon and the `tailscale up` / `tailscale down` client toggles: `standby`
//! puts the daemon on *standby* (VPN state torn down) without killing the
//! process, so the next `resume` is a cheap, unprivileged IPC call rather than
//! a root service restart.
//!
//! # Cancellation tokens
//!
//! There are two tiers, and the distinction is what makes standby work:
//!
//! - `shutdown_token` (the token passed into [`run_daemon`]) gates all the
//!   always-on infrastructure. Cancelling it stops the **process**. `Standby`
//!   never touches it — otherwise the IPC accept loop would die and there would
//!   be nothing left to receive the next `Resume`.
//! - Each active network owns a `shutdown_token.child_token()` stored on its
//!   [`NetworkHandle`]. `deactivate` cancels these per-network children to stop
//!   that network's background tasks. Because cancellation is one-shot, every
//!   `activate` mints *fresh* child tokens, so `resume → standby → resume`
//!   cycles work.

use bytes::Bytes;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::Ipv4Addr;

use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dashmap::{DashMap, DashSet};

use anyhow::{Context, Result};
use iroh::address_lookup::PkarrRelayClient;
use iroh::endpoint::{Connection, Endpoint, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{EndpointId, SecretKey};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobsProtocol, HashAndFormat};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config;
use crate::control::{self, ControlMsg};
use crate::dht;
use crate::forward;
use crate::identity;
use crate::ipc::{self, IpcMessage, NetworkRole, NetworkStatus, PeerStatus};
use crate::membership::{
    ApprovedEntry, ApprovedList, GroupMode, IdentityProvider, IrohIdentityProvider, Member,
    MemberList, Subnet, canonical_group_bytes, derive_ipv6, group_blob_hash, verify_group_blob,
};
use crate::network_name;
use crate::peers::PeerTable;
use crate::stats::ForwardMetrics;
use crate::transport;
// The desktop TUN device doesn't exist on Android, where the packet interface
// is a `VpnService` fd supplied from Kotlin.
#[cfg(not(target_os = "android"))]
use crate::tun;

// `MeshManager`'s IPC operations are split by domain into the `mesh/` submodule;
// see `mesh/mod.rs`. Each holds an additional `impl MeshManager` block. Nested a
// level down so the module names can be the clean domain names without colliding
// with the `use crate::{config, …}` aliases above.
mod mesh;
// The mesh core's join handshake and background-task/reconvergence helpers were
// moved into `mesh/{join,background}.rs`; re-export them at the daemon level so
// `mod.rs` and the other `mesh/` submodules (via `use super::super::*`) call them
// by bare name, as before the split.
pub(crate) use mesh::*;
// `run_daemon` (the `tetron daemon` entry point) stays public for the binary.
pub use mesh::run_daemon;
// `build_headless` is the embedder construction entry point.
pub use mesh::build_headless;

/// Legacy name for [`MeshManager`], kept for compile compatibility with code
/// (including this crate's own tests) written against the pre-refactor
/// `DaemonState` name.
pub type DaemonState = MeshManager;

const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Shared handles for one network's accept handlers and background tasks.
/// Every field is a cheap `Clone` — an `Arc`-backed handle, a channel sender,
/// or a small wrapper — so the whole bundle is cloned by value instead of
/// threaded as a dozen separate arguments/struct fields. `identity`/`stats`/
/// `blob_store`/`pruned_peers` are genuinely daemon-wide; `peers`/`tun_tx` are
/// that network's own (MULTISEG-002 moved these off the daemon onto each
/// [`NetworkHandle`], so a `MeshCtx` is now built per network — either freshly
/// (before the handle exists, at create/join/restore) or via
/// [`MeshManager::mesh_ctx_for`] (after it exists, at promotion).
#[derive(Clone)]
pub(crate) struct MeshCtx {
    identity: IrohIdentityProvider,
    /// This network's own public key (IPV6-001) — every peer IPv6 address
    /// derived through this context is scoped to this network alone.
    network_key: EndpointId,
    peers: PeerTable,
    tun_tx: Arc<arc_swap::ArcSwap<mpsc::Sender<Bytes>>>,
    stats: Arc<ForwardMetrics>,
    blob_store: FsStore,
    /// Peers removed from a network's roster (via `tetron kick` or a stale-entry
    /// prune during reconverge), keyed by `(network, transport id)`. A member
    /// closes such a peer's connection but can't see its own close code, so its
    /// reconnect loop would re-dial the removed peer (which still lists it) and
    /// re-form the link. The reconnect loop consumes an entry here to skip that
    /// one reconnect. Populated in [`reconverge_and_apply`] and the kick handler.
    pruned_peers: Arc<DashSet<(String, EndpointId)>>,
}

impl MeshCtx {
    /// Build the per-peer data-plane bundle for `forward::spawn_peer_reader`,
    /// combining this context's shared handles with the caller's per-connection
    /// `disconnect_tx`/`token`.
    fn forward_ctx(
        &self,
        disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
        token: CancellationToken,
    ) -> forward::ForwardCtx {
        forward::ForwardCtx {
            tun_tx: self.tun_tx.clone(),
            disconnect_tx,
            token,
            stats: self.stats.clone(),
        }
    }
}

/// Project a roster's `Member`s into the persistable `config::MemberEntry` form
/// (drops the runtime-only `user_identity`/`device_cert`/`collision_index`).
pub(crate) fn to_member_entries<'a>(
    members: impl IntoIterator<Item = &'a Member>,
) -> Vec<config::MemberEntry> {
    members
        .into_iter()
        .map(|m| config::MemberEntry {
            identity: m.identity,
            ip: m.ip,
            is_coordinator: m.is_coordinator,
            hostname: m.hostname.clone(),
        })
        .collect()
}

/// Project approved entries into the persistable `config::ApprovedConfigEntry`.
pub(crate) fn to_approved_entries<'a>(
    approved: impl IntoIterator<Item = &'a ApprovedEntry>,
) -> Vec<config::ApprovedConfigEntry> {
    approved
        .into_iter()
        .map(|a| config::ApprovedConfigEntry {
            identity: a.identity,
            ip: a.ip,
            hostname: a.hostname.clone(),
        })
        .collect()
}

#[derive(Clone)]
struct GroupSnapshot {
    hash: blake3::Hash,
    msgpack_bytes: Vec<u8>,
}

/// A per-network state cell shared (read-mostly) across the accept handlers,
/// publisher, poller, and cleanup tasks for that network.
pub(crate) type SharedNetworkState = Arc<RwLock<NetworkState>>;

pub(crate) struct NetworkState {
    /// Monotonic blob version (CONVERGE-005). Bumped by
    /// [`NetworkState::bump_generation_and_refresh`] on every local content
    /// mutation; set directly (never bumped) when adopting a freshly fetched,
    /// verified blob during reconverge, so it always reflects that blob's own
    /// generation rather than this node's local mutation count.
    generation: u64,
    members: MemberList,
    approved: ApprovedList,
    snapshot: Option<GroupSnapshot>,
    network_secret_key: Option<SecretKey>,
    network_public_key: EndpointId,
    network_name: Option<String>,
    /// Access mode, carried through from config for wire/config-format
    /// compatibility. Admission is invite-only regardless of this value
    /// (`LIVE-001`) and tetron never creates an `Open` network
    /// (`MINIMAL-013`), so nothing in this daemon consults it anymore --
    /// same dead-weight class as `membership::OpenPolicy`/`policy_for_mode`.
    #[allow(dead_code)]
    mode: GroupMode,
    /// Reusable join keys carried in the signed blob (keyed by hex
    /// `blake3(secret)`). On a network-key holder this is what it publishes and
    /// validates redemptions against; on a plain member it is what it last
    /// received. Reloaded from the verified blob on every reconverge so any admin
    /// can admit and revocation propagates.
    reusable_keys: BTreeMap<String, crate::membership::ReusableKey>,
    /// Single-use invite entries carried in the signed blob (keyed by hex
    /// `blake3(secret)`). On a network-key holder this is what it publishes and
    /// validates redemptions against. Entries are removed on successful
    /// redemption (the blob is republished without the used invite).
    invites: BTreeMap<String, crate::membership::InviteEntry>,
    /// Pending nuke proposals carried in the signed blob (NUKE-CONSENSUS),
    /// keyed by proposer identity string. Synced from every freshly verified
    /// blob purely for visibility (`tetron status` surfaces pending proposals);
    /// the only code path that ever acts on this is the synchronous
    /// `MeshManager::nuke_network` command handler, not reconverge.
    pub(crate) nuke_proposals: BTreeMap<String, u64>,
    /// The network's resolved overlay subnet (from the signed `GroupBlob`, or the
    /// default). Used to derive/validate member IPs and to publish the subnet
    /// field back into the blob.
    subnet: Subnet,
}

impl NetworkState {
    /// Snapshot the current member roster as an owned `Vec` (the members map is
    /// the single source of truth; callers take a copy to release the lock).
    fn roster(&self) -> Vec<Member> {
        self.members.all().into_iter().cloned().collect()
    }

    /// Snapshot the current approved-but-not-yet-joined entries as an owned `Vec`.
    fn approved_snapshot(&self) -> Vec<ApprovedEntry> {
        self.approved.all().into_iter().cloned().collect()
    }

    /// Hostnames currently claimed by other members (excluding `except`), used to
    /// resolve a rename/join collision against the roster.
    fn taken_hostnames(&self, except: EndpointId) -> Vec<String> {
        self.members
            .all()
            .iter()
            .filter(|m| m.identity != except)
            .filter_map(|m| m.hostname.clone())
            .collect()
    }

    /// Recompute the blob snapshot (hash + bytes) from the current fields as-is
    /// — does not touch `generation`. Use this when adopting a freshly fetched,
    /// verified blob (set `generation` from it first) or from
    /// `bump_generation_and_refresh` for a local mutation.
    fn refresh_snapshot(&mut self) {
        let bytes = canonical_group_bytes(
            self.generation,
            &self.members,
            &self.approved,
            self.network_name.as_deref(),
            &self.reusable_keys,
            self.blob_subnet(),
            &self.invites,
            &self.nuke_proposals,
        );
        let hash = blake3::hash(&bytes);
        self.snapshot = Some(GroupSnapshot {
            hash,
            msgpack_bytes: bytes,
        });
    }

    /// Recompute the blob snapshot after a genuine local content mutation
    /// (admit, kick, invite create/revoke, admin grant, ...): increments
    /// `generation` first so every publisher/poller can tell this is newer than
    /// whatever it last saw, regardless of DHT write order (CONVERGE-005).
    fn bump_generation_and_refresh(&mut self) {
        self.generation += 1;
        self.refresh_snapshot();
    }

    /// The subnet as it should appear in the published blob: always the
    /// network's actual, currently-resolved subnet (SUBNET-DRIFT-001).
    /// Previously omitted (`None`) whenever it matched the compiled default,
    /// to keep default-range networks byte-identical -- but `self.subnet`
    /// here is only ever correct if it was resolved correctly in the first
    /// place, and a restart that mis-resolves it (falling back to the
    /// compiled default when nothing else is known) would then have this
    /// method collapse that wrong value back to `None` too, republishing
    /// the ambiguity into the signed blob and spreading the corruption to
    /// every peer that fetches it. Explicit always, so a correctly-resolved
    /// subnet is never lost to a later mis-resolution's `None`.
    fn blob_subnet(&self) -> Option<Subnet> {
        Some(self.subnet)
    }
}

/// Runtime state for one active network. Created when a network is joined,
/// created, or reconnected; dropped (after `cancel`ling and awaiting `tasks`)
/// when the network is left or the VPN is put on standby. The persisted config
/// (in `networks.toml`) outlives this handle — standby tears down the handle
/// but keeps the config so `activate` can rebuild it.
///
/// **MULTISEG-002:** each network owns its own data-plane bundle (`peers`,
/// `tun_name`, `tun_tx`, `tun_tasks`) instead of sharing one daemon-wide copy
/// — see that requirement in `spec/design_spec.py` for the full rationale.
#[allow(dead_code)]
pub struct NetworkHandle {
    name: String,
    network_key: EndpointId,
    role: NetworkRole,
    my_ip: Ipv4Addr,
    state: SharedNetworkState,
    /// DHT republish trigger; `Some` only on the coordinator (the sole publisher).
    /// Lets admission/kick re-publish the group blob.
    dht_notify: Option<Arc<Notify>>,
    /// Child of the daemon `shutdown_token`. Cancelling it stops this network's
    /// background tasks (reconnect loop, group poller, publisher, peer readers)
    /// without affecting the rest of the daemon.
    cancel: CancellationToken,
    /// Background tasks owned by this network, awaited on teardown.
    tasks: Vec<JoinHandle<()>>,
    /// Disconnect channel for this network's accept handlers, kept so a member
    /// promoted to coordinator (via `AdminGrant`) can re-register a
    /// [`CoordinatorAcceptState`] on the live channel without rebuilding it.
    disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
    /// This network's own routing table (MULTISEG-002). Populated as soon as
    /// the handle exists, independent of whether a TUN is attached yet — a
    /// headless embedder can accept/join before calling `attach_tun`, same as
    /// the pre-multi-segment daemon-wide table was usable before boot's single
    /// `attach_tun` call.
    peers: PeerTable,
    /// Name of this network's OS TUN device (desktop), or a placeholder until
    /// `attach_tun` runs. See `MeshManager.tun_name`'s pre-MULTISEG-002 doc
    /// comment for why this is interior-mutable.
    tun_name: std::sync::Mutex<String>,
    /// Sender half of this network's TUN write channel, in a swappable cell.
    /// See `MeshManager::attach_tun`'s doc comment (pre-MULTISEG-002) for the
    /// fresh-channel-per-attach mechanism this enables; now per-network rather
    /// than daemon-wide.
    tun_tx: Arc<arc_swap::ArcSwap<mpsc::Sender<Bytes>>>,
    /// Handles for this network's packet-forwarding tasks, spawned by
    /// [`MeshManager::attach_tun`].
    tun_tasks: std::sync::Mutex<Option<TunTasks>>,
    /// This network's own data-plane activation state (STANDBY-PER-NETWORK).
    /// `true` once its TUN link/routes are up; `false` in standby (still
    /// connected to peers, no packets flow). Gates `spawn_tun_writer`'s
    /// actual write-to-TUN — the per-network counterpart of the role
    /// `MeshManager.active` played daemon-wide before `tetron resume`/
    /// `standby` gained `--network` scoping. Starts `false` at construction;
    /// brought up by `create_and_attach_network_tun` if the daemon's default
    /// state (`MeshManager.active`) is already active at attach time, or later by
    /// an explicit `activate()`.
    active: Arc<AtomicBool>,
}

/// Shared, always-on daemon state. Cloned (via `Arc`) into every IPC handler
/// and background task. Holds both the infrastructure that lives for the whole
/// process and the handles for the currently-active networks. See the
/// module-level docs for the two-lifecycle model.
/// Handles for the packet-forwarding tasks a [`MeshManager::attach_tun`] call
/// spawns (the TUN writer and the `run_mesh` reader loop), plus a dedicated
/// cancellation token so the data plane can be stopped independently of a full
/// daemon shutdown (used by [`MeshManager::detach_tun`]).
struct TunTasks {
    /// Cancels the `run_mesh` reader loop without touching `shutdown_token`.
    cancel: CancellationToken,
    /// The TUN writer task (`spawn_tun_writer`).
    writer: JoinHandle<()>,
    /// The `run_mesh` reader loop task.
    mesh: JoinHandle<()>,
}

pub struct MeshManager {
    endpoint: Endpoint,
    identity: IrohIdentityProvider,
    stats: Arc<ForwardMetrics>,
    networks: Arc<DashMap<String, NetworkHandle>>,
    shutdown_token: CancellationToken,
    blob_store: FsStore,
    protocol_router: Arc<ProtocolRouter>,
    /// Promotion-channel receiver drained by [`serve_ipc`]. Stored here so the
    /// headless builder can construct the daemon and hand the receiver back to
    /// [`run_daemon`] afterwards.
    promote_rx: std::sync::Mutex<Option<mpsc::Receiver<String>>>,
    /// Peers removed from a roster whose reconnect should be suppressed once.
    /// Shared into [`MeshCtx::pruned_peers`]; see that field for the mechanism.
    pruned_peers: Arc<DashSet<(String, EndpointId)>>,
    /// Default data-plane activation state for a network at the moment it is
    /// first created/joined/restored (`create_and_attach_network_tun`
    /// consults this to decide whether to bring a brand-new `NetworkHandle`
    /// straight up). Toggled by an *unscoped* `Resume`/`Standby` IPC command
    /// (no `--network`). The actual per-packet gate is each network's own
    /// `NetworkHandle.active` (STANDBY-PER-NETWORK) — this field no longer
    /// gates forwarding directly, it only seeds new networks' initial state
    /// and is what an unscoped `tetron resume`/`standby` sets across the board.
    active: Arc<AtomicBool>,
    /// Promotion signal: a co-coordinator's per-peer control reader sends the
    /// network name here after persisting an `AdminGrant` key, and the main
    /// daemon loop ([`serve_ipc`]) drains it into
    /// [`MeshManager::promote_to_coordinator`]. The reader holds only field
    /// clones (not the full `MeshManager`), so it can't promote itself — hence
    /// the channel hand-off to the loop that does hold the `Arc<MeshManager>`.
    promote_tx: mpsc::Sender<String>,
    /// Self-removal-channel receiver drained by [`serve_ipc`]. Mirrors
    /// `promote_rx`: stored here so the headless builder can construct the
    /// daemon and hand the receiver back to [`run_daemon`] afterwards.
    left_rx: std::sync::Mutex<Option<mpsc::Receiver<String>>>,
    /// Self-removal signal (CONVERGE-003): a network's group poller or
    /// debounced reconverge worker sends the network name here on detecting
    /// that the local node is no longer in the authoritative roster, and the
    /// main daemon loop ([`serve_ipc`]) drains it into
    /// [`MeshManager::handle_removed_from_network`]. Same rationale as
    /// `promote_tx`: those background tasks hold only field clones, not the
    /// full `MeshManager`, so they hand off to the loop that does.
    left_tx: mpsc::Sender<String>,
}

/// Map key-holding status to a [`NetworkRole`].
///
/// A node that holds the per-network secret key (original coordinator or one
/// promoted via `tetron admin add`) runs as `Coordinator`; all other nodes run
/// as `Member`.
fn role_for_key_holder(holds_network_key: bool) -> NetworkRole {
    if holds_network_key {
        NetworkRole::Coordinator
    } else {
        NetworkRole::Member
    }
}

/// Whether an `AdminGrant`'s key is genuinely this network's key.
///
/// Self-authenticating admission of the granted key: we adopt it only if its
/// public half equals the network pubkey. An attacker who does not already hold
/// the real secret cannot forge a key that passes, so a forged `AdminGrant`
/// from a non-coordinator member is rejected without any roster lookup (and so
/// without depending on reconverge timing for the granter's `is_coordinator`
/// flag, which a sender-identity check would).
fn admin_grant_key_valid(secret_key: [u8; 32], net_pubkey: EndpointId) -> bool {
    SecretKey::from(secret_key).public() == net_pubkey
}

/// Whether a network in `current` role should be (re-)registered as coordinator.
///
/// A member promoted via `AdminGrant` must swap to the coordinator accept
/// handler; a network already running as coordinator is a no-op.
fn should_promote(current: NetworkRole) -> bool {
    !current.is_coordinator()
}

impl MeshManager {
    /// Gracefully take the whole node offline: cancel the daemon-wide shutdown
    /// token (stopping every network run loop, the accept loop, and the
    /// data-plane forward tasks) and then close the iroh endpoint so all QUIC
    /// connections terminate cleanly and peers see us drop immediately, rather
    /// than lingering until an idle timeout. Awaiting the close matters for
    /// embedders (mobile) that rebuild a fresh daemon on re-enable: without it
    /// the old endpoint's connections outlive `stop`, so a coordinator keeps the
    /// stale session while the rebuilt endpoint (same node key) comes up and the
    /// device shows offline until the race clears. Mirrors the shutdown tail of
    /// `run_daemon`. After this the `MeshManager` is spent; build a new one to
    /// come back online.
    pub async fn shutdown_and_close(&self) {
        self.shutdown_token.cancel();
        self.endpoint.close().await;
    }

    /// Bundle an existing network's own data-plane handles into a [`MeshCtx`]
    /// (MULTISEG-002). Used only where the [`NetworkHandle`] already exists in
    /// `self.networks` — [`promote_to_coordinator`], the only post-creation
    /// caller. Every other `MeshCtx` construction site builds one directly
    /// from a freshly created `peers`/`tun_tx` pair, before the handle exists
    /// (see `create_network_inner`/`join_network_inner`/
    /// `restore_coordinator_network`).
    pub(crate) fn mesh_ctx_for(&self, network: &str) -> Option<MeshCtx> {
        let handle = self.networks.get(network)?;
        Some(MeshCtx {
            identity: self.identity.clone(),
            network_key: handle.network_key,
            peers: handle.peers.clone(),
            tun_tx: handle.tun_tx.clone(),
            stats: self.stats.clone(),
            blob_store: self.blob_store.clone(),
            pruned_peers: self.pruned_peers.clone(),
        })
    }

    /// Refresh the peer address cache (CACHE-001) from every network's own
    /// routing table. MULTISEG-002 moved `PeerTable` off the daemon onto each
    /// `NetworkHandle`, so this now iterates `self.networks` instead of
    /// reading one daemon-wide table.
    pub(crate) fn refresh_peer_cache(&self) {
        for entry in self.networks.iter() {
            crate::peercache::refresh_from_peers(&entry.value().peers);
        }
    }

    /// Build a fresh, empty per-network data-plane bundle (MULTISEG-002): a new
    /// `PeerTable` and a placeholder `tun_tx` cell (its receiver dropped
    /// immediately — no real channel exists until `attach_tun` creates one).
    /// Used by `create_network_inner`/`join_network_inner`/
    /// `restore_coordinator_network` to build this network's own `MeshCtx`
    /// before its `NetworkHandle` exists in `self.networks` — mirrors the
    /// placeholder `build_daemon` used to set up once, daemon-wide.
    pub(crate) fn new_network_data_plane(
        &self,
    ) -> (PeerTable, Arc<arc_swap::ArcSwap<mpsc::Sender<Bytes>>>) {
        let peers = PeerTable::new();
        let (placeholder_tx, _placeholder_rx) = mpsc::channel::<Bytes>(1);
        let tun_tx = Arc::new(arc_swap::ArcSwap::from_pointee(placeholder_tx));
        (peers, tun_tx)
    }

    /// Create `network`'s own OS TUN device and attach it (MULTISEG-003) —
    /// desktop-only, the per-network counterpart of what `run_daemon` used to
    /// do once, daemon-wide, at boot. Non-fatal on failure (logged): the
    /// network still exists with its control plane connected, just without a
    /// data plane, matching `activate()`'s existing "warn, don't fail" pattern
    /// for TUN problems.
    ///
    /// If the VPN's default state is active (`self.active` — the
    /// daemon-wide flag an unscoped `activate()`/`deactivate()` sets, used
    /// to seed a brand-new network's initial state per STANDBY-PER-NETWORK),
    /// also brings this network's own link up and installs its routes
    /// immediately (setting its own `NetworkHandle.active`), rather than
    /// waiting for a future `activate()` call — needed for `tetron join`/
    /// `create` while already up, and for a restore whose attach lands after
    /// boot's one `activate(None, None)` call already ran. This does not
    /// fully close the boot-time race: `connect_all_networks` fires each
    /// restore as a detached task, so in principle one could reach this
    /// check a moment before `activate()`'s own `self.active` store — in
    /// practice the DHT round-trip every restore does first makes that
    /// complete long before any restore reaches here, but it is not a hard
    /// guarantee. Flagged, not fixed here: closing it fully would mean
    /// awaiting every restore before `run_daemon` proceeds, undoing the
    /// fire-and-forget design `connect_all_networks` deliberately uses so
    /// one dead network can't delay the others.
    #[cfg(not(target_os = "android"))]
    pub(crate) async fn create_and_attach_network_tun(
        self: &Arc<Self>,
        network: &str,
        my_ip: Ipv4Addr,
        subnet: crate::membership::Subnet,
    ) {
        let Some(network_key) = self.networks.get(network).map(|h| h.network_key) else {
            tracing::warn!(network, "network handle missing at TUN attach time");
            return;
        };
        let my_ipv6 = derive_ipv6(&self.identity.local_identity(), &network_key);
        match tun::create(my_ip, my_ipv6, subnet).await {
            Ok((reader, writer, tun_name)) => {
                if let Some(handle) = self.networks.get(network) {
                    *handle.tun_name.lock().unwrap() = tun_name.clone();
                }
                self.attach_tun(network, reader, writer).await;
                if self.active.load(Ordering::SeqCst) {
                    if let Some(handle) = self.networks.get(network) {
                        handle.active.store(true, Ordering::SeqCst);
                    }
                    if let Err(e) = tun::set_link_up(&tun_name) {
                        tracing::warn!(network, error = %e, "failed to bring newly attached TUN up");
                    }
                    let network_prefix = crate::membership::ipv6_network_prefix(&network_key);
                    if let Err(e) = tun::route_peer_range(
                        &tun_name,
                        subnet,
                        network_prefix,
                        crate::membership::IPV6_NETWORK_PREFIX_LEN,
                    )
                    .await
                    {
                        tracing::warn!(network, error = %e, "failed to route peer range into newly attached TUN");
                    }
                    if let Err(e) = tun::route_self_loopback(my_ip, my_ipv6).await {
                        tracing::warn!(network, error = %e, "failed to install loopback self-route");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(network, error = %e, "failed to create TUN device for network");
            }
        }
    }

    pub(crate) async fn refresh_alpns(&self) {
        let alpns = self.protocol_router.alpns();
        let alpn_strs: Vec<String> = alpns
            .iter()
            .map(|a| String::from_utf8_lossy(a).to_string())
            .collect();
        tracing::info!(alpns = ?alpn_strs, "refreshing ALPNs");
        self.endpoint.set_alpns(alpns);
    }

    /// Attach a packet interface to one network and start that network's data
    /// plane forwarding tasks: the TUN writer (`spawn_tun_writer`) and the mesh
    /// forwarding loop (`run_mesh`, reading `reader` and using that network's own
    /// `peers`/the daemon's shared `stats`).
    ///
    /// **MULTISEG-003:** this is a per-network operation, not a per-daemon one —
    /// `network` must already have a [`NetworkHandle`] in `self.networks` (a
    /// no-op, logged, if it doesn't). Desktop calls this once per network, right
    /// after that network's own `tun::create()` succeeds (in
    /// `create_network_inner`/`finalize_join`/`restore_coordinator_network`),
    /// mirroring the pre-MULTISEG-003 single daemon-wide call in `run_daemon`.
    /// An embedder (mobile) calls it once per network with that network's own
    /// packet interface.
    ///
    /// A fresh `tun_tx`/`tun_rx` channel is created on every call: the new
    /// receiver feeds the writer, and the new sender is stored in the network's
    /// `tun_tx` cell so incoming send-sites (that network's peer readers) resolve
    /// the live writer via `tun_tx.load()`. This makes re-attach work: after a
    /// [`detach_tun`] the next `attach_tun` for the same network swaps in a new
    /// sender and a new writer, so forwarding resumes. This is the exact VPN
    /// off/on toggle path on Android.
    ///
    /// The forwarding loop runs under a child of `shutdown_token`, and its
    /// handles are stored on the `NetworkHandle` so a later `down()`/detach can
    /// stop that network's data plane without tearing down the whole daemon.
    /// Desktop attaches each network exactly once, so its cell is never swapped.
    pub async fn attach_tun<R: crate::tun::TunRead, W: crate::tun::TunWrite>(
        self: &Arc<Self>,
        network: &str,
        reader: R,
        writer: W,
    ) {
        let Some(handle) = self.networks.get(network) else {
            tracing::warn!(network, "attach_tun: network not active, ignoring");
            return;
        };

        // Fresh channel per attach. The previous writer (if any) was torn down by
        // `detach_tun`, which dropped the old receiver; swapping in the new sender
        // reconnects every incoming send-site to this writer.
        let (new_tx, new_rx) = mpsc::channel::<Bytes>(256);
        handle.tun_tx.store(Arc::new(new_tx.clone()));

        // A dedicated child token so the data plane can be stopped independently
        // of a full daemon shutdown; it still cancels when `shutdown_token` does.
        let cancel = self.shutdown_token.child_token();
        let writer_handle = forward::spawn_tun_writer(writer, new_rx, handle.active.clone());
        let mesh_handle = {
            let peers = handle.peers.clone();
            let cancel = cancel.clone();
            let stats = self.stats.clone();
            tokio::spawn(async move {
                if let Err(e) = forward::run_mesh(reader, peers, cancel, stats).await {
                    tracing::warn!(error = %e, "mesh forwarding loop exited with error");
                }
            })
        };

        // Self-healing: if `attach_tun` is called twice for the same network
        // without an intervening `detach_tun`, stop the previous data plane
        // before installing the new one. `JoinHandle::drop` detaches rather than
        // aborts, so without this the old writer + `run_mesh` loop would keep
        // running forever on the old fds (a leak of two live mesh loops). On the
        // normal detach->attach path `detach_tun` already took the old tasks, so
        // `replace` returns `None`.
        let new_tasks = TunTasks {
            cancel,
            writer: writer_handle,
            mesh: mesh_handle,
        };
        let old = handle.tun_tasks.lock().unwrap().replace(new_tasks);
        if let Some(old) = old {
            old.cancel.cancel();
            old.writer.abort();
            old.mesh.abort();
        }
    }

    /// Part of the embedding API: stop `network`'s packet-forwarding data plane
    /// started by [`attach_tun`] (the TUN writer and
    /// the `run_mesh` reader loop) WITHOUT tearing down the control plane. That
    /// network's connections stay live, so the node remains reachable to its
    /// peers and keeps receiving roster/blob updates; only local packet
    /// forwarding over the attached interface stops. Cancelling the loop's
    /// child token and aborting the tasks drops the reader/writer, closing the
    /// underlying fds. Idempotent: a no-op if no interface is attached, or if
    /// `network` isn't active.
    ///
    /// **MULTISEG-003:** unlike [`activate`]/[`deactivate`] (the global,
    /// all-networks `Resume`/`Standby` data-plane toggle, which flips the shared
    /// `active` flag every forwarding task already reads), this only tears
    /// down `network`'s own forwarding tasks — it deliberately does not touch
    /// `active`, since detaching one network's interface must not silently
    /// mark every other network's data plane inactive too.
    pub fn detach_tun(&self, network: &str) {
        let Some(handle) = self.networks.get(network) else {
            return;
        };
        if let Some(tasks) = handle.tun_tasks.lock().unwrap().take() {
            tasks.cancel.cancel();
            tasks.writer.abort();
            tasks.mesh.abort();
        }
    }

    /// Register a [`CoordinatorAcceptState`] handler for `network` and update
    /// the network's role in `self.networks` to [`NetworkRole::Coordinator`].
    ///
    /// Calling this at create, restore, and admin-promotion sites keeps the
    /// coordinator-registration logic in one place. The method is synchronous
    /// (no `.await`) because `protocol_router.register` is a plain HashMap
    /// swap; the caller is responsible for spawning the `disconnect_rx` cleanup
    /// task **before** calling this so the channel is live when the first
    /// incoming connection arrives.
    /// **MULTISEG-002:** `ctx` is this network's own [`MeshCtx`] (built from its
    /// `peers`/`tun_tx`, not a daemon-wide one) — the caller supplies it because
    /// at create/restore time the `NetworkHandle` doesn't exist in
    /// `self.networks` yet (so [`mesh_ctx_for`] can't look it up); at
    /// promote-to-coordinator time it does, and the caller uses `mesh_ctx_for`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_coordinator_handler(
        &self,
        network: &str,
        state: SharedNetworkState,
        dht_notify: Option<Arc<Notify>>,
        network_key: EndpointId,
        disconnect_tx: mpsc::Sender<forward::DisconnectEvent>,
        cancel: CancellationToken,
        ctx: MeshCtx,
    ) {
        self.protocol_router.register(
            transport::network_alpn(&network_key),
            AcceptHandler::Coordinator(Arc::new(CoordinatorAcceptState {
                ctx,
                network_name: network.to_string(),
                state,
                disconnect_tx,
                token: cancel,
                dht_notify,
                pending_pongs: self.protocol_router.pending_pongs.clone(),
            })),
        );
        // Flip the stored role so `tetron status` reports Coordinator immediately.
        if let Some(mut handle) = self.networks.get_mut(network) {
            handle.role = NetworkRole::Coordinator;
        }
    }

    /// Re-register the [`CoordinatorAcceptState`] for `network` so a node just
    /// granted the per-network key (via `AdminGrant`) can admit fresh joiners
    /// instead of silently dropping their `JoinRequest`s under
    /// `AcceptHandler::Member`.
    ///
    /// Idempotent: a network already running as coordinator is left untouched
    /// ([`should_promote`]). The needed [`NetworkHandle`] fields are cloned
    /// inside a scoped block so the `DashMap` ref is dropped before the
    /// (synchronous) registration — never held across it.
    pub(crate) async fn promote_to_coordinator(&self, network: &str) {
        let parts = {
            let Some(h) = self.networks.get(network) else {
                return;
            };
            if !should_promote(h.role.clone()) {
                return;
            }
            (
                h.state.clone(),
                h.dht_notify.clone(),
                h.network_key,
                h.disconnect_tx.clone(),
                h.cancel.clone(),
            )
        }; // DashMap ref dropped before the registration below.
        let Some(ctx) = self.mesh_ctx_for(network) else {
            return;
        };
        self.register_coordinator_handler(
            network, parts.0, parts.1, parts.2, parts.3, parts.4, ctx,
        );
        self.refresh_alpns().await;
        tracing::info!(network, "promoted to coordinator accept handler");
    }

    /// Tailscale-style access control. Read-only queries are open to any local
    /// user; mutating commands require the caller to be root or the configured
    /// operator UID; setting the operator itself is root-only. Returns `None`
    /// when the request is permitted, or `Some(error)` to short-circuit it.
    ///
    /// Identity is taken from the connecting socket's `SO_PEERCRED` (the kernel
    /// vouches for it — it can't be forged by the client), so the socket file
    /// mode only has to permit the connection, not gate authority.
    pub(crate) fn check_authorized(
        req: &IpcMessage,
        peer_cred: Option<(u32, u32)>,
    ) -> Option<IpcMessage> {
        // Reads are available to everyone.
        if matches!(req, IpcMessage::Status) {
            return None;
        }

        let uid = peer_cred.map(|(uid, _)| uid);

        // Root may do anything.
        if uid == Some(0) {
            return None;
        }

        // Granting operator access is reserved for root.
        if matches!(req, IpcMessage::SetOperator { .. }) {
            return Some(IpcMessage::Error {
                message: "permission denied: granting operator access requires root \
                          (re-run with sudo)"
                    .to_string(),
            });
        }

        // Otherwise the caller must be the configured operator.
        let operator = config::load().ok().and_then(|c| c.operator_uid);
        if uid.is_some() && uid == operator {
            return None;
        }

        Some(IpcMessage::Error {
            message: "permission denied: this user is not authorized to control tetron.\n\
                      Grant access with: sudo tetron set-operator <user>"
                .to_string(),
        })
    }

    /// Persist the operator UID so that user can run mutating `ray` commands
    /// without root. Authorization (root-only) is enforced in `check_authorized`.
    pub(crate) fn set_operator(&self, uid: u32) -> IpcMessage {
        let mut app_config = match config::load() {
            Ok(c) => c,
            Err(e) => {
                return IpcMessage::Error {
                    message: format!("failed to load config: {e}"),
                };
            }
        };
        app_config.operator_uid = Some(uid);
        if let Err(e) = config::save_settings(&app_config) {
            return IpcMessage::Error {
                message: format!("failed to save config: {e}"),
            };
        }
        IpcMessage::Ok {
            message: format!(
                "operator set to uid {uid}; that user can now run tetron without sudo"
            ),
        }
    }

    pub(crate) async fn handle_request(
        self: &Arc<Self>,
        req: IpcMessage,
        peer_cred: Option<(u32, u32)>,
    ) -> IpcMessage {
        if let Some(denied) = Self::check_authorized(&req, peer_cred) {
            return denied;
        }
        match req {
            IpcMessage::Create {
                mode,
                network_name,
                hostname,
                transport,
                subnet,
            } => {
                let parsed = match subnet
                    .as_deref()
                    .map(crate::membership::parse_cidr)
                    .transpose()
                {
                    Ok(s) => s,
                    Err(e) => {
                        return IpcMessage::Error {
                            message: format!("invalid --subnet: {e:#}"),
                        };
                    }
                };
                self.create_network(mode, network_name, hostname, transport, parsed)
                    .await
            }
            IpcMessage::Join {
                network_key,
                alias,
                hostname,
                transport,
                invite,
                ..
            } => {
                self.join_network(&network_key, alias.as_deref(), hostname, transport, invite)
                    .await
            }
            IpcMessage::Leave { network, force } => {
                self.leave_network(&network, force).await
            }
            IpcMessage::Nuke {
                network_key,
                force,
                cancel,
                second,
            } => {
                self.nuke_network(&network_key, force, cancel, second.as_deref())
                    .await
            }
            IpcMessage::Kick {
                network_key,
                endpoint_id,
            } => self.kick_member(&network_key, &endpoint_id).await,
            IpcMessage::Status => self.status(),
            IpcMessage::Resume { hostname, network } => {
                self.activate(hostname, network.as_deref()).await
            }
            IpcMessage::Standby { network } => self.deactivate(network.as_deref()).await,
            IpcMessage::Shutdown => {
                self.shutdown_token.cancel();
                IpcMessage::Ok {
                    message: "shutting down".to_string(),
                }
            }
            IpcMessage::SetOperator { uid } => self.set_operator(uid),
            IpcMessage::AdminAdd { network, peer } => self.admin_add(&network, &peer).await,
            IpcMessage::AdminList { network } => self.admin_list(&network),
            IpcMessage::InviteCreate { network, expires } => {
                self.invite_create(&network, expires.as_deref()).await
            }
            IpcMessage::InviteList { network } => self.invite_list(&network),
            IpcMessage::InviteRevoke { network, invite_id } => {
                self.invite_revoke(&network, &invite_id).await
            }
            other => IpcMessage::Error {
                message: format!("unexpected message: {:?}", other),
            },
        }
    }

    /// Resolve a peer's endpoint-id prefix (or the literal "self") to
    /// exactly one member across all joined networks. Requires at least as
    /// many characters as the short id `tetron status` displays
    /// (`fmt_short()`'s 10 hex chars), and errors out -- rather than
    /// silently guessing -- if the prefix matches more than one member.
    /// Destructive callers (`kick`, `nuke --second`) depend on this never
    /// picking a peer it wasn't sure about.
    pub(crate) fn resolve_short_id_any_network(&self, short: &str) -> Result<EndpointId, String> {
        if short == "self" {
            return Ok(self.endpoint.id());
        }
        const MIN_PREFIX_LEN: usize = 10;
        if short.len() < MIN_PREFIX_LEN {
            return Err(format!(
                "'{short}' is too short to safely identify a peer -- use at least \
                 {MIN_PREFIX_LEN} characters (the short id shown by `tetron status`), \
                 or the full id"
            ));
        }
        let mut matches: Vec<EndpointId> = Vec::new();
        for entry in self.networks.iter() {
            let state = entry.value().state.read().unwrap();
            for m in state.members.all().iter() {
                if m.identity.to_string().starts_with(short) && !matches.contains(&m.identity) {
                    matches.push(m.identity);
                }
            }
        }
        match matches.as_slice() {
            [] => Err(format!("could not resolve peer '{short}'")),
            [id] => Ok(*id),
            _ => Err(format!(
                "'{short}' is ambiguous -- matches {} peers ({}); use more characters",
                matches.len(),
                matches
                    .iter()
                    .map(|id| id.fmt_short().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Resolve a network's short id (a prefix of its public key) to the
    /// local display name used to key `self.networks`. Requires at least as
    /// many characters as the short id `tetron status` displays
    /// (`fmt_short()`'s 10 hex chars), and errors out -- rather than
    /// silently guessing -- if the prefix matches more than one joined
    /// network. Destructive callers (`nuke`, `kick`) depend on this never
    /// picking a network they weren't sure about; unlike peer resolution,
    /// there is deliberately no local-name/alias fallback here -- the whole
    /// point is that a network's identity is resolved by its cryptographic
    /// key, not by the locally mutable string it happens to be filed under.
    pub(crate) fn resolve_network_short_id(&self, short: &str) -> Result<String, String> {
        const MIN_PREFIX_LEN: usize = 10;
        if short.len() < MIN_PREFIX_LEN {
            return Err(format!(
                "'{short}' is too short to safely identify a network -- use at least \
                 {MIN_PREFIX_LEN} characters (the `network_key` shown by `tetron status`), \
                 or the full value"
            ));
        }
        let matches: Vec<(String, EndpointId)> = self
            .networks
            .iter()
            .filter(|entry| entry.value().network_key.to_string().starts_with(short))
            .map(|entry| (entry.key().clone(), entry.value().network_key))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("could not resolve network '{short}'")),
            [(name, _)] => Ok(name.clone()),
            _ => Err(format!(
                "'{short}' is ambiguous -- matches {} networks ({}); use more characters",
                matches.len(),
                matches
                    .iter()
                    .map(|(name, key)| format!("{name} {}", key.fmt_short()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Resolve `leave`'s network argument: the local display name if it
    /// matches exactly (today's only path, unchanged), else fall back to a
    /// network-key prefix match (same `>=10`-char rule and ambiguity
    /// handling as [`Self::resolve_network_short_id`]). Unlike that
    /// function, this one *does* try the local name first -- `leave` only
    /// ever acts on the caller's own participation (no roster mutation of
    /// anyone else), so the destructive-action argument for key-only
    /// resolution doesn't apply here. Lets a user who only has an invite
    /// key or room id handy (e.g. at uninstall time) still `leave` without
    /// having to remember the local name `tetron status` assigned it.
    pub(crate) fn resolve_network_name_or_key(&self, s: &str) -> Result<String, String> {
        if self.networks.contains_key(s) {
            return Ok(s.to_string());
        }
        // Don't propagate resolve_network_short_id's raw error as-is -- its
        // "too short to safely identify a network" wording assumes the
        // caller was attempting key resolution, which is misleading here
        // when `s` is just a plain local-name typo (the common case for
        // `leave`).
        self.resolve_network_short_id(s).map_err(|_| {
            format!(
                "'{s}' is not a known local network name (see `tetron status`), and does not \
                 resolve as a network key either (needs >=10 characters, matching the \
                 `network_key` shown by `tetron status`, or the full value)"
            )
        })
    }

    // -----------------------------------------------------------------------
    // Join-request handlers (coordinator only)
    // -----------------------------------------------------------------------

    /// Confirm we coordinate `network`, returning its public key, or an error
    /// response if it's absent or we're only a member.
    #[allow(clippy::result_large_err)]
    pub(crate) fn coordinator_handle(
        &self,
        network: &str,
    ) -> std::result::Result<EndpointId, IpcMessage> {
        let Some(handle) = self.networks.get(network) else {
            return Err(IpcMessage::Error {
                message: format!("network '{network}' not active"),
            });
        };
        if !handle.role.is_coordinator() {
            return Err(IpcMessage::Error {
                message: format!("only the coordinator of '{network}' can manage join requests"),
            });
        }
        Ok(handle.network_key)
    }
}

// Process bootstrap + IPC server live in `mesh/bootstrap.rs`; background tasks +
// roster reconvergence in `mesh/background.rs`.

// ---------------------------------------------------------------------------
// Control-message helpers (daemon-initiated, fire-and-forget)
// ---------------------------------------------------------------------------

/// Open a fresh bi stream and send one control message on it. Every
/// daemon-initiated control message rides its own `open_bi` (the control readers
/// drop the request stream's send half, so a reply can't ride it back). Returns
/// the result so callers can log per-peer failures.
async fn open_and_send(conn: &Connection, msg: &ControlMsg) -> Result<()> {
    let (mut send, _recv) = conn.open_bi().await.context("open control stream")?;
    control::send_msg(&mut send, msg).await
}

async fn send_member_sync(conn: &Connection) {
    let _ = open_and_send(conn, &ControlMsg::MemberSync).await;
}

/// Reply to a `tetron ping` probe by echoing `Pong{nonce}` over a fresh stream
/// (see [`open_and_send`] for why the reply can't ride the request stream back).
async fn respond_pong(conn: &Connection, nonce: u64) {
    let _ = open_and_send(conn, &ControlMsg::Pong { nonce }).await;
}

async fn broadcast_member_sync(peers: &PeerTable, exclude_ip: Option<Ipv4Addr>) {
    for (ip, conn) in peers.all_connections() {
        if Some(ip) == exclude_ip {
            continue;
        }
        if let Err(e) = open_and_send(&conn, &ControlMsg::MemberSync).await {
            tracing::warn!(peer_ip = %ip, error = %e, "failed to sync members");
        }
    }
}

async fn broadcast_control_msg(peers: &PeerTable, msg: &ControlMsg) {
    for (_ip, conn) in peers.all_connections() {
        let _ = open_and_send(&conn, msg).await;
    }
}

#[cfg(test)]
mod accept_handler_tests {
    use super::*;
    use crate::membership::default_subnet;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    // Build a minimal NetworkState for use in test AcceptHandler construction.
    fn make_network_state() -> SharedNetworkState {
        let net_secret = SecretKey::from_bytes(&[1u8; 32]);
        let net_pub = net_secret.public();
        Arc::new(RwLock::new(NetworkState {
            generation: 0,
            members: MemberList::new(),
            approved: ApprovedList::new(),
            snapshot: None,
            network_secret_key: None,
            network_public_key: net_pub,
            network_name: Some("test-net".to_string()),
            mode: GroupMode::Restricted,
            subnet: default_subnet(),
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
            nuke_proposals: BTreeMap::new(),
        }))
    }

    /// Throwaway [`MeshCtx`] for accept-handler tests: a fresh blob store and
    /// dummy handles, none of which the constructed handlers exercise here.
    fn sample_mesh_ctx(identity: IrohIdentityProvider, blob_store: FsStore) -> MeshCtx {
        let (tun_tx, _) = tokio::sync::mpsc::channel(1);
        MeshCtx {
            identity,
            network_key: SecretKey::from_bytes(&[1u8; 32]).public(),
            peers: PeerTable::new(),
            tun_tx: Arc::new(arc_swap::ArcSwap::from_pointee(tun_tx)),
            stats: Arc::new(ForwardMetrics::default()),
            blob_store,
            pruned_peers: Arc::new(DashSet::new()),
        }
    }

    async fn sample_coordinator_handler() -> AcceptHandler {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = FsStore::load(tmp.path()).await.unwrap();
        let (disconnect_tx, _) = tokio::sync::mpsc::channel(1);
        let my_key = SecretKey::from_bytes(&[2u8; 32]);
        let my_id = my_key.public();
        AcceptHandler::Coordinator(Arc::new(CoordinatorAcceptState {
            ctx: sample_mesh_ctx(
                IrohIdentityProvider::new(my_id, 0, default_subnet()),
                blob_store,
            ),
            network_name: "test-net".to_string(),
            state: make_network_state(),
            disconnect_tx,
            token: CancellationToken::new(),
            dht_notify: None,
            pending_pongs: Arc::new(DashMap::new()),
        }))
    }

    async fn sample_member_handler() -> AcceptHandler {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = FsStore::load(tmp.path()).await.unwrap();
        let (disconnect_tx, _) = tokio::sync::mpsc::channel(1);
        let my_key = SecretKey::from_bytes(&[3u8; 32]);
        AcceptHandler::Member(Arc::new(MemberAcceptState {
            ctx: sample_mesh_ctx(
                IrohIdentityProvider::new(my_key.public(), 0, default_subnet()),
                blob_store,
            ),
            network_name: "test-net".to_string(),
            state: make_network_state(),
            disconnect_tx,
            token: CancellationToken::new(),
        }))
    }

    #[tokio::test]
    async fn register_replaces_member_handler_with_coordinator() {
        // AcceptHandler exposes whether it is the coordinator variant.
        assert!(!sample_member_handler().await.is_coordinator());
        assert!(sample_coordinator_handler().await.is_coordinator());
    }

    #[test]
    fn holds_key_implies_coordinator_role() {
        assert_eq!(role_for_key_holder(true), NetworkRole::Coordinator);
        assert_eq!(role_for_key_holder(false), NetworkRole::Member);
    }

    #[test]
    fn choose_path_prefers_selected() {
        use ipc::ConnType::*;
        // The selected path wins even when it isn't the "best" type.
        let classes = [(Relay, false), (Direct, true)];
        assert_eq!(super::choose_path_index(&classes), Some(1));
    }

    #[test]
    fn choose_path_falls_back_to_best_unselected() {
        use ipc::ConnType::*;
        // No path selected: report a concrete path (Direct > Relay > Tor)
        // instead of Unknown, so a live connection never shows `?`.
        let classes = [(Relay, false), (Direct, false), (Tor, false)];
        assert_eq!(super::choose_path_index(&classes), Some(1));

        let only_relay = [(Relay, false)];
        assert_eq!(super::choose_path_index(&only_relay), Some(0));
    }

    #[test]
    fn choose_path_empty_is_none() {
        assert_eq!(super::choose_path_index(&[]), None);
    }

    #[test]
    fn promote_is_idempotent_decision() {
        // Re-registering an already-coordinator network is a no-op decision.
        assert!(should_promote(NetworkRole::Member));
        assert!(!should_promote(NetworkRole::Coordinator));
    }
}

#[cfg(test)]
mod coordinator_dial_order_tests {
    use super::*;
    use crate::membership::{Member, default_subnet, derive_ip};

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = SecretKey::from(key_bytes);
        key.public()
    }

    #[test]
    fn dial_order_puts_minter_first_then_other_coordinators() {
        let (a, b, c, me) = (test_id(1), test_id(2), test_id(3), test_id(9));
        let mk = |id, coord| Member {
            identity: id,
            ip: derive_ip(&id, default_subnet()),
            is_coordinator: coord,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let members = vec![mk(a, true), mk(b, true), mk(c, false), mk(me, true)];
        // minter = b: b first, then the other coordinator a, never c (not coord), never me.
        assert_eq!(super::coordinator_dial_order(b, &members, me), vec![b, a]);
    }

    #[test]
    fn dial_order_edge_cases() {
        let (a, b, me) = (test_id(1), test_id(2), test_id(9));
        let mk = |id, coord| Member {
            identity: id,
            ip: derive_ip(&id, default_subnet()),
            is_coordinator: coord,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };

        // No coordinators in the roster ⇒ empty order (caller bails).
        let none_coord = vec![mk(a, false), mk(b, false)];
        assert!(super::coordinator_dial_order(a, &none_coord, me).is_empty());

        // Minter == me (the no-invite case where we pass our own id): we are
        // filtered out, leaving just the other coordinators.
        let members = vec![mk(a, true), mk(me, true)];
        assert_eq!(super::coordinator_dial_order(me, &members, me), vec![a]);

        // Minter isn't a coordinator in the blob: it is not promoted to the
        // front, but real coordinators still get dialed.
        let members = vec![mk(a, true), mk(b, false)];
        assert_eq!(super::coordinator_dial_order(b, &members, me), vec![a]);

        // Minter is a coordinator AND also appears in the member scan: listed
        // once (front), no duplicate.
        let members = vec![mk(a, true), mk(b, true)];
        assert_eq!(super::coordinator_dial_order(a, &members, me), vec![a, b]);
    }

    #[test]
    fn admin_grant_key_accepted_only_when_public_matches_network() {
        // The real network key: its public half is the network pubkey.
        let net_secret = SecretKey::from({
            let mut b = [0u8; 32];
            b[0] = 42;
            b
        });
        let net_pubkey = net_secret.public();

        // A genuine grant carries the real secret → accepted.
        assert!(super::admin_grant_key_valid(
            net_secret.to_bytes(),
            net_pubkey
        ));

        // A forged grant carries an attacker-chosen key whose public half does
        // not match the network pubkey → rejected (no roster lookup needed).
        let forged = SecretKey::from({
            let mut b = [0u8; 32];
            b[0] = 7;
            b
        });
        assert!(!super::admin_grant_key_valid(forged.to_bytes(), net_pubkey));
    }
}

#[cfg(test)]
mod dial_fallback_tests {
    use super::*;

    #[test]
    fn dial_fallback_stops_on_first_welcome() {
        // outcomes simulate dialing in order: first errors, second welcomes, third never tried.
        let outcomes = vec![
            DialOutcome::Unreachable,
            DialOutcome::Welcomed,
            DialOutcome::Denied,
        ];
        let (idx, welcomed) = pick_first_welcome(&outcomes);
        assert_eq!((idx, welcomed), (1, true));
    }

    #[test]
    fn dial_fallback_reports_failure_when_all_exhausted() {
        let outcomes = vec![DialOutcome::Unreachable, DialOutcome::Denied];
        let (_idx, welcomed) = pick_first_welcome(&outcomes);
        assert!(!welcomed);
    }

    #[test]
    fn dial_fallback_empty_is_not_welcomed() {
        // Defensive: no coordinators tried at all. Must not panic and must
        // report "not welcomed" so the caller bails rather than indexing.
        let (idx, welcomed) = pick_first_welcome(&[]);
        assert_eq!((idx, welcomed), (0, false));
    }

    #[test]
    fn dial_fallback_first_welcome_wins_over_later() {
        let outcomes = vec![DialOutcome::Welcomed, DialOutcome::Welcomed];
        let (idx, welcomed) = pick_first_welcome(&outcomes);
        assert_eq!((idx, welcomed), (0, true));
    }
}

#[cfg(test)]
mod headless_tests {
    use super::*;
    use crate::membership::default_subnet;

    /// `build_headless()` constructs a usable `Arc<DaemonState>` (identity,
    /// endpoint, blob store, DNS, pollers) in an isolated config dir and answers a
    /// `status()` call, all without binding the Unix-socket IPC server that
    /// `run_daemon`/`serve_ipc` would.
    ///
    /// Multi-threaded flavor: `build_headless` builds an iroh endpoint and an
    /// iroh-blobs `FsStore` whose background actor tasks must make progress while
    /// the builder awaits, matching the daemon binary's `#[tokio::main]` runtime.
    /// The `timeout` guard turns a future startup regression into a fast failure
    /// instead of a hung test.
    /// Process-wide lock serializing tests that mutate `TETRON_CONFIG_DIR` (or
    /// any other env var read by `config::config_dir()`), since lib tests share
    /// one process and run on parallel threads. Shared with `identity::tests`
    /// via `crate::config::CONFIG_ENV_LOCK` so neither module's tests observe a
    /// `TETRON_CONFIG_DIR` bled through from the other.
    use crate::config::CONFIG_ENV_LOCK as ENV_LOCK;

    /// RAII guard that restores a previous env var value (or removes it if it
    /// was unset) on drop, so the var is restored even if the test body panics.
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    // `ENV_LOCK` is a `Mutex<()>` used only to serialize whole tests against each
    // other; it guards no data mutated across the awaits, so holding it across
    // them is intentional (that is the point) and safe.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn build_headless_returns_usable_state_without_ipc_socket() {
        // Serialize against any other test that touches env vars read by
        // `config::config_dir()`, so no concurrent test observes a bled-through
        // `TETRON_CONFIG_DIR`.
        let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().unwrap();
        // Isolate identity/config/blobs from the system config dir. The guard
        // restores the previous value (or removes the var) on drop, including
        // on panic, so this can't poison later tests.
        let _env_guard = EnvVarGuard::set("TETRON_CONFIG_DIR", tmp.path());

        let daemon = tokio::time::timeout(std::time::Duration::from_secs(30), build_headless())
            .await
            .expect("build_headless should not hang")
            .expect("build_headless should succeed");

        // It returns a shared `Arc<DaemonState>`.
        assert!(Arc::strong_count(&daemon) >= 1);

        // The embedding `status()` API answers without a socket ever being bound.
        assert!(matches!(daemon.status(), IpcMessage::StatusResponse { .. }));
    }

    /// In-memory TUN writer that records every written packet into a shared
    /// buffer, so a test can observe which writer the data plane routed to.
    #[derive(Clone, Default)]
    struct FakeTunWriter {
        written: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    }

    impl crate::tun::TunWrite for FakeTunWriter {
        async fn write_packet(&mut self, packet: &[u8]) -> anyhow::Result<()> {
            self.written.lock().unwrap().push(packet.to_vec());
            Ok(())
        }
    }

    /// In-memory TUN reader that never yields a packet, so `run_mesh` parks in
    /// its read and only exits when its task is cancelled/aborted. It carries an
    /// `Arc<()>` liveness token: the reader is owned solely by the spawned
    /// `run_mesh` future, so the token's strong count drops back to the caller's
    /// single reference the moment that task's future is dropped on abort. That
    /// makes "the old data plane was torn down" directly observable.
    struct FakeTunReader {
        _alive: Arc<()>,
    }

    impl crate::tun::TunRead for FakeTunReader {
        async fn read_into(&mut self, _buf: &mut bytes::BytesMut) -> anyhow::Result<usize> {
            std::future::pending::<()>().await;
            unreachable!("FakeTunReader never returns");
        }
    }

    /// Poll `sink` until it holds `want` packets. Bounded (~2s total) so a real
    /// failure fails fast instead of hanging; the short poll interval leaves room
    /// for the cross-thread wakeup of the writer task without a fixed sleep that
    /// would either flake (too short) or slow the suite (too long).
    async fn wait_for_len(sink: &Arc<std::sync::Mutex<Vec<Vec<u8>>>>, want: usize) -> bool {
        for _ in 0..400 {
            if sink.lock().unwrap().len() >= want {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        false
    }

    /// Re-attaching the TUN after a `detach_tun` must resume forwarding to the
    /// new writer (the VPN off/on toggle path), and a second `attach_tun`
    /// WITHOUT an intervening detach must stop the previous writer instead of
    /// leaking it (two live writers on two fds).
    // See `build_headless_returns_usable_state_without_ipc_socket`: `ENV_LOCK`
    // only serializes tests and guards no data across the awaits.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_tun_is_self_healing_on_reattach_and_double_attach() {
        let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _env_guard = EnvVarGuard::set("TETRON_CONFIG_DIR", tmp.path());

        let daemon = tokio::time::timeout(std::time::Duration::from_secs(30), build_headless())
            .await
            .expect("build_headless should not hang")
            .expect("build_headless should succeed");

        // MULTISEG-002: `attach_tun` is per-network now, so this test needs a
        // `NetworkHandle` to attach to. `build_headless` builds a daemon with no
        // saved networks, so insert a bare-bones one directly rather than going
        // through the real `create_network` (which would also try to create a
        // genuine OS TUN device, publish to the DHT, and mint an invite — heavy
        // side effects this test doesn't want and a sandboxed test runner may
        // lack permission for). Same-module access lets the test build the
        // private `NetworkHandle` fields directly.
        const NET: &str = "test-net";
        {
            let net_secret = SecretKey::from_bytes(&[9u8; 32]);
            let (placeholder_tx, _placeholder_rx) = mpsc::channel::<Bytes>(1);
            let (disconnect_tx, _disconnect_rx) =
                mpsc::channel::<forward::DisconnectEvent>(1);
            daemon.networks.insert(
                NET.to_string(),
                NetworkHandle {
                    name: NET.to_string(),
                    network_key: net_secret.public(),
                    role: NetworkRole::Coordinator,
                    my_ip: daemon.identity.local_ip(),
                    state: Arc::new(RwLock::new(NetworkState {
                        generation: 0,
                        members: MemberList::new(),
                        approved: ApprovedList::new(),
                        snapshot: None,
                        network_secret_key: None,
                        network_public_key: net_secret.public(),
                        network_name: Some(NET.to_string()),
                        mode: GroupMode::Restricted,
                        subnet: default_subnet(),
                        reusable_keys: BTreeMap::new(),
                        invites: BTreeMap::new(),
                        nuke_proposals: BTreeMap::new(),
                    })),
                    dht_notify: None,
                    cancel: CancellationToken::new(),
                    tasks: Vec::new(),
                    disconnect_tx,
                    peers: PeerTable::new(),
                    tun_name: std::sync::Mutex::new("placeholder".to_string()),
                    tun_tx: Arc::new(arc_swap::ArcSwap::from_pointee(placeholder_tx)),
                    tun_tasks: std::sync::Mutex::new(None),
                    active: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        use std::sync::atomic::Ordering;

        // Helper: send one packet through the same `tun_tx` cell the peer-reader
        // and DNS-injection paths use, then wait for the given writer to see it.
        async fn send_pkt(daemon: &Arc<DaemonState>, pkt: &'static [u8]) {
            daemon
                .networks
                .get(NET)
                .expect("test network present")
                .tun_tx
                .load_full()
                .send(Bytes::from_static(pkt))
                .await
                .expect("tun_tx send should reach the live writer");
        }

        // Poll until `token`'s strong count falls back to 1 (only this test
        // holds it), i.e. the `run_mesh` task that owned the matching reader was
        // dropped. Bounded so a leak fails fast instead of hanging.
        async fn wait_for_reader_dropped(token: &Arc<()>) -> bool {
            for _ in 0..400 {
                if Arc::strong_count(token) == 1 {
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            false
        }

        // 1. First attach: reader1 + writer1, forwarding active.
        let writer1 = FakeTunWriter::default();
        let sink1 = writer1.written.clone();
        daemon
            .attach_tun(
                NET,
                FakeTunReader {
                    _alive: Arc::new(()),
                },
                writer1,
            )
            .await;
        daemon.networks.get(NET).unwrap().active.store(true, Ordering::SeqCst);

        send_pkt(&daemon, b"packet-1").await;
        assert!(
            wait_for_len(&sink1, 1).await,
            "writer1 should receive the first packet"
        );

        // 2. Toggle: detach, then re-attach reader2 + writer2. This is the path
        //    that used to silently break before the fresh-channel-per-attach fix.
        daemon.detach_tun(NET);
        let writer2 = FakeTunWriter::default();
        let sink2 = writer2.written.clone();
        let alive2 = Arc::new(());
        daemon
            .attach_tun(
                NET,
                FakeTunReader {
                    _alive: alive2.clone(),
                },
                writer2,
            )
            .await;
        daemon.networks.get(NET).unwrap().active.store(true, Ordering::SeqCst);

        send_pkt(&daemon, b"packet-2").await;
        assert!(
            wait_for_len(&sink2, 1).await,
            "writer2 should receive the packet after a detach->attach toggle"
        );

        // 3. Double-attach guard: attach writer3 WITHOUT detaching first. The
        //    previous data plane (writer2's mesh loop + writer) must be aborted,
        //    not leaked. Observe both halves of "no two live data planes":
        //    - writer3 receives the packet (the cell now routes to writer3), and
        //    - reader2's `run_mesh` task was dropped (`alive2` count back to 1),
        //      which without the self-healing guard would leak and stay at 2.
        let writer3 = FakeTunWriter::default();
        let sink3 = writer3.written.clone();
        daemon
            .attach_tun(
                NET,
                FakeTunReader {
                    _alive: Arc::new(()),
                },
                writer3,
            )
            .await;
        daemon.networks.get(NET).unwrap().active.store(true, Ordering::SeqCst);

        send_pkt(&daemon, b"packet-3").await;
        assert!(
            wait_for_len(&sink3, 1).await,
            "writer3 should receive the packet after a double-attach"
        );
        assert!(
            wait_for_reader_dropped(&alive2).await,
            "the prior mesh loop must be aborted on a second attach without detach (no leak)"
        );

        daemon.detach_tun(NET);
    }

    /// STANDBY-PER-NETWORK: `activate`/`deactivate` scoped to one network
    /// (`--network`) must only touch that network's own `active` flag,
    /// leaving every other joined network's data-plane state untouched. The
    /// unscoped form (`network: None`) must still touch every network,
    /// matching the pre-existing daemon-wide behavior. An unknown network
    /// name must error rather than silently doing nothing.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_deactivate_scope_to_one_network_when_given() {
        let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _env_guard = EnvVarGuard::set("TETRON_CONFIG_DIR", tmp.path());

        let daemon = tokio::time::timeout(std::time::Duration::from_secs(30), build_headless())
            .await
            .expect("build_headless should not hang")
            .expect("build_headless should succeed");

        // Insert two bare-bones networks directly, same technique as
        // `attach_tun_is_self_healing_on_reattach_and_double_attach` above
        // (going through the real create/join path would publish to the
        // DHT). Their real TUN-touching OS calls (set_link_up /
        // route_peer_range) will fail against the placeholder device name
        // -- non-fatal, logged as warnings by `activate`/`deactivate`, and
        // irrelevant to what this test checks: which networks' own `active`
        // flags moved.
        fn insert_bare_network(daemon: &Arc<DaemonState>, name: &str, key_byte: u8) {
            let net_secret = SecretKey::from_bytes(&[key_byte; 32]);
            let (placeholder_tx, _placeholder_rx) = mpsc::channel::<Bytes>(1);
            let (disconnect_tx, _disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(1);
            daemon.networks.insert(
                name.to_string(),
                NetworkHandle {
                    name: name.to_string(),
                    network_key: net_secret.public(),
                    role: NetworkRole::Coordinator,
                    my_ip: daemon.identity.local_ip(),
                    state: Arc::new(RwLock::new(NetworkState {
                        generation: 0,
                        members: MemberList::new(),
                        approved: ApprovedList::new(),
                        snapshot: None,
                        network_secret_key: None,
                        network_public_key: net_secret.public(),
                        network_name: Some(name.to_string()),
                        mode: GroupMode::Restricted,
                        subnet: default_subnet(),
                        reusable_keys: BTreeMap::new(),
                        invites: BTreeMap::new(),
                        nuke_proposals: BTreeMap::new(),
                    })),
                    dht_notify: None,
                    cancel: CancellationToken::new(),
                    tasks: Vec::new(),
                    disconnect_tx,
                    peers: PeerTable::new(),
                    tun_name: std::sync::Mutex::new("placeholder".to_string()),
                    tun_tx: Arc::new(arc_swap::ArcSwap::from_pointee(placeholder_tx)),
                    tun_tasks: std::sync::Mutex::new(None),
                    active: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        insert_bare_network(&daemon, "net-a", 20);
        insert_bare_network(&daemon, "net-b", 21);

        let is_active = |name: &str| {
            daemon
                .networks
                .get(name)
                .unwrap()
                .active
                .load(Ordering::SeqCst)
        };

        assert!(!is_active("net-a"));
        assert!(!is_active("net-b"));

        // Scoped activate: only net-a comes up.
        daemon.activate(None, Some("net-a")).await;
        assert!(
            is_active("net-a"),
            "net-a should be active after a scoped activate"
        );
        assert!(
            !is_active("net-b"),
            "net-b must stay untouched by a scoped activate"
        );

        // Scoped deactivate on the still-standby network: no-op, no error.
        let resp = daemon.deactivate(Some("net-b")).await;
        assert!(matches!(resp, IpcMessage::Ok { .. }));
        assert!(!is_active("net-b"));

        // Unscoped activate: brings up every network, including the one
        // already up (idempotent) and the one still down.
        daemon.activate(None, None).await;
        assert!(is_active("net-a"));
        assert!(
            is_active("net-b"),
            "unscoped activate must bring up every network"
        );

        // Scoped deactivate: only net-a goes down.
        daemon.deactivate(Some("net-a")).await;
        assert!(!is_active("net-a"));
        assert!(
            is_active("net-b"),
            "net-b must stay untouched by a scoped deactivate"
        );

        // Unknown network name errors rather than silently no-op-ing.
        let resp = daemon.activate(None, Some("does-not-exist")).await;
        assert!(matches!(resp, IpcMessage::Error { .. }));
        let resp = daemon.deactivate(Some("does-not-exist")).await;
        assert!(matches!(resp, IpcMessage::Error { .. }));

        // Unscoped deactivate: brings down every network.
        daemon.deactivate(None).await;
        assert!(!is_active("net-a"));
        assert!(!is_active("net-b"));
    }

    /// STRANDED-COORDINATOR-WARN's auto-promotion: a sole-coordinator leave
    /// with other members who are all unreachable (no live connection, so
    /// none can receive the `AdminGrant`) is blocked by default and names
    /// exactly which members would be stranded, rather than silently
    /// abandoning them or requiring the caller to already know who's safe
    /// to leave behind. `--force` still bypasses the check entirely. (The
    /// "successfully auto-promoted a reachable member" happy path needs a
    /// real live QUIC connection to exercise -- not covered here; see this
    /// requirement's own live-testing caveat in `spec/design_spec.py`.)
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn leave_blocks_on_sole_coordinator_with_unreachable_members() {
        let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _env_guard = EnvVarGuard::set("TETRON_CONFIG_DIR", tmp.path());

        let daemon = tokio::time::timeout(std::time::Duration::from_secs(30), build_headless())
            .await
            .expect("build_headless should not hang")
            .expect("build_headless should succeed");

        const NET: &str = "test-net";
        let my_id = daemon.identity.local_identity();
        let member_a = SecretKey::from_bytes(&[30u8; 32]).public();
        let member_b = SecretKey::from_bytes(&[31u8; 32]).public();

        let mut members = MemberList::new();
        members
            .add(Member {
                identity: my_id,
                ip: daemon.identity.local_ip(),
                is_coordinator: true,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        members
            .add(Member {
                identity: member_a,
                ip: Ipv4Addr::new(10, 88, 0, 2),
                is_coordinator: false,
                hostname: Some("member-a".to_string()),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        members
            .add(Member {
                identity: member_b,
                ip: Ipv4Addr::new(10, 88, 0, 3),
                is_coordinator: false,
                hostname: Some("member-b".to_string()),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();

        let net_secret = SecretKey::from_bytes(&[32u8; 32]);
        let (placeholder_tx, _placeholder_rx) = mpsc::channel::<Bytes>(1);
        let (disconnect_tx, _disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(1);
        daemon.networks.insert(
            NET.to_string(),
            NetworkHandle {
                name: NET.to_string(),
                network_key: net_secret.public(),
                role: NetworkRole::Coordinator,
                my_ip: daemon.identity.local_ip(),
                state: Arc::new(RwLock::new(NetworkState {
                    generation: 0,
                    members,
                    approved: ApprovedList::new(),
                    snapshot: None,
                    network_secret_key: Some(net_secret.clone()),
                    network_public_key: net_secret.public(),
                    network_name: Some(NET.to_string()),
                    mode: GroupMode::Restricted,
                    subnet: default_subnet(),
                    reusable_keys: BTreeMap::new(),
                    invites: BTreeMap::new(),
                    nuke_proposals: BTreeMap::new(),
                })),
                dht_notify: None,
                cancel: CancellationToken::new(),
                tasks: Vec::new(),
                disconnect_tx,
                // Empty: no live connection to either member, so neither
                // can receive an AdminGrant -- both are "offline" for the
                // purposes of this test.
                peers: PeerTable::new(),
                tun_name: std::sync::Mutex::new("placeholder".to_string()),
                tun_tx: Arc::new(arc_swap::ArcSwap::from_pointee(placeholder_tx)),
                tun_tasks: std::sync::Mutex::new(None),
                active: Arc::new(AtomicBool::new(false)),
            },
        );

        // Blocked by default: both members are unreachable, so neither can
        // be auto-promoted, and the leave must not proceed.
        let resp = daemon.leave_network(NET, false).await;
        match &resp {
            IpcMessage::Error { message } => {
                assert!(message.contains(&member_a.fmt_short().to_string()));
                assert!(message.contains(&member_b.fmt_short().to_string()));
            }
            other => panic!("expected a blocking Error, got {other:?}"),
        }
        assert!(
            daemon.networks.contains_key(NET),
            "a blocked leave must not tear down the network"
        );

        // --force bypasses the check entirely and proceeds.
        let resp = daemon.leave_network(NET, true).await;
        assert!(matches!(resp, IpcMessage::Ok { .. }));
        assert!(!daemon.networks.contains_key(NET));
    }

    /// STATUS-004: hostname resolution (`admin add`, etc.) must be
    /// case-insensitive. Every roster hostname is already lowercased at
    /// creation (`hostname::sanitize_hostname`), so a user typing it back
    /// with different capitalization (e.g. mirroring their OS hostname's own
    /// casing) should still resolve -- found live when `erikk-ThinkPad-P1`
    /// failed to resolve against the roster's `erikk-thinkpad-p1`.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_peer_name_is_case_insensitive() {
        let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _env_guard = EnvVarGuard::set("TETRON_CONFIG_DIR", tmp.path());

        let daemon = tokio::time::timeout(std::time::Duration::from_secs(30), build_headless())
            .await
            .expect("build_headless should not hang")
            .expect("build_headless should succeed");

        const NET: &str = "test-net";
        let member = SecretKey::from_bytes(&[40u8; 32]).public();

        let mut members = MemberList::new();
        members
            .add(Member {
                identity: member,
                ip: Ipv4Addr::new(10, 88, 0, 2),
                is_coordinator: false,
                hostname: Some("erikk-thinkpad-p1".to_string()),
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();

        let net_secret = SecretKey::from_bytes(&[41u8; 32]);
        let (placeholder_tx, _placeholder_rx) = mpsc::channel::<Bytes>(1);
        let (disconnect_tx, _disconnect_rx) = mpsc::channel::<forward::DisconnectEvent>(1);
        daemon.networks.insert(
            NET.to_string(),
            NetworkHandle {
                name: NET.to_string(),
                network_key: net_secret.public(),
                role: NetworkRole::Coordinator,
                my_ip: daemon.identity.local_ip(),
                state: Arc::new(RwLock::new(NetworkState {
                    generation: 0,
                    members,
                    approved: ApprovedList::new(),
                    snapshot: None,
                    network_secret_key: Some(net_secret.clone()),
                    network_public_key: net_secret.public(),
                    network_name: Some(NET.to_string()),
                    mode: GroupMode::Restricted,
                    subnet: default_subnet(),
                    reusable_keys: BTreeMap::new(),
                    invites: BTreeMap::new(),
                    nuke_proposals: BTreeMap::new(),
                })),
                dht_notify: None,
                cancel: CancellationToken::new(),
                tasks: Vec::new(),
                disconnect_tx,
                peers: PeerTable::new(),
                tun_name: std::sync::Mutex::new("placeholder".to_string()),
                tun_tx: Arc::new(arc_swap::ArcSwap::from_pointee(placeholder_tx)),
                tun_tasks: std::sync::Mutex::new(None),
                active: Arc::new(AtomicBool::new(false)),
            },
        );

        for candidate in [
            "erikk-ThinkPad-P1",
            "ERIKK-THINKPAD-P1",
            "erikk-thinkpad-p1",
        ] {
            assert_eq!(
                daemon.resolve_peer_name(NET, candidate).await,
                Ok(member),
                "hostname resolution should be case-insensitive for '{candidate}'"
            );
        }

        assert!(
            daemon
                .resolve_peer_name(NET, "not-a-real-host")
                .await
                .is_err()
        );
    }
}
