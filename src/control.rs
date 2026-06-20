use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

use crate::membership::{ApprovedEntry, Member};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    JoinApproved {
        your_ip: Ipv4Addr,
        members: Vec<Member>,
    },
    JoinDenied {
        reason: String,
    },
    MemberSync {
        members: Vec<Member>,
    },
    ReconnectRequest {
        identity: String,
        ip: Ipv4Addr,
    },
    MeshHello {
        identity: String,
        ip: Ipv4Addr,
    },
    MeshWelcome {
        identity: String,
        ip: Ipv4Addr,
    },
    AdvertiseServices {
        ip: Ipv4Addr,
        services: Vec<ServiceTag>,
    },
    MemberApproved {
        identity: String,
        ip: Ipv4Addr,
    },
    Welcome {
        members: Vec<Member>,
        approved: Vec<ApprovedEntry>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTag {
    pub name: String,
    pub port: u16,
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
    stream
        .write_all(&data)
        .await
        .context("send control message")?;
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
    fn test_roundtrip_join_approved_basic() {
        let msg = ControlMsg::JoinApproved {
            your_ip: Ipv4Addr::new(100, 64, 0, 3),
            members: vec![Member {
                identity: "test-id-abc123".to_string(),
                ip: Ipv4Addr::new(100, 64, 0, 2),
                is_coordinator: true,
            }],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_mesh_hello_basic() {
        let msg = ControlMsg::MeshHello {
            identity: "peer-abc".to_string(),
            ip: Ipv4Addr::new(100, 64, 0, 4),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_join_approved() {
        let msg = ControlMsg::JoinApproved {
            your_ip: Ipv4Addr::new(100, 64, 10, 5),
            members: vec![Member {
                identity: "coord-id".to_string(),
                ip: Ipv4Addr::new(100, 64, 5, 3),
                is_coordinator: true,
            }],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_join_denied() {
        let msg = ControlMsg::JoinDenied {
            reason: "not authorized".to_string(),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_member_sync() {
        let msg = ControlMsg::MemberSync {
            members: vec![
                Member {
                    identity: "a".to_string(),
                    ip: Ipv4Addr::new(100, 64, 0, 2),
                    is_coordinator: true,
                },
                Member {
                    identity: "b".to_string(),
                    ip: Ipv4Addr::new(100, 64, 0, 3),
                    is_coordinator: false,
                },
            ],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_reconnect_request() {
        let msg = ControlMsg::ReconnectRequest {
            identity: "returning-peer".to_string(),
            ip: Ipv4Addr::new(100, 64, 7, 42),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_mesh_hello_with_identity() {
        let msg = ControlMsg::MeshHello {
            identity: "peer-xyz".to_string(),
            ip: Ipv4Addr::new(100, 64, 0, 4),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_member_approved() {
        let msg = ControlMsg::MemberApproved {
            identity: "new-peer-xyz".to_string(),
            ip: Ipv4Addr::new(100, 64, 12, 34),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_welcome() {
        use crate::membership::ApprovedEntry;
        let msg = ControlMsg::Welcome {
            members: vec![Member {
                identity: "coord".to_string(),
                ip: Ipv4Addr::new(100, 64, 0, 2),
                is_coordinator: true,
            }],
            approved: vec![ApprovedEntry {
                identity: "pending-peer".to_string(),
                ip: Ipv4Addr::new(100, 64, 0, 5),
            }],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_welcome_empty_approved() {
        let msg = ControlMsg::Welcome {
            members: vec![Member {
                identity: "a".to_string(),
                ip: Ipv4Addr::new(100, 64, 0, 2),
                is_coordinator: true,
            }],
            approved: vec![],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }
}
