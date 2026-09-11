//! An [`iroh`] `CustomTransport` (the `unstable-custom-transports` mechanism
//! also used by `iroh-tor-transport`, tetron's `tor` feature) that carries
//! QUIC traffic over an embedded [`veilid-core`] node instead of raw UDP.
//!
//! `spec/core.py`'s `VeilidCustomTransportMechanism` (`VEILID-001`) is the
//! requirement this crate implements; read its docstring for the full
//! rationale and the explicitly-deferred follow-up work. Summary of what is
//! **not** built here:
//!
//! - **No automatic discovery.** There is no `AddressLookup` implementation
//!   resolving an iroh [`iroh::EndpointId`] to a peer's Veilid [`NodeId`].
//!   Callers must already know the peer's `NodeId` out of band (see
//!   [`node_id_to_custom_addr`]) and build an `EndpointAddr` with it
//!   directly, the way this crate's own integration test does.
//! - **No hardened `VeilidConfig`.** [`VeilidTransportBuilder::build`] reuses
//!   `veilid-core`'s own `test-util` fixture config (self-signed TLS
//!   certificate, insecure-fallback protected store) as a placeholder.
//! - **Not wired into tetron.** Nothing in `tetron`'s own `Cargo.toml`,
//!   `src/transport.rs`, CLI, or config schema references this crate yet.
//!
//! # Addressing
//!
//! Each Veilid node has a stable `NodeId` (unlike a private/safety route,
//! which rotates). This transport addresses peers directly by `NodeId`
//! (`veilid_core::Target::NodeId`), which keeps Veilid's default
//! sender-privacy (safety routing) without taking on route-churn
//! bookkeeping — see `VEILID-001`'s docstring for why receiver-anonymity
//! (private routes) was intentionally not chosen for tetron's use case
//! (mutually-known, invite-gated peers, not anonymous hidden services).

use std::fmt;
use std::io;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use tokio::sync::mpsc;
pub use veilid_core::NodeId;
use veilid_core::tests::fixture_veilid_core_with_namespace;
use veilid_core::{RoutingContext, Target, VeilidAPI, VeilidUpdate, api_startup};

/// Transport id for this crate's [`CustomAddr`]s.
///
/// Not registered in iroh's `TRANSPORTS.md` registry (`VEILID-001` is
/// unshipped) -- pick and submit a real id before this ships as a tetron
/// feature.
pub const VEILID_TRANSPORT_ID: u64 = u64::from_be_bytes(*b"\0\0veilid");

/// Encodes a Veilid [`NodeId`] as the [`CustomAddr`] this transport
/// understands, for manual, out-of-band address exchange (no discovery
/// mechanism exists yet -- see the module docs).
pub fn node_id_to_custom_addr(node_id: &NodeId) -> CustomAddr {
    CustomAddr::from_parts(VEILID_TRANSPORT_ID, node_id.to_string().as_bytes())
}

fn parse_custom_addr(addr: &CustomAddr) -> io::Result<NodeId> {
    if addr.id() != VEILID_TRANSPORT_ID {
        return Err(io::Error::other("not a veilid custom address"));
    }
    std::str::from_utf8(addr.data())
        .map_err(|_| io::Error::other("veilid custom address is not utf8"))?
        .parse::<NodeId>()
        .map_err(|_| io::Error::other("invalid veilid node id"))
}

/// Builds a [`VeilidCustomTransport`] by starting an embedded Veilid node.
#[derive(Debug, Clone, Default)]
pub struct VeilidTransportBuilder {
    namespace: String,
}

impl VeilidTransportBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Partitions `veilid-core`'s on-disk state (mirrors `veilid-core`'s own
    /// `namespace` config field). Must be unique per node identity running
    /// in the same process; defaults to the empty string.
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Starts the embedded Veilid node and waits for it to attach to the
    /// public Veilid network, returning a ready-to-use custom transport.
    ///
    /// See the module docs for what this deliberately does not do yet
    /// (hardened config, automatic discovery).
    pub async fn build(self) -> anyhow::Result<VeilidCustomTransport> {
        let (_default_cb, config) = fixture_veilid_core_with_namespace(&self.namespace);
        let (tx, rx) = mpsc::unbounded_channel::<(CustomAddr, Vec<u8>)>();
        let update_cb: veilid_core::UpdateCallback = Arc::new(move |update| {
            if let VeilidUpdate::AppMessage(msg) = update
                && let Some(sender) = msg.sender()
            {
                let _ = tx.send((node_id_to_custom_addr(sender), msg.message().to_vec()));
            }
        });
        let api = api_startup(update_cb, config).await?;
        wait_public_internet_ready(&api).await;
        let routing_context = api.routing_context()?;
        let node_id = own_node_id(&api).await?;
        let local_addr = node_id_to_custom_addr(&node_id);
        Ok(VeilidCustomTransport {
            shared: Arc::new(Shared {
                api,
                routing_context,
                node_id,
            }),
            local_addrs: n0_watcher::Watchable::new(vec![local_addr]),
            receiver: Arc::new(Mutex::new(Some(rx))),
        })
    }
}

