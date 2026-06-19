use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::Connection, endpoint::presets};

pub const DEFAULT_ALPN: &[u8] = b"pitopi/net/0";

pub fn network_alpn(network_name: &str) -> Vec<u8> {
    format!("pitopi/net/{network_name}").into_bytes()
}

pub async fn create_endpoint(secret_key: SecretKey) -> Result<Endpoint> {
    let ep = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(vec![DEFAULT_ALPN.to_vec()])
        .bind()
        .await
        .context("failed to bind iroh endpoint")?;

    tracing::info!(id = %ep.id().fmt_short(), "iroh endpoint ready");

    Ok(ep)
}


pub async fn accept_connection(ep: &Endpoint) -> Result<Connection> {
    let incoming = ep.accept().await.context("no incoming connection")?;
    let conn = incoming.await.context("failed to accept connection")?;
    tracing::info!(peer = %conn.remote_id().fmt_short(), "peer connected");
    Ok(conn)
}

pub async fn connect_to_peer(ep: &Endpoint, id: EndpointId) -> Result<Connection> {
    let addr: EndpointAddr = id.into();
    let conn = ep
        .connect(addr, DEFAULT_ALPN)
        .await
        .context("failed to connect to peer")?;
    tracing::info!(peer = %conn.remote_id().fmt_short(), "connected to peer");
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
