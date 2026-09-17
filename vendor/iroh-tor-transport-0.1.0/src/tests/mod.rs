//! Internal tests for packet protocol and sender.

mod user_transport;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use iroh::SecretKey;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

use crate::{
    TorPacket, TorPacketSender, TorPacketService, TorStreamIo, hs_desc_settle_should_stop,
    hs_desc_wait_should_stop, iroh_to_tor_secret_key, read_tor_packet, write_tor_packet,
};

/// Get the onion address for an iroh SecretKey (test helper).
fn onion_address(key: &SecretKey) -> torut::onion::OnionAddressV3 {
    let tor_key = iroh_to_tor_secret_key(key);
    tor_key.public().get_onion_address()
}

#[tokio::test]
async fn test_packet_service_roundtrip() -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<TorPacket>(1);
    let service = TorPacketService::new(tx);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _addr) = listener.accept().await?;
        service.handle_stream(stream).await
    });

    let mut client = TcpStream::connect(addr).await?;
    let from = SecretKey::generate().public();
    let packet = TorPacket {
        from,
        data: Bytes::from_static(b"hello-tor-packet"),
        segment_size: Some(512),
    };
    write_tor_packet(&mut client, &packet).await?;

    // The service should deliver the packet quickly.
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .map_err(|_| anyhow!("Timed out waiting for packet"))?
        .ok_or_else(|| anyhow!("Packet channel closed"))?;

    assert_eq!(received.from, packet.from);
    assert_eq!(received.data, packet.data);
    assert_eq!(received.segment_size, packet.segment_size);

    drop(client);

    server.await??;
    Ok(())
}

#[tokio::test]
async fn test_sender_reuses_connection() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut stream, _peer) = listener.accept().await?;
        let mut packets = Vec::new();
        if let Some(packet) = read_tor_packet(&mut stream).await? {
            packets.push(packet);
        }
        if let Some(packet) = read_tor_packet(&mut stream).await? {
            packets.push(packet);
        }
        // Ensure no second connection is opened.
        let accept_timeout =
            tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accept_timeout.is_err());
        Ok::<_, anyhow::Error>(packets)
    });

    let connects = Arc::new(AtomicUsize::new(0));
    let io = Arc::new(TorStreamIo::new(
        || async { Err(std::io::Error::other("accept not used in this test")) },
        {
            let connects = connects.clone();
            move |_endpoint| {
                let connects = connects.clone();
                async move {
                    connects.fetch_add(1, Ordering::SeqCst);
                    let stream = TcpStream::connect(addr).await?;
                    Ok(stream)
                }
            }
        },
    ));
    let sender = TorPacketSender::new(io);

    let to = SecretKey::generate().public();
    let from = SecretKey::generate().public();
    let packet1 = TorPacket {
        from,
        data: Bytes::from_static(b"one"),
        segment_size: Some(32),
    };
    let packet2 = TorPacket {
        from,
        data: Bytes::from_static(b"two"),
        segment_size: None,
    };

    sender.send(to, &packet1).await?;
    sender.send(to, &packet2).await?;

    let packets = server.await??;
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0], packet1);
    assert_eq!(packets[1], packet2);
    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "expected single connection reuse"
    );

    Ok(())
}

#[test]
fn test_key_conversion() {
    // Generate an iroh key
    let iroh_key = SecretKey::generate();

    // Convert to tor key
    let tor_key = iroh_to_tor_secret_key(&iroh_key);

    // The public keys should match
    let iroh_public = iroh_key.public();
    let tor_public = tor_key.public();

    // iroh public key is 32 bytes, tor public key is also 32 bytes
    assert_eq!(iroh_public.as_bytes(), tor_public.as_bytes());
}

#[test]
fn test_onion_address_deterministic() {
    let iroh_key = SecretKey::generate();

    let addr1 = onion_address(&iroh_key);
    let addr2 = onion_address(&iroh_key);

    assert_eq!(addr1.to_string(), addr2.to_string());
}

#[test]
fn test_onion_address_methods_match() {
    use crate::onion_address_from_endpoint;

    let iroh_key = SecretKey::generate();
    let endpoint_id = iroh_key.public();

    // Method 1: Via secret key conversion (used by builder to create hidden service)
    let addr_via_secret = onion_address(&iroh_key);

    // Method 2: Via endpoint ID (used by connect to derive target address)
    let addr_via_endpoint = onion_address_from_endpoint(endpoint_id).unwrap();

    assert_eq!(
        addr_via_secret.to_string(),
        addr_via_endpoint.to_string(),
        "Onion addresses derived from secret key and endpoint ID must match!"
    );
}

// tetron-local patch (PATCH.md, Patch 3, TOR-DIAL-001 follow-up): pure
// unit tests for the HS_DESC publish-wait quorum decision (Fix 6,
// tetron/spec/core.py's TorDialPathWiring) -- no live Tor connection
// needed, mirroring tetron core's own path_flap_decision test pattern.

