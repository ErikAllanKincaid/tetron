use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::membership::GroupMode;

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    Create {
        name: String,
        mode: GroupMode,
    },
    Join {
        node_id: String,
        name: Option<String>,
    },
    Leave {
        name: String,
    },
    Status,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok {
        message: String,
    },
    Error {
        message: String,
    },
    Created {
        name: String,
        room_code: String,
        my_ip: Ipv4Addr,
    },
    Joined {
        name: String,
        my_ip: Ipv4Addr,
    },
    Status {
        endpoint_id: String,
        networks: Vec<NetworkStatus>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub name: String,
    pub role: NetworkRole,
    pub my_ip: Ipv4Addr,
    pub peers: Vec<PeerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkRole {
    Coordinator,
    Member,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerStatus {
    pub endpoint_id: String,
    pub ip: Ipv4Addr,
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not determine config directory")?
        .join("pitopi")
        .join("pitopi.sock"))
}

pub async fn connect() -> Result<UnixStream> {
    let path = socket_path()?;
    UnixStream::connect(&path)
        .await
        .context("daemon not running — start it with: sudo pitopi daemon")
}

pub async fn send_msg<T: Serialize>(stream: &mut UnixStream, msg: &T) -> Result<()> {
    let json = serde_json::to_vec(msg).context("serialize IPC message")?;
    let len = (json.len() as u32).to_be_bytes();
    stream.write_all(&len).await.context("write IPC length")?;
    stream.write_all(&json).await.context("write IPC body")?;
    stream.flush().await.context("flush IPC")?;
    Ok(())
}

pub async fn recv_msg<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .context("read IPC length")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 1_048_576, "IPC message too large");
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .context("read IPC body")?;
    serde_json::from_slice(&body).context("decode IPC message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_roundtrip() {
        let req = IpcRequest::Create {
            name: "gaming".to_string(),
            mode: GroupMode::Open,
        };
        let json = serde_json::to_vec(&req).unwrap();
        let decoded: IpcRequest = serde_json::from_slice(&json).unwrap();
        match decoded {
            IpcRequest::Create { name, mode } => {
                assert_eq!(name, "gaming");
                assert_eq!(mode, GroupMode::Open);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_response_roundtrip() {
        let resp = IpcResponse::Created {
            name: "test".to_string(),
            room_code: "test/abc-def".to_string(),
            my_ip: Ipv4Addr::new(100, 64, 10, 5),
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let decoded: IpcResponse = serde_json::from_slice(&json).unwrap();
        match decoded {
            IpcResponse::Created { name, room_code, my_ip } => {
                assert_eq!(name, "test");
                assert_eq!(room_code, "test/abc-def");
                assert_eq!(my_ip, Ipv4Addr::new(100, 64, 10, 5));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_status_response_roundtrip() {
        let resp = IpcResponse::Status {
            endpoint_id: "abc123".to_string(),
            networks: vec![NetworkStatus {
                name: "gaming".to_string(),
                role: NetworkRole::Coordinator,
                my_ip: Ipv4Addr::new(100, 64, 10, 5),
                peers: vec![PeerStatus {
                    endpoint_id: "def456".to_string(),
                    ip: Ipv4Addr::new(100, 64, 10, 6),
                }],
            }],
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let decoded: IpcResponse = serde_json::from_slice(&json).unwrap();
        match decoded {
            IpcResponse::Status { endpoint_id, networks } => {
                assert_eq!(endpoint_id, "abc123");
                assert_eq!(networks.len(), 1);
                assert_eq!(networks[0].peers.len(), 1);
            }
            _ => panic!("wrong variant"),
        }
    }
}
