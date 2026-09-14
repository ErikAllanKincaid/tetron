//! The connection actor: owns the TCP socket to `tetron-veilid`'s
//! `client_api`, performs the initial handshake, and resolves this node's
//! own identity. See `spec/core.py`'s `VeilidExternalDaemonProtocol`
//! (VEILID-017).

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::sync::watch;

use crate::NodeId;
use crate::wire::{self, Incoming};

/// `veilid-server`'s own upstream-documented default
/// (`veilid-server/src/settings.rs`'s `client_api.listen_address`) --
/// `tetron-veilid` adopts it rather than inventing a tetron-specific port,
/// matching how `iroh-tor-transport` hardcodes Tor's own ControlPort 9051.
pub(crate) const DEFAULT_DAEMON_ADDR: &str = "127.0.0.1:5959";

/// How many times [`spawn_identity_watcher`] polls `GetState` before
/// giving up, and the delay between attempts -- mirrors the embedded
/// design's own bounded background-resolution contract
/// (`spawn_identity_and_attach_watcher`, up to 5 minutes at 500ms
/// intervals).
const IDENTITY_POLL_ATTEMPTS: usize = 600;
const IDENTITY_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A single request/response round trip against a fresh connection --
/// used only during the handshake sequence (`NewRoutingContext` ->
/// `WithSafety` -> `GetState`), before the general-purpose actor/command
/// channel exists (VEILID-018 introduces that for ongoing sends).
async fn call_once(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    request: serde_json::Value,
) -> io::Result<serde_json::Value> {
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
/// `Unsafe`-mode routing context id, ready for `poll_send`/reconnect logic
/// (VEILID-018/019) to use.
pub(crate) async fn connect_and_handshake(
    addr: SocketAddr,
) -> io::Result<(
    BufReader<OwnedReadHalf>,
    tokio::net::tcp::OwnedWriteHalf,
    u32,
)> {
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

/// Polls `GetState` on an already-handshaken connection until this node's
/// own identity resolves (`network.node_ids`'s first entry), updating
/// `node_id_tx` the moment it does. Bounded the same way the embedded
/// design's own identity watcher was -- a node that never resolves an
/// identity doesn't leak a task that polls forever.
pub(crate) async fn spawn_identity_watcher(
    mut reader: BufReader<OwnedReadHalf>,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    node_id_tx: watch::Sender<Option<NodeId>>,
) {
    let start = std::time::Instant::now();
    let mut next_id: u64 = 100;
    for _ in 0..IDENTITY_POLL_ATTEMPTS {
        next_id += 1;
        match call_once(
            &mut reader,
            &mut writer,
            wire::req_get_state(next_id as u32),
        )
        .await
        {
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
    /// `Push`/`Close` are unused until VEILID-018/019's own tests need a
    /// mid-script unsolicited push or disconnect -- lands correctly staged
    /// here since the harness itself is written once, in VEILID-017.
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

    fn ok_response(id: u32, value: serde_json::Value) -> serde_json::Value {
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

    #[tokio::test]
    async fn identity_watcher_resolves_from_get_state() {
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
            // First GetState: no identity yet.
            Step::Reply(ok_response(101, serde_json::json!({"network": {"node_ids": []}}))),
            // Second GetState: identity resolved.
            Step::Reply(ok_response(
                102,
                serde_json::json!({"network": {"node_ids": ["VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo"]}}),
            )),
        ])
        .await;

        let (reader, writer, _rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (tx, mut rx) = watch::channel(None);
        tokio::spawn(spawn_identity_watcher(reader, writer, tx));

        // wait_for_own_node_id-equivalent: wait for the watch to change.
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
    async fn build_does_not_block_on_identity_resolution() {
        // The mock never sends a GetState reply -- if anything here waited
        // synchronously on identity, this test would hang and time out.
        let addr = spawn_scripted_daemon(vec![
            Step::Reply(ok_response(1, serde_json::json!(1))),
            Step::Reply(ok_response(2, serde_json::json!(2))),
        ])
        .await;

        let (reader, writer, _rc_id) = connect_and_handshake(addr).await.expect("handshake");
        let (tx, rx) = watch::channel(None);
        tokio::spawn(spawn_identity_watcher(reader, writer, tx));
        // No await on identity here at all -- reaching this point without
        // hanging is the assertion.
        assert!(rx.borrow().is_none());
    }
}
