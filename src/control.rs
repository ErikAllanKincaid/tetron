use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub ip: Ipv4Addr,
    pub endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    Welcome {
        your_ip: Ipv4Addr,
        peers: Vec<PeerInfo>,
    },
    PeerJoined(PeerInfo),
    PeerLeft {
        ip: Ipv4Addr,
    },
    MeshHello {
        ip: Ipv4Addr,
    },
    MeshWelcome {
        ip: Ipv4Addr,
    },
}

pub fn encode_msg(msg: &ControlMsg) -> Vec<u8> {
    let json = serde_json::to_vec(msg).expect("serialize control message");
    let len = (json.len() as u32).to_be_bytes();
    [len.as_slice(), &json].concat()
}

pub fn decode_msg(data: &[u8]) -> Result<ControlMsg> {
    anyhow::ensure!(data.len() >= 4, "message too short");
    let len = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
    anyhow::ensure!(data.len() >= 4 + len, "incomplete message");
    serde_json::from_slice(&data[4..4 + len]).context("invalid control message")
}

pub async fn send_msg(stream: &mut SendStream, msg: &ControlMsg) -> Result<()> {
    let data = encode_msg(msg);
    stream.write_all(&data).await.context("send control message")?;
    Ok(())
}

pub async fn recv_msg(stream: &mut RecvStream) -> Result<ControlMsg> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .context("read message length")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 65536, "control message too large");
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .context("read message body")?;
    serde_json::from_slice(&body).context("decode control message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_welcome() {
        let msg = ControlMsg::Welcome {
            your_ip: Ipv4Addr::new(100, 64, 0, 3),
            peers: vec![PeerInfo {
                ip: Ipv4Addr::new(100, 64, 0, 2),
                endpoint_id: "test-id-abc123".to_string(),
            }],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_peer_joined() {
        let msg = ControlMsg::PeerJoined(PeerInfo {
            ip: Ipv4Addr::new(100, 64, 0, 5),
            endpoint_id: "node-xyz".to_string(),
        });
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_mesh_hello() {
        let msg = ControlMsg::MeshHello {
            ip: Ipv4Addr::new(100, 64, 0, 4),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }
}
