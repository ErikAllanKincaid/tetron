//! The connection actor: owns the TCP socket to `tetron-veilid`'s
//! `client_api`, performs the initial handshake, then multiplexes ongoing
//! requests (identity polling, `AppMessage` sends) and demultiplexes
//! inbound `AppMessage` pushes over that one connection. See
//! `spec/core.py`'s `VeilidExternalDaemonProtocol` (VEILID-017) and
//! `VeilidExternalDaemonDataPath` (VEILID-018).

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use iroh_base::CustomAddr;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot, watch};

use crate::wire::{self, Incoming};
use crate::{NodeId, node_id_to_custom_addr};

/// `veilid-server`'s own upstream-documented default
/// (`veilid-server/src/settings.rs`'s `client_api.listen_address`) --
/// `tetron-veilid` adopts it rather than inventing a tetron-specific port,
/// matching how `iroh-tor-transport` hardcodes Tor's own ControlPort 9051.
pub(crate) const DEFAULT_DAEMON_ADDR: &str = "127.0.0.1:5959";

/// How many times [`identity_poll_loop`] polls `GetState` before giving
/// up, and the delay between attempts -- mirrors the embedded design's own
/// bounded background-resolution contract
/// (`spawn_identity_and_attach_watcher`, up to 5 minutes at 500ms
/// intervals).
const IDENTITY_POLL_ATTEMPTS: usize = 600;
const IDENTITY_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A single request/response round trip against a fresh, not-yet-actored
/// connection -- used only during the handshake sequence
/// (`NewRoutingContext` -> `WithSafety`), before [`spawn_actor`] takes
/// ownership of the connection for everything after.
async fn call_once(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    request: Value,
) -> io::Result<Value> {
    let id = request
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| io::Error::other("veilid-transport: request missing id"))?;
    let mut line = serde_json::to_string(&request).map_err(|e| {
        io::Error::other(format!("veilid-transport: failed to encode request: {e}"))
    })?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;

    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            return Err(io::Error::other(
                "veilid-transport: connection closed during handshake",
            ));
        }
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            continue;
        }
        let incoming: Incoming = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, line = trimmed, "veilid-transport: unparseable line during handshake, skipping");
                continue;
            }
        };
        match incoming {
            Incoming::Response(r) if r.id as u64 == id => {
                if let Some(err) = r.error {
                    return Err(io::Error::other(format!(
                        "veilid-transport: daemon returned an error: {}",
                        err.message
                    )));
                }
                return Ok(r.value);
            }
            Incoming::Response(_) | Incoming::Update(_) => {
                // Not the reply we're waiting for (a stale response from a
                // prior attempt, or an unsolicited push) -- ignore during
                // the handshake sequence.
                continue;
            }
        }
    }
}

/// Connects to `addr` and runs the handshake: `NewRoutingContext` ->
/// `WithSafety{Unsafe(PreferUnordered)}` (VEILID-001/007's decided safety
/// selection, reasserted here). Returns the connection halves and the
/// `Unsafe`-mode routing context id, ready for [`spawn_actor`].
pub(crate) async fn connect_and_handshake(
    addr: SocketAddr,
) -> io::Result<(BufReader<OwnedReadHalf>, OwnedWriteHalf, u32)> {
    let stream = TcpStream::connect(addr).await?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let base_rc_id = wire::extract_u32(
        &call_once(
            &mut reader,
            &mut write_half,
            wire::req_new_routing_context(1),
        )
        .await?,
    )
    .ok_or_else(|| {
        io::Error::other("veilid-transport: NewRoutingContext did not return a routing context id")
    })?;
    let unsafe_rc_id = wire::extract_u32(
        &call_once(
            &mut reader,
            &mut write_half,
            wire::req_with_safety_unsafe(2, base_rc_id),
        )
        .await?,
    )
    .ok_or_else(|| {
        io::Error::other("veilid-transport: WithSafety did not return a routing context id")
    })?;

    Ok((reader, write_half, unsafe_rc_id))
}

/// One outstanding ask of the connection actor: send `body` (its `"id"`
/// field is overwritten by the actor with a freshly allocated one -- a
/// caller building `body` via `wire::req_*` can pass any placeholder) and
/// deliver the correlated response (or a clean disconnected/daemon error)
/// back through `reply`.
enum Command {
    Send {
        body: Value,
        reply: oneshot::Sender<io::Result<Value>>,
    },
}

