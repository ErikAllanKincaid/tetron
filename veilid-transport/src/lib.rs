//! An [`iroh`] `CustomTransport` (the `unstable-custom-transports`
//! mechanism also used by `iroh-tor-transport`, tetron's `tor` feature)
//! carrying QUIC traffic over a companion `tetron-veilid` daemon.
//!
//! `spec/core.py`'s `VeilidExternalDaemonProtocol` (`VEILID-017`) is the
//! requirement this crate implements; read its docstring, and
//! `VeilidCustomTransportMechanism`'s (`VEILID-001`) own UPDATE, for the
//! full rationale. Summary: this crate used to embed a `veilid-core` node
//! in-process; it is now a thin TCP/JSON client to `tetron-veilid` (a
//! separate addon repo building `veilid-server` with
//! `--features footgun-nodeid-target`), the same shape `iroh-tor-transport`
//! already uses for Tor. The public API below is unchanged from the
//! embedded design on purpose -- `src/transport.rs` (tetron core's entire
//! integration surface with this crate) needs zero changes as a result.
//!
//! # Addressing
//!
//! Each Veilid node has a stable `NodeId` (unlike a private/safety route,
//! which rotates). This transport addresses peers directly by `NodeId` --
//! see `VEILID-001`'s docstring for why receiver-anonymity (private
//! routes) was intentionally not chosen for tetron's use case
//! (mutually-known, invite-gated peers, not anonymous hidden services).
//!
//! # Routing mode: `Unsafe`, not the library default `Safe`
//!
//! The connection is put into `SafetySelection::Unsafe` on handshake — a
//! direct send, no sender-anonymizing safety route. This is a deliberate
//! product decision, reasserted from the embedded design, not revisited:
//! `Safe` never once delivered a message across the entire live
//! investigation behind `VEILID-007`..`015` (`spec/core.py`), despite
//! `poll_send`/`app_message` locally reporting success every time.
//! `Unsafe` worked immediately and reliably. Accepted tradeoff: within the
//! Veilid network itself, a network-level observer can now tell which real
//! Veilid node is talking to which — but tetron peers already know each
//! other's identity directly (invite-gated mesh, not anonymous), and
//! Veilid is a last-resort fallback ranked below Tor
//! (`select.rs::choose_path_index`), which already provides the more
//! mature anonymity property for whoever actually needs it.

mod client;
mod wire;

use std::fmt;
use std::io;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use tokio::sync::mpsc;

/// Transport id for this crate's [`CustomAddr`]s.
///
/// Not registered in iroh's `TRANSPORTS.md` registry (`VEILID-001` is
/// unshipped) -- pick and submit a real id before this ships as a tetron
/// feature.
pub const VEILID_TRANSPORT_ID: u64 = u64::from_be_bytes(*b"\0\0veilid");

/// A Veilid node id, in its wire string form (`"VLD0:<base64url-nopad>"`).
///
/// This is a local, opaque-string newtype -- not a re-export of
/// `veilid_core::NodeId` -- since this crate no longer depends on
/// `veilid-core` at all (VEILID-017). It carries no cryptographic
/// operations of its own; every use in this crate is "pass this string to
/// `tetron-veilid` verbatim, or parse a roster string enough to reject
/// garbage cleanly."
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(String);

/// Veilid's own node-id crypto-kind prefix (`VLD0` = the current default
/// crypto kind) followed by a 32-byte public key, base64url-nopad encoded
/// -- confirmed live against real captured node ids throughout this
/// session (e.g. `VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo`).
const NODE_ID_PREFIX: &str = "VLD0:";
const NODE_ID_KEY_LEN: usize = 32;

