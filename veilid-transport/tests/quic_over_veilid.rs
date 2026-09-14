//! Integration test for `VEILID-001`: does a real `iroh` QUIC connection
//! actually come up and carry data over this crate's `CustomTransport`,
//! with no IP/relay path available at all?
//!
//! `#[ignore]`d by default -- needs a real, reachable `tetron-veilid`
//! daemon (VEILID-017) attached to Veilid's public bootstrap network,
//! which is not available in every CI/sandbox environment. Run explicitly:
//!
//! ```text
//! cargo test -p veilid-transport -- --ignored --nocapture
//! ```
//!
//! **Not yet re-verified against the external-daemon architecture**
//! (VEILID-017/018/019, 2026-09-14): this test's body still assumes two
//! independent transport instances the way two independently-embedded
//! Veilid nodes worked pre-rewrite. Under the external-daemon design both
//! `VeilidTransportBuilder::build()` calls below connect to the *same*
//! local `tetron-veilid` daemon and therefore resolve to the *same*
//! Veilid identity -- this test needs a real rework (either accept that
//! and test a single-identity loopback echo, or stand up two separate
//! `tetron-veilid` instances on two different ports for a true two-node
//! test) once VEILID-018's data path lands. `.namespace(...)` is removed
//! here only so this file compiles; the test's own logic is not yet
//! meaningful again.

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
#[ignore = "needs a real tetron-veilid daemon reachable from Veilid's public bootstrap network; not yet reworked for the external-daemon architecture, see module docs"]
async fn quic_connection_over_veilid_only() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let transport_a = timeout(
        Duration::from_secs(180),
        VeilidTransportBuilder::new().build(),
    )
    .await??;
    let transport_b = timeout(
        Duration::from_secs(180),
        VeilidTransportBuilder::new().build(),
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