/// A cheap-to-clone handle to the connection actor. Every caller wanting
/// to talk to the daemon (identity polling, `poll_send`) goes through
/// this -- the actor is the sole owner of the socket, the request-id
/// counter, and the pending-request table, so no lock is needed anywhere.
#[derive(Clone)]
pub(crate) struct ClientHandle {
    cmd_tx: mpsc::UnboundedSender<Command>,
    /// The `Unsafe`-mode routing context id from the handshake. Fixed for
    /// VEILID-018's scope (no reconnect yet -- VEILID-019 will need to
    /// make this reconnect-aware).
    pub(crate) rc_id: u32,
}

impl ClientHandle {
    /// Sends `body` and awaits the correlated response's `value`, or a
    /// clean error (daemon-reported, or the connection/actor being gone).
    pub(crate) async fn call(&self, body: Value) -> io::Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Send { body, reply: tx })
            .map_err(|_| io::Error::other("veilid-transport: connection actor is gone"))?;
        rx.await.map_err(|_| {
            io::Error::other("veilid-transport: connection actor dropped the request")
        })?
    }

    /// Fire-and-forget: sends `body` and spawns a task that awaits the
    /// reply purely for logging, matching UDP's own unreliable-send
    /// semantics (the embedded design's `poll_send` had the identical
    /// contract: `tokio::spawn(routing_context.app_message(..))`, ignoring
    /// the result beyond a debug log). Never blocks the caller.
    pub(crate) fn send_fire_and_forget(&self, body: Value) {
        let this = self.clone();
        tokio::spawn(async move {
            match this.call(body).await {
                Ok(_) => tracing::debug!("veilid-transport: app_message send succeeded"),
                Err(e) => tracing::debug!("veilid-transport: app_message send failed: {e}"),
            }
        });
    }
}

/// Takes ownership of an already-handshaken connection and turns it into
/// an ongoing multiplexing actor: [`ClientHandle::call`]/
/// `send_fire_and_forget` requests are written out with freshly allocated
/// ids, correlated responses are routed back by id, and unsolicited
/// `AppMessage` pushes are decoded and pushed into `inbound_tx` (the same
/// channel [`crate::VeilidCustomEndpoint::poll_recv`] drains).
pub(crate) fn spawn_actor(
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    rc_id: u32,
    inbound_tx: mpsc::UnboundedSender<(CustomAddr, Vec<u8>)>,
) -> ClientHandle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(actor_loop(reader, writer, cmd_rx, inbound_tx));
    ClientHandle { cmd_tx, rc_id }
}

async fn actor_loop(
    mut reader: BufReader<OwnedReadHalf>,
    mut writer: OwnedWriteHalf,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    inbound_tx: mpsc::UnboundedSender<(CustomAddr, Vec<u8>)>,
) {
    // 1/2 were used by the handshake (connect_and_handshake) before this
    // actor existed -- start well clear of those.
    let mut next_id: u32 = 100;
    let mut pending: HashMap<u32, oneshot::Sender<io::Result<Value>>> = HashMap::new();
    let mut line_buf = String::new();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(Command::Send { mut body, reply }) = cmd else {
                    // Every ClientHandle dropped -- nothing left to serve.
                    return;
                };
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                body["id"] = serde_json::json!(id);
                let mut line = match serde_json::to_string(&body) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = reply.send(Err(io::Error::other(format!("veilid-transport: failed to encode request: {e}"))));
                        continue;
                    }
                };
                line.push('\n');
                if let Err(e) = writer.write_all(line.as_bytes()).await {
                    let _ = reply.send(Err(io::Error::other(format!("veilid-transport: write failed: {e}"))));
                    // VEILID-019 adds reconnect; for now a write failure ends the actor.
                    return;
                }
                pending.insert(id, reply);
            }
            read_result = reader.read_line(&mut line_buf) => {
                match read_result {
                    Ok(0) => return, // EOF -- VEILID-019 adds reconnect.
                    Ok(_) => {
                        let trimmed = line_buf.trim();
                        if !trimmed.is_empty() {
                            handle_incoming_line(trimmed, &mut pending, &inbound_tx);
                        }
                        line_buf.clear();
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "veilid-transport: read failed, connection actor stopping");
                        return; // VEILID-019 adds reconnect.
                    }
                }
            }
        }
    }
}