impl FromStr for NodeId {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(NODE_ID_PREFIX)
            .ok_or_else(|| io::Error::other("veilid NodeId missing 'VLD0:' prefix"))?;
        let decoded = wire::b64_decode(rest)
            .map_err(|e| io::Error::other(format!("veilid NodeId is not valid base64url: {e}")))?;
        if decoded.len() != NODE_ID_KEY_LEN {
            return Err(io::Error::other(format!(
                "veilid NodeId decoded to {} bytes, expected {NODE_ID_KEY_LEN}",
                decoded.len()
            )));
        }
        Ok(NodeId(s.to_string()))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Encodes a Veilid [`NodeId`] as the [`CustomAddr`] this transport
/// understands, for manual, out-of-band address exchange (no discovery
/// mechanism exists yet -- see `VEILID-001`'s module docs).
pub fn node_id_to_custom_addr(node_id: &NodeId) -> CustomAddr {
    CustomAddr::from_parts(VEILID_TRANSPORT_ID, node_id.0.as_bytes())
}

fn parse_custom_addr(addr: &CustomAddr) -> io::Result<NodeId> {
    if addr.id() != VEILID_TRANSPORT_ID {
        return Err(io::Error::other("not a veilid custom address"));
    }
    std::str::from_utf8(addr.data())
        .map_err(|_| io::Error::other("veilid custom address is not utf8"))?
        .parse::<NodeId>()
}

/// Builds a [`VeilidCustomTransport`] by connecting to a companion
/// `tetron-veilid` daemon.
#[derive(Debug, Clone, Default)]
pub struct VeilidTransportBuilder {}

impl VeilidTransportBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects to the companion daemon and returns a ready-to-use custom
    /// transport immediately, **without** waiting for this node's own
    /// identity to resolve first -- matching the embedded design's own
    /// non-blocking `build()` contract (`bind_endpoint`,
    /// `src/transport.rs`, awaits this synchronously while building the
    /// one shared iroh `Endpoint` at daemon startup, before the IPC
    /// socket, TUN, or any other network is even up).
    pub async fn build(self) -> anyhow::Result<VeilidCustomTransport> {
        self.build_with_addr(
            client::DEFAULT_DAEMON_ADDR
                .parse()
                .expect("DEFAULT_DAEMON_ADDR is a valid socket address"),
        )
        .await
    }

    /// Test-only hook so a mock daemon on a random port can stand in for
    /// the real one -- the production `build()` above always uses the
    /// hardcoded default, matching `iroh-tor-transport`'s own precedent of
    /// not exposing the daemon address as a config knob (see
    /// `VEILID-017`'s docstring).
    async fn build_with_addr(
        self,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<VeilidCustomTransport> {
        let (reader, writer, rc_id) = client::connect_and_handshake(addr).await?;
        let (inbound_tx, rx) = mpsc::unbounded_channel::<(CustomAddr, Vec<u8>)>();
        let (node_id_tx, _) = tokio::sync::watch::channel(None);
        let handle =
            client::spawn_actor(addr, reader, writer, rc_id, inbound_tx, node_id_tx.clone());

        let identity_task = tokio::spawn(client::identity_poll_loop(
            handle.clone(),
            node_id_tx.clone(),
        ));
        let local_addrs = n0_watcher::Watchable::new(Vec::<CustomAddr>::new());
        {
            // Runs for the transport's whole life, not just until the
            // first resolution -- a reconnect (VEILID-019) can re-resolve
            // to a *different* identity (a fresh tetron-veilid process is
            // a fresh Veilid identity), and this needs to keep publishing
            // whatever `node_id_tx` currently holds.
            let mut node_id_rx = node_id_tx.subscribe();
            let local_addrs = local_addrs.clone();
            tokio::spawn(async move {
                loop {
                    if let Some(node_id) = node_id_rx.borrow_and_update().clone() {
                        local_addrs.set(vec![node_id_to_custom_addr(&node_id)]).ok();
                    }
                    if node_id_rx.changed().await.is_err() {
                        return;
                    }
                }
            });
        }
        let shared = Arc::new(Shared {
            node_id_tx,
            handle,
            identity_task,
        });
        Ok(VeilidCustomTransport {
            shared,
            local_addrs,
            receiver: Arc::new(Mutex::new(Some(rx))),
        })
    }
}

struct Shared {
    node_id_tx: tokio::sync::watch::Sender<Option<NodeId>>,
    /// The connection actor's own handle -- `poll_send` sends `AppMessage`
    /// requests through this (VEILID-018); `identity_task` also used it to
    /// poll `GetState` during identity resolution.
    handle: client::ClientHandle,
    /// Aborted on [`VeilidCustomTransport::shutdown`].
    identity_task: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("node_id", &*self.node_id_tx.borrow())
            .finish_non_exhaustive()
    }
}

type InboundReceiver = mpsc::UnboundedReceiver<(CustomAddr, Vec<u8>)>;