async fn wait_public_internet_ready(api: &VeilidAPI) {
    loop {
        if let Ok(state) = api.get_state().await
            && state.attachment.public_internet_ready
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn own_node_id(api: &VeilidAPI) -> anyhow::Result<NodeId> {
    let state = api.get_state().await?;
    state
        .network
        .node_ids
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("veilid node reported no node id after startup"))
}

struct Shared {
    api: VeilidAPI,
    routing_context: RoutingContext,
    node_id: NodeId,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("node_id", &self.node_id)
            .finish_non_exhaustive()
    }
}

type InboundReceiver = mpsc::UnboundedReceiver<(CustomAddr, Vec<u8>)>;

/// The [`CustomTransport`] entry point. Cheap to clone (shares the
/// underlying embedded Veilid node via `Arc`).
#[derive(Debug, Clone)]
pub struct VeilidCustomTransport {
    shared: Arc<Shared>,
    local_addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
    receiver: Arc<Mutex<Option<InboundReceiver>>>,
}

impl VeilidCustomTransport {
    /// This node's own address, for handing to peers out of band (no
    /// discovery mechanism exists yet -- see the module docs).
    pub fn own_addr(&self) -> CustomAddr {
        node_id_to_custom_addr(&self.shared.node_id)
    }

    /// This node's own Veilid [`NodeId`].
    pub fn own_node_id(&self) -> NodeId {
        self.shared.node_id.clone()
    }

    /// Shuts down the embedded Veilid node. Best-effort; there is no way to
    /// signal shutdown back through the `CustomTransport`/`CustomEndpoint`
    /// traits themselves, so callers that need a clean stop (tests, in
    /// particular) should call this explicitly.
    pub async fn shutdown(self) {
        self.shared.api.clone().shutdown().await;
    }
}

impl CustomTransport for VeilidCustomTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let receiver = self
            .receiver
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| io::Error::other("veilid custom transport already bound once"))?;
        Ok(Box::new(VeilidCustomEndpoint {
            shared: self.shared.clone(),
            local_addrs: self.local_addrs.clone(),
            receiver,
        }))
    }
}

#[derive(Debug)]
struct VeilidCustomEndpoint {
    shared: Arc<Shared>,
    local_addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
    receiver: InboundReceiver,
}

impl CustomEndpoint for VeilidCustomEndpoint {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.local_addrs.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(VeilidCustomSender {
            shared: self.shared.clone(),
        })
    }

    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        recv_infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        let n = bufs.len();
        if n == 0 {
            return Poll::Ready(Ok(0));
        }
        let mut items = Vec::new();
        match self.receiver.poll_recv_many(cx, &mut items, n) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(0) => {
                return Poll::Ready(Err(io::Error::other("veilid update channel closed")));
            }
            Poll::Ready(_) => {}
        }
        let mut count = 0;
        for (from, data) in items {
            if data.len() > bufs[count].len() {
                // Shouldn't happen for QUIC-sized datagrams; drop rather than panic.
                continue;
            }
            bufs[count][..data.len()].copy_from_slice(&data);
            metas[count].len = data.len();
            metas[count].stride = data.len();
            recv_infos[count] = RecvInfo::new(from, None);
            count += 1;
        }
        Poll::Ready(Ok(count))
    }
}

#[derive(Debug)]
struct VeilidCustomSender {
    shared: Arc<Shared>,
}

impl CustomSender for VeilidCustomSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == VEILID_TRANSPORT_ID
    }

    fn poll_send(
        &self,
        _cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let target = match parse_custom_addr(dst) {
            Ok(node_id) => Target::NodeId(node_id),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let routing_context = self.shared.routing_context.clone();
        let payload = transmit.contents.to_vec();
        // `app_message` is async; fire-and-forget, matching UDP's own
        // unreliable-send semantics (no delivery confirmation here either).
        tokio::spawn(async move {
            if let Err(e) = routing_context.app_message(target, payload).await {
                tracing::debug!("veilid-transport: app_message send failed: {e}");
            }
        });
        Poll::Ready(Ok(()))
    }
}
