//! Integration test for `VEILID-001`: does a real `iroh` QUIC connection
//! actually come up and carry data over this crate's `CustomTransport`,
//! with no IP/relay path available at all?
//!
//! `#[ignore]`d by default -- both nodes must attach to Veilid's public
//! bootstrap network (`bootstrap-v1.veilid.net`), which is not reachable
//! from every CI/sandbox environment (confirmed blocked inside Claude
//! Code's own Bash tool while writing this crate, 2026-09-11 -- a plain
//! `curl` to an unrelated host on port 443 timed out the same way, so this
//! is an environment restriction, not a bug in this test). Run explicitly:
//!
//! ```text
//! cargo test -p veilid-transport -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{EndpointAddr, RelayMode, TransportAddr};
use tokio::time::timeout;
use veilid_transport::VeilidTransportBuilder;

const ECHO_ALPN: &[u8] = b"veilid-transport-test/echo";

#[derive(Debug, Clone)]
struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        tokio::io::copy(&mut recv, &mut send).await?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

#[tokio::test]
#[ignore = "needs Veilid's public bootstrap network; see module docs"]
async fn quic_connection_over_veilid_only() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let transport_a = timeout(
        Duration::from_secs(180),
        VeilidTransportBuilder::new()
            .namespace("veilid-transport-test-a")
            .build(),
    )
    .await??;
    let transport_b = timeout(
        Duration::from_secs(180),
        VeilidTransportBuilder::new()
            .namespace("veilid-transport-test-b")
            .build(),
    )
    .await??;
    eprintln!(
        "both veilid nodes started in {:?} (attachment continues in the background -- build() no longer blocks on it, see its doc comment)",
        t0.elapsed()
    );

    let ep_a = iroh::Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .add_custom_transport(std::sync::Arc::new(transport_a.clone()))
        .bind()
        .await?;
    let ep_b = iroh::Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .add_custom_transport(std::sync::Arc::new(transport_b.clone()))
        .bind()
        .await?;

    let router = Router::builder(ep_b.clone())
        .accept(ECHO_ALPN, Echo)
        .spawn();

    // own_addr() resolves in the background (identity is coupled to
    // attachment progress, not available synchronously -- see
    // VeilidTransportBuilder::build's doc comment), so poll for it.
    let t_addr = Instant::now();
    let b_addr = loop {
        if let Some(addr) = transport_b.own_addr() {
            break addr;
        }
        if t_addr.elapsed() > Duration::from_secs(240) {
            anyhow::bail!("transport_b's own_addr() never resolved within 240s");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    eprintln!(
        "transport_b's own_addr() resolved in {:?}",
        t_addr.elapsed()
    );

    let dst = EndpointAddr::from_parts(ep_b.id(), std::iter::once(TransportAddr::Custom(b_addr)));

    let t1 = Instant::now();
    // Generous: build() no longer waits for attachment (see its doc
    // comment), so this connect attempt races real attachment happening in
    // the background on both sides, which has taken up to ~2 minutes in
    // manual testing.
    let conn = timeout(Duration::from_secs(240), ep_a.connect(dst, ECHO_ALPN)).await??;
    eprintln!(
        "QUIC connection established over veilid-transport in {:?}",
        t1.elapsed()
    );

    let t2 = Instant::now();
    let msg = b"hello over veilid QUIC";
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(msg).await?;
    send.finish()?;
    let response = recv.read_to_end(1024).await?;
    eprintln!("echo round trip in {:?}", t2.elapsed());
    assert_eq!(response, msg);

    conn.close(0u32.into(), b"done");
    router.shutdown().await?;
    transport_a.shutdown().await;
    transport_b.shutdown().await;
    Ok(())
}