/// The [`CustomTransport`] entry point. Cheap to clone (shares the
/// underlying connection state via `Arc`).
#[derive(Debug, Clone)]
pub struct VeilidCustomTransport {
    shared: Arc<Shared>,
    local_addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
    receiver: Arc<Mutex<Option<InboundReceiver>>>,
}

impl VeilidCustomTransport {
    /// This node's own address, for handing to peers out of band (no
    /// discovery mechanism exists yet -- see the module docs). `None`
    /// until identity resolves in the background.
    pub fn own_addr(&self) -> Option<CustomAddr> {
        self.own_node_id().map(|id| node_id_to_custom_addr(&id))
    }

    /// This node's own Veilid [`NodeId`], once resolved -- see [`Self::own_addr`].
    pub fn own_node_id(&self) -> Option<NodeId> {
        self.shared.node_id_tx.borrow().clone()
    }

    /// Waits until this node's own identity has resolved, however long
    /// that takes. Returns immediately if already known.
    pub async fn wait_for_own_node_id(&self) -> NodeId {
        let mut rx = self.shared.node_id_tx.subscribe();
        loop {
            if let Some(node_id) = rx.borrow_and_update().clone() {
                return node_id;
            }
            if rx.changed().await.is_err() {
                // Sender dropped -- can't happen while `self` is alive,
                // since `Shared` owns it. Park rather than spin.
                std::future::pending::<()>().await;
            }
        }
    }

    /// Stops background work (identity resolution / the connection actor).
    /// Best-effort; there is no way to signal shutdown back through the
    /// `CustomTransport`/`CustomEndpoint` traits themselves, so callers
    /// that need a clean stop (tests, in particular) should call this
    /// explicitly.
    pub async fn shutdown(self) {
        self.shared.identity_task.abort();
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
        let node_id = match parse_custom_addr(dst) {
            Ok(id) => id,
            Err(e) => return Poll::Ready(Err(e)),
        };
        let rc_id = self.shared.handle.rc_id();
        let payload = transmit.contents.to_vec();
        tracing::debug!(%node_id, len = payload.len(), "veilid-transport: poll_send invoked, sending app_message");
        // Fire-and-forget, matching UDP's own unreliable-send semantics --
        // see ClientHandle::send_fire_and_forget's own doc comment.
        self.shared
            .handle
            .send_fire_and_forget(wire::req_app_message(0, rc_id, &node_id.0, &payload));
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::test_support::{Step, spawn_scripted_daemon};

    fn ok_response(id: u32, value: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"type": "Response", "id": id, "value": value})
    }

    #[test]
    fn node_id_round_trip() {
        let valid = "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo";
        let node_id: NodeId = valid.parse().expect("valid node id");
        assert_eq!(node_id.to_string(), valid);
    }

    #[test]
    fn node_id_rejects_wrong_prefix() {
        assert!("XYZ0:abc".parse::<NodeId>().is_err());
    }

    #[test]
    fn node_id_rejects_bad_base64() {
        assert!("VLD0:not-valid-base64!!!".parse::<NodeId>().is_err());
    }

    #[test]
    fn node_id_rejects_wrong_length() {
        // Valid base64url, wrong decoded length (not 32 bytes).
        assert!("VLD0:aGVsbG8".parse::<NodeId>().is_err());
    }

    #[tokio::test]
    async fn build_resolves_own_identity_without_blocking() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            Step::Reply(ok_response(100, serde_json::json!({"network": {"node_ids": []}}))),
            Step::Reply(ok_response(
                101,
                serde_json::json!({"network": {"node_ids": ["VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo"]}}),
            )),
        ])
        .await;

        let transport = VeilidTransportBuilder::new()
            .build_with_addr(addr)
            .await
            .expect("build should not require identity to be known yet");
        assert!(transport.own_node_id().is_none());

        let node_id = transport.wait_for_own_node_id().await;
        assert_eq!(
            node_id.to_string(),
            "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo"
        );
        assert_eq!(transport.own_addr(), Some(node_id_to_custom_addr(&node_id)));

        transport.shutdown().await;
    }

    #[tokio::test]
    async fn build_fails_cleanly_when_daemon_unreachable() {
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(
            VeilidTransportBuilder::new()
                .build_with_addr(addr)
                .await
                .is_err()
        );
    }
}
