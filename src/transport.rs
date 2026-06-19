use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::Connection, endpoint::presets};

pub fn network_alpn(network_name: &str) -> Vec<u8> {
    format!("pitopi/net/{network_name}").into_bytes()
}

pub async fn create_endpoint_with_alpns(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
) -> Result<Endpoint> {
    let ep = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(alpns)
        .bind()
        .await
        .context("failed to bind iroh endpoint")?;

    tracing::info!(id = %ep.id().fmt_short(), "iroh endpoint ready");

    Ok(ep)
}

pub async fn accept_connection_with_alpn(ep: &Endpoint) -> Result<(Connection, Vec<u8>)> {
    let incoming = ep.accept().await.context("no incoming connection")?;
    let conn = incoming.await.context("failed to accept connection")?;
    let alpn = conn.alpn().to_vec();
    tracing::info!(
        peer = %conn.remote_id().fmt_short(),
        alpn = %String::from_utf8_lossy(&alpn),
        "peer connected"
    );
    Ok((conn, alpn))
}

pub async fn connect_to_peer_with_alpn(
    ep: &Endpoint,
    id: EndpointId,
    alpn: &[u8],
) -> Result<Connection> {
    let addr: EndpointAddr = id.into();
    let conn = ep
        .connect(addr, alpn)
        .await
        .context("failed to connect to peer")?;
    tracing::info!(
        peer = %conn.remote_id().fmt_short(),
        alpn = %String::from_utf8_lossy(alpn),
        "connected to peer"
    );
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_alpn() {
        assert_eq!(network_alpn("gaming"), b"pitopi/net/gaming");
        assert_eq!(network_alpn("default"), b"pitopi/net/default");
    }
}