fn handle_incoming_line(
    line: &str,
    pending: &mut HashMap<u32, oneshot::Sender<io::Result<Value>>>,
    inbound_tx: &mpsc::UnboundedSender<(CustomAddr, Vec<u8>)>,
) {
    let incoming: Incoming = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, line, "veilid-transport: unparseable line, skipping");
            return;
        }
    };
    match incoming {
        Incoming::Response(r) => {
            if let Some(reply) = pending.remove(&r.id) {
                let result = match r.error {
                    Some(err) => Err(io::Error::other(format!(
                        "veilid-transport: daemon returned an error: {}",
                        err.message
                    ))),
                    None => Ok(r.value),
                };
                let _ = reply.send(result);
            } else {
                tracing::debug!(
                    id = r.id,
                    "veilid-transport: response with no matching pending request, dropping"
                );
            }
        }
        Incoming::Update(u) if u.kind == "AppMessage" => {
            let (Some(sender), Some(message_b64)) = (u.sender, u.message) else {
                tracing::debug!(
                    "veilid-transport: AppMessage received with no sender (routed anonymously), dropping -- this transport addresses by NodeId only"
                );
                return;
            };
            let node_id = match sender.parse::<NodeId>() {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!(error = %e, sender, "veilid-transport: AppMessage push had an unparseable sender, dropping");
                    return;
                }
            };
            let payload = match wire::b64_decode(&message_b64) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::debug!(error = %e, "veilid-transport: AppMessage push had an undecodable payload, dropping");
                    return;
                }
            };
            tracing::debug!(%node_id, len = payload.len(), "veilid-transport: AppMessage received");
            let _ = inbound_tx.send((node_id_to_custom_addr(&node_id), payload));
        }
        Incoming::Update(_) => {
            // Anything else (Network, Attachment, ...) is not this
            // transport's concern -- ignored, not an error.
        }
    }
}