#[test]
fn test_hs_desc_wait_keeps_waiting_below_quorum_before_deadline() {
    assert!(!hs_desc_wait_should_stop(1, 8, false));
    assert!(!hs_desc_wait_should_stop(7, 8, false));
}

#[test]
fn test_hs_desc_wait_stops_at_quorum() {
    assert!(hs_desc_wait_should_stop(8, 8, false));
    assert!(hs_desc_wait_should_stop(9, 8, false));
}

#[test]
fn test_hs_desc_wait_stops_at_deadline_even_below_quorum() {
    assert!(hs_desc_wait_should_stop(0, 8, true));
    assert!(hs_desc_wait_should_stop(3, 8, true));
}

#[test]
fn test_hs_desc_wait_zero_confirmations_before_deadline_keeps_waiting() {
    assert!(!hs_desc_wait_should_stop(0, 8, false));
}

// tetron-local patch (PATCH.md, Patch 5, TOR-DIAL-001 follow-up): pure unit
// tests for the post-quorum settle-wait decision (Fix 11,
// tetron/spec/core.py's TorDialPathWiring) -- reaching quorum alone was
// live-verified insufficient (0xf2 / INTRODUCE_ACK Reason 1), so `build()`
// now also waits for confirmations to stop increasing before trusting the
// descriptor.

#[test]
fn test_hs_desc_settle_keeps_waiting_while_still_incrementing() {
    assert!(!hs_desc_settle_should_stop(0.0, false));
    assert!(!hs_desc_settle_should_stop(5.0, false));
    assert!(!hs_desc_settle_should_stop(19.9, false));
}

#[test]
fn test_hs_desc_settle_stops_once_quiet_for_the_settle_window() {
    assert!(hs_desc_settle_should_stop(20.0, false));
    assert!(hs_desc_settle_should_stop(45.0, false));
}

#[test]
fn test_hs_desc_settle_stops_at_deadline_even_if_still_incrementing() {
    assert!(hs_desc_settle_should_stop(0.0, true));
    assert!(hs_desc_settle_should_stop(5.0, true));
}

// tetron-local patch (PATCH.md, Patch 4, TOR-DIAL-001 follow-up): concurrent
// sends to the same peer must not race independent SOCKS5 connects (Fix 8,
// tetron/spec/core.py's TorDialPathWiring) -- each concurrent `send()` call
// arriving before a prior connect for the same peer finishes must share
// that one in-flight attempt, not each kick off its own.
#[tokio::test]
async fn test_sender_dedupes_concurrent_connects_to_same_peer() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Accepts every incoming connection and holds it open in `held` (this
    // test only cares how many connect ATTEMPTS were made, not the data
    // path -- an accepted stream immediately dropped would close its
    // client-side counterpart, failing the client's write with a broken
    // pipe before the assertion is ever reached).
    let server = tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => held.push(stream),
                Err(_) => return,
            }
        }
    });

    let connects = Arc::new(AtomicUsize::new(0));
    // Gates the connector so every concurrent `send()` call is guaranteed to
    // arrive at `get_or_connect` before the first connect finishes -- a
    // connector that resolves instantly could let calls race in a way that
    // happens not to overlap, defeating the point of the test.
    let (release_tx, _release_rx) = tokio::sync::broadcast::channel::<()>(1);
    let io = Arc::new(TorStreamIo::new(
        || async { Err(std::io::Error::other("accept not used in this test")) },
        {
            let connects = connects.clone();
            let release_tx = release_tx.clone();
            move |_endpoint| {
                let connects = connects.clone();
                let mut release_rx = release_tx.subscribe();
                async move {
                    connects.fetch_add(1, Ordering::SeqCst);
                    let _ = release_rx.recv().await;
                    let stream = TcpStream::connect(addr).await?;
                    Ok(stream)
                }
            }
        },
    ));
    let sender = Arc::new(TorPacketSender::new(io));

    let to = SecretKey::generate().public();
    let from = SecretKey::generate().public();
    let packet = TorPacket {
        from,
        data: Bytes::from_static(b"concurrent"),
        segment_size: None,
    };

    let mut sends = tokio::task::JoinSet::new();
    for _ in 0..5 {
        let sender = sender.clone();
        let packet = packet.clone();
        sends.spawn(async move { sender.send(to, &packet).await });
    }

    // Give every spawned send a chance to reach (and block inside)
    // `get_or_connect` before releasing the connector -- if de-duplication
    // were missing, this is the window where each would have already
    // kicked off its own independent connect.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = release_tx.send(());

    while let Some(res) = sends.join_next().await {
        res??;
    }

    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "5 concurrent sends to the same peer should share one connect attempt"
    );

    server.abort();
    Ok(())
}
