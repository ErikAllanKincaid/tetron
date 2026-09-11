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
    eprintln!("both veilid nodes attached in {:?}", t0.elapsed());

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

    let dst = EndpointAddr::from_parts(
        ep_b.id(),
        std::iter::once(TransportAddr::Custom(transport_b.own_addr())),
    );

    let t1 = Instant::now();
    let conn = timeout(Duration::from_secs(60), ep_a.connect(dst, ECHO_ALPN)).await??;
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