/// Polls `GetState` via `handle` until this node's own identity resolves
/// (`network.node_ids`'s first entry), updating `node_id_tx` the moment it
/// does. Bounded the same way the embedded design's own identity watcher
/// was -- a node that never resolves an identity doesn't leak a task that
/// polls forever.
pub(crate) async fn identity_poll_loop(
    handle: ClientHandle,
    node_id_tx: watch::Sender<Option<NodeId>>,
) {
    let start = std::time::Instant::now();
    for _ in 0..IDENTITY_POLL_ATTEMPTS {
        match handle.call(wire::req_get_state(0)).await {
            Ok(value) => {
                if let Some(id_str) = wire::extract_node_ids(&value).into_iter().next() {
                    match id_str.parse::<NodeId>() {
                        Ok(node_id) => {
                            tracing::info!(%node_id, elapsed = ?start.elapsed(), "veilid-transport: own identity resolved");
                            node_id_tx.send_replace(Some(node_id));
                            return;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, raw = id_str, "veilid-transport: daemon reported an unparseable own NodeId");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "veilid-transport: GetState failed while awaiting identity");
                return;
            }
        }
        tokio::time::sleep(IDENTITY_POLL_INTERVAL).await;
    }
    tracing::warn!(elapsed = ?start.elapsed(), "veilid-transport: own identity did not resolve within the poll budget");
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A scripted loopback TCP server for testing the client above without
    //! a real `tetron-veilid` instance. No mock-server pattern existed
    //! anywhere in this repo before VEILID-017 -- built from scratch.
    use std::net::SocketAddr;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// One step of a scripted mock daemon's behavior.
    ///
    /// `Close` is unused until VEILID-019's own disconnect tests -- lands
    /// correctly staged here since the harness itself is written once, in
    /// VEILID-017.
    #[allow(dead_code)]
    pub(crate) enum Step {
        /// Read one request line (ignored content) and write back `reply`
        /// verbatim (a complete JSON line, no trailing newline needed).
        Reply(serde_json::Value),
        /// Push an unsolicited line with no preceding request.
        Push(serde_json::Value),
        /// Close the connection immediately.
        Close,
    }

    /// Binds a random loopback port, accepts exactly one connection, and
    /// plays `script` against it in order. Returns the bound address.
    pub(crate) async fn spawn_scripted_daemon(script: Vec<Step>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock daemon");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => return,
            };
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            for step in script {
                match step {
                    Step::Reply(value) => {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let mut out = serde_json::to_string(&value).unwrap();
                        out.push('\n');
                        if write_half.write_all(out.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Step::Push(value) => {
                        let mut out = serde_json::to_string(&value).unwrap();
                        out.push('\n');
                        if write_half.write_all(out.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Step::Close => return,
                }
            }
            // Keep the connection open (don't drop it) until the test itself
            // finishes, so a caller doing more reads after the script ends
            // sees a live-but-quiet connection rather than an immediate EOF.
            std::future::pending::<()>().await;
        });
        addr
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{Step, spawn_scripted_daemon};
    use super::*;
    use tokio::sync::watch;

    fn ok_response(id: u32, value: Value) -> Value {
        serde_json::json!({"type": "Response", "id": id, "value": value})
    }

    #[tokio::test]
    async fn handshake_resolves_routing_context_id() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))), // NewRoutingContext
            Step::Reply(ok_response(2, serde_json::json!(2))), // WithSafety
        ])
        .await;

        let (_reader, _writer, unsafe_rc_id) =
            connect_and_handshake(addr).await.expect("handshake");
        assert_eq!(unsafe_rc_id, 2);
    }

    #[tokio::test]
    async fn handshake_propagates_daemon_error() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(serde_json::json!({
                "type": "Response", "id": 2,
                "error": {"kind": "Generic", "message": "Unsafe routing mode is not allowed without the 'footgun-nodeid-target' feature enabled"}
            })),
        ])
        .await;

        let err = connect_and_handshake(addr).await.expect_err("should fail");
        assert!(err.to_string().contains("footgun-nodeid-target"));
    }

    #[tokio::test]
    async fn connect_failure_is_a_clean_error() {
        // Nothing listening on this port.
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(connect_and_handshake(addr).await.is_err());
    }

    /// Every test needs an inbound channel of this exact shape -- a type
    /// alias keeps call sites terse without tripping clippy's
    /// `type_complexity` on a bare tuple-of-generics return type.
    type InboundChannel = (
        mpsc::UnboundedSender<(CustomAddr, Vec<u8>)>,
        mpsc::UnboundedReceiver<(CustomAddr, Vec<u8>)>,
    );

    fn unbounded_inbound() -> InboundChannel {
        mpsc::unbounded_channel()
    }

    #[tokio::test]
    async fn identity_poll_loop_resolves_from_get_state() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            // First GetState: no identity yet.
            Step::Reply(ok_response(100, serde_json::json!({"network": {"node_ids": []}}))),
            // Second GetState: identity resolved.
            Step::Reply(ok_response(
                101,
                serde_json::json!({"network": {"node_ids": ["VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo"]}}),
            )),
        ])
        .await;

        let (reader, writer, rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (inbound_tx, _inbound_rx) = unbounded_inbound();
        let handle = spawn_actor(reader, writer, rc_id, inbound_tx);
        let (tx, mut rx) = watch::channel(None);
        tokio::spawn(identity_poll_loop(handle, tx));

        let node_id = loop {
            if let Some(id) = rx.borrow_and_update().clone() {
                break id;
            }
            rx.changed().await.expect("watcher still running");
        };
        assert_eq!(
            node_id.to_string(),
            "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo"
        );
    }

    #[tokio::test]
    async fn app_message_send_reaches_the_daemon_well_formed() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            // The actor's first request after handshake is id 100.
            Step::Reply(ok_response(100, Value::Null)),
        ])
        .await;

        let (reader, writer, rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (inbound_tx, _inbound_rx) = unbounded_inbound();
        let handle = spawn_actor(reader, writer, rc_id, inbound_tx);

        let target = "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo";
        let result = handle
            .call(wire::req_app_message(0, rc_id, target, b"hello"))
            .await;
        assert!(
            result.is_ok(),
            "expected the mock's scripted Ok reply: {result:?}"
        );
    }

    #[tokio::test]
    async fn app_message_push_is_delivered_to_inbound_channel() {
        let target = "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo";
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            Step::Push(serde_json::json!({
                "type": "Update", "kind": "AppMessage",
                "sender": target, "route_id": null,
                "message": wire::b64_encode(b"hello from the mock"),
            })),
        ])
        .await;

        let (reader, writer, rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (inbound_tx, mut inbound_rx) = unbounded_inbound();
        let _handle = spawn_actor(reader, writer, rc_id, inbound_tx);

        let (from, payload) = inbound_rx.recv().await.expect("push delivered");
        assert_eq!(from, node_id_to_custom_addr(&target.parse().unwrap()));
        assert_eq!(payload, b"hello from the mock");
    }

    #[tokio::test]
    async fn app_message_push_with_no_sender_is_dropped() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            Step::Push(serde_json::json!({
                "type": "Update", "kind": "AppMessage",
                "sender": null, "route_id": null,
                "message": wire::b64_encode(b"anonymous"),
            })),
            // A second, real push proves the actor kept running past the
            // dropped one rather than getting stuck on it.
            Step::Push(serde_json::json!({
                "type": "Update", "kind": "AppMessage",
                "sender": "VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo", "route_id": null,
                "message": wire::b64_encode(b"named sender"),
            })),
        ])
        .await;

        let (reader, writer, rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (inbound_tx, mut inbound_rx) = unbounded_inbound();
        let _handle = spawn_actor(reader, writer, rc_id, inbound_tx);

        let (_from, payload) = inbound_rx
            .recv()
            .await
            .expect("the named-sender push should arrive");
        assert_eq!(payload, b"named sender");
    }
}
