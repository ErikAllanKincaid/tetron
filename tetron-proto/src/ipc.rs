use std::marker::PhantomData;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use iroh::EndpointId;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::net::UnixStream;
use tokio_util::codec::{Decoder, Encoder, Framed, LengthDelimitedCodec};

use crate::{GroupMode, TransportMode};

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    // Requests
    Create {
        mode: GroupMode,
        network_name: Option<String>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        /// Overlay IPv4 subnet as a CIDR string (e.g. "10.88.0.0/24"). `None`
        /// uses the default 10.88.0.0/24. Kept as a raw string here so the
        /// wire protocol crate stays free of the main crate's parsing helpers;
        /// the daemon parses/validates it.
        #[serde(default)]
        subnet: Option<String>,
        /// Minimum distinct, unexpired proposers required to execute a nuke
        /// (NUKE-CONSENSUS-THRESHOLD-001) once this network has 2+
        /// coordinators. `None` uses the default of 2. Fixed at creation
        /// time, never mutated afterward.
        #[serde(default)]
        nuke_consensus: Option<u32>,
    },
    /// `network_key` is already resolved to the network's public key by the
    /// time it crosses IPC -- the CLI decodes the raw invite code client-side
    /// (`invite::decode_invite_code`) and sends the key plus the secret
    /// (`invite`) separately. It is not the invite-code text itself.
    Join {
        network_key: String,
        alias: Option<String>,
        hostname: Option<String>,
        transport: Option<TransportMode>,
        /// One-time invite secret to present for invite-gated admission.
        #[serde(default)]
        invite: Option<Vec<u8>>,
    },
    Leave {
        network: String,
        /// Leave even if you are the only coordinator and other members
        /// would be stranded. Defaults to `false` on old wire data
        /// (pre-`STRANDED-COORDINATOR-WARN` clients), preserving the
        /// safer behavior of warning rather than silently leaving.
        #[serde(default)]
        force: bool,
    },
    /// `network_key` is the network's own key (or an unambiguous prefix of
    /// it, as shown by `tetron status`'s `network_key` line) -- never the
    /// local display name, which `MeshManager::resolve_network_short_id`
    /// deliberately does not accept as a fallback.
    Nuke {
        network_key: String,
        force: bool,
        /// Withdraw the caller's own pending nuke proposal (NUKE-CONSENSUS).
        #[serde(default)]
        cancel: bool,
        /// Second a specific proposer's proposal by short id, erroring if it
        /// doesn't match an active one rather than silently proposing fresh.
        #[serde(default)]
        second: Option<String>,
    },
    /// Coordinator-only: remove a member from a closed network. Prunes it from the
    /// roster + approved list, republishes the signed blob, and disconnects it
    /// mesh-wide. `network_key` is the network's own key (see `Nuke`'s doc
    /// comment); `endpoint_id` is the target member's endpoint id (or an
    /// unambiguous prefix of it) -- never a hostname, which
    /// `MeshManager::resolve_short_id_any_network` deliberately does not
    /// accept as a fallback (unlike `AdminAction::Add`'s peer resolution).
    Kick {
        network_key: String,
        endpoint_id: String,
    },
    Status,
    Shutdown,
    /// Activate the VPN: bring the TUN interface up and reconnect all saved
    /// networks. Handled by the already-running daemon, so no root
    /// privileges are needed on the client. An optional `hostname` sets the
    /// personal default hostname used for future creates/joins.
    Resume {
        #[serde(default)]
        hostname: Option<String>,
        /// Resume only this network (by local display name) instead of
        /// every joined network. `None` preserves the original daemon-wide
        /// behavior (STANDBY-PER-NETWORK).
        #[serde(default)]
        network: Option<String>,
    },
    /// Put the daemon on standby: tear down active network connections and
    /// bring the TUN interface down. The daemon process keeps running so it
    /// can be reactivated with `Resume`.
    Standby {
        /// Take only this network (by local display name) offline instead
        /// of every joined network. `None` preserves the original
        /// daemon-wide behavior (STANDBY-PER-NETWORK).
        #[serde(default)]
        network: Option<String>,
    },
    /// Authorize a local user (by UID) to control the daemon without root, the
    /// way `tailscale up --operator` does. Root-only.
    SetOperator {
        uid: u32,
    },
    /// Coordinator-only: grant the per-network secret key to a member, making it
    /// a co-coordinator (can publish / suggest firewall rules).
    AdminAdd {
        network: String,
        peer: String,
    },
    /// List the identities this coordinator has granted the network key to
    /// (plus itself). Open read.
    AdminList {
        network: String,
    },

    // Responses
    Ok {
        message: String,
    },
    Error {
        message: String,
    },
    Created {
        network: String,
        network_key: EndpointId,
        my_ip: Ipv4Addr,
        my_ipv6: Option<Ipv6Addr>,
        /// SUBNET-014: set when the network's subnet only applies after a restart.
        #[serde(default)]
        warning: Option<String>,
        /// A single-use invite key automatically minted for this network (Phase 4).
        /// Present when the coordinator mints one on create.
        #[serde(default)]
        initial_invite_key: Option<String>,
        /// This network's resolved overlay subnet as a CIDR string (e.g.
        /// `"10.88.1.0/24"`). Every network on a node gets a genuinely
        /// distinct one now (auto-advanced past a collision when unspecified
        /// -- see `next_available_subnet`), so it's surfaced here rather
        /// than left implicit, in case it's not the one the caller expected.
        #[serde(default)]
        subnet: String,
    },
    Joined {
        network: String,
        my_ip: Ipv4Addr,
        my_ipv6: Option<Ipv6Addr>,
        /// SUBNET-014: set when the joined network's subnet only applies after a restart.
        #[serde(default)]
        warning: Option<String>,
    },
    StatusResponse {
        endpoint_id: EndpointId,
        /// Whether the VPN is active (TUN up, networks connected) or on standby.
        active: bool,
        /// The running daemon's compiled version (`CARGO_PKG_VERSION`). The CLI
        /// compares it to its own version and hints a restart on a mismatch
        /// — e.g. after a manual binary upgrade where the daemon never
        /// restarted onto the new binary. Empty when talking to a daemon
        /// predating this field.
        #[serde(default)]
        daemon_version: String,
        networks: Vec<NetworkStatus>,
        packets_rx: u64,
        packets_tx: u64,
        bytes_rx: u64,
        bytes_tx: u64,
        /// Daemon-wide drop counters by reason (MTU-DIAG-001).
        /// `#[serde(default)]` so an older daemon's response still decodes.
        #[serde(default)]
        drops: DropCounts,
        /// Original oversized packets successfully fragmented, by IP version
        /// (MTU-DIAG-001) -- once per packet, not once per wire fragment.
        #[serde(default)]
        fragmented_ipv4: u64,
        #[serde(default)]
        fragmented_ipv6: u64,
    },
    /// The list of network key-holders (reply to `AdminList`): the local node
    /// plus every identity it has granted the key to.
    AdminListResponse {
        admins: Vec<AdminInfo>,
    },
    /// Coordinator-only: mint a single-use invite key for `network`.
    /// `expires` is an optional human-readable duration (e.g. "24h", "7d")
    /// parsed daemon-side.
    InviteCreate {
        network: String,
        #[serde(default)]
        expires: Option<String>,
    },
    /// Response to a successful [`InviteCreate`].
    InviteCreated {
        invite_key: String,
        invite_id: String,
        #[serde(default)]
        expires_at: Option<u64>,
    },
    /// List outstanding invites for `network` (coordinator-only).
    InviteList {
        network: String,
    },
    /// Response to [`InviteList`].
    InviteListResponse {
        invites: Vec<InviteInfo>,
    },
    /// Coordinator-only: revoke (mark as used) an invite by its short id.
    InviteRevoke {
        network: String,
        invite_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InviteInfo {
    pub id: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// Whether this invite was explicitly revoked (`tetron invite revoke`).
    /// This is the *only* thing this field can ever mean -- an invite that
    /// was actually redeemed by a joiner is removed from the blob entirely
    /// on successful redemption, so it's never listed again at all. A
    /// field/label calling this "used" (as it briefly did) would be
    /// actively wrong: revoking an invite nobody ever redeemed showed up
    /// identically to one that was, with no way to tell them apart.
    pub revoked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminInfo {
    /// Short id of the key-holder.
    pub short_id: String,
    /// `true` if this is the local node.
    pub self_node: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkStatus {
    /// STATUS-NETWORK-FIELD-001: superseded by `network` below (identical
    /// value) -- kept only for the in-flight fleet upgrade window, since
    /// `tetron-webui`/`tetron-systray` both access this as a direct Rust
    /// field, not just a JSON key, so removing it outright is a compile-time
    /// break in those repos rather than a soft wire-format change. Not
    /// `#[deprecated]`: that would trip `clippy -D warnings` at every
    /// construction/read site still populating it during the transition,
    /// for no benefit over a plain comment. See `DO-NOT-COMMIT/TODO.md`'s
    /// fleet checklist for exactly when it's safe to delete this field.
    pub name: String,
    /// This network's local display name (STATUS-NETWORK-FIELD-001). Same
    /// value as `name` above -- the two are identical for the duration of
    /// the fleet upgrade; use this one in new code. `#[serde(default)]` so a
    /// pre-upgrade daemon's response (which won't send this key at all)
    /// still decodes for an already-updated client.
    #[serde(default)]
    pub network: String,
    pub role: NetworkRole,
    pub my_ip: Ipv4Addr,
    pub my_ipv6: Option<Ipv6Addr>,
    pub my_hostname: Option<String>,
    pub network_key: Option<String>,
    pub member_count: usize,
    pub peers: Vec<PeerStatus>,
    /// Name of this network's OS TUN device (e.g. `tun0`), so a node
    /// belonging to several networks can tell which interface is which
    /// (host firewall rules, `ip link show`, etc.) instead of guessing.
    #[serde(default)]
    pub tun_name: String,
    /// Active (unexpired) nuke proposals (NUKE-CONSENSUS), so members can see a
    /// nuke is being considered before it executes. Empty on a solo-coordinator
    /// network (nuke there is immediate, no proposal phase).
    #[serde(default)]
    pub nuke_proposals: Vec<NukeProposalInfo>,
    /// Whether this network's own data plane (TUN link, routes) is up, as
    /// opposed to on standby (STANDBY-PER-NETWORK) — control-plane
    /// connections stay live either way. `#[serde(default)]` (defaults to
    /// `false`) so an older daemon's response still decodes.
    #[serde(default)]
    pub active: bool,
    /// This network's overlay subnet as a CIDR string (e.g. `"10.88.0.0/24"`),
    /// resolved from the signed `GroupBlob`/config (`membership::Subnet` is a
    /// bare `(Ipv4Addr, u8)` tuple with no serde impl of its own, so this is
    /// formatted client-side rather than carrying the tuple type across the
    /// wire). `#[serde(default)]` so an older daemon's response still decodes.
    #[serde(default)]
    pub subnet: String,
    /// This network's NUKE-CONSENSUS proposer threshold
    /// (NUKE-CONSENSUS-THRESHOLD-001), so an admin can see what's actually
    /// configured (fixed at creation, invisible otherwise). `#[serde(default
    /// = ...)]` so an older daemon's response (predating this field) still
    /// decodes -- as the historical hardcoded value of 2, which is what it
    /// actually was running.
    #[serde(default = "default_nuke_consensus_threshold")]
    pub nuke_consensus_threshold: u32,
}

/// Mirrors `membership::default_nuke_consensus_threshold` (2) -- duplicated
/// rather than shared since this crate doesn't depend on the main crate's
/// `membership` module; kept as one named function instead of a bare literal
/// so the two are easy to grep and keep in sync if the default ever changes.
fn default_nuke_consensus_threshold() -> u32 {
    2
}

/// One pending nuke proposal, as surfaced by `tetron status` (NUKE-CONSENSUS).
#[derive(Debug, Serialize, Deserialize)]
pub struct NukeProposalInfo {
    /// Short id of the proposing coordinator (prefix of its full identity string).
    pub short_id: String,
    /// Unix-seconds timestamp of the proposal.
    pub proposed_at: u64,
}

#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, derive_more::IsVariant, derive_more::Display,
)]
pub enum NetworkRole {
    /// Display string is "admin," not "coordinator" -- user-facing rename
    /// only (matches `tetron admin`, the CLI command that already used this
    /// word for the same concept); the variant name and internal identifiers
    /// (`is_coordinator`, `coordinator_count()`, spec IDs) are unchanged.
    #[display("admin")]
    Coordinator,
    #[display("member")]
    Member,
    /// An auto-minted 2-peer direct connection (`ray connect`). Display-only: the
    /// node is structurally still the coordinator or a member, but `ray status`
    /// surfaces these as `direct` and hides the (non-shareable) room id.
    #[display("direct")]
    Direct,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerStatus {
    pub endpoint_id: EndpointId,
    pub ip: Ipv4Addr,
    pub ipv6: Option<Ipv6Addr>,
    pub hostname: Option<String>,
    pub connection: Option<ConnectionInfo>,
    /// Whether this peer holds the network key (`tetron status`'s `role`
    /// column: admin/member). `#[serde(default)]` so an older daemon's
    /// response still decodes (defaults to `false`, i.e. "member").
    #[serde(default)]
    pub is_coordinator: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub conn_type: ConnType,
    pub remote_addr: Option<String>,
    pub rtt_ms: Option<f64>,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub datagrams_tx: u64,
    pub datagrams_rx: u64,
    pub lost_packets: u64,
    /// This connection's current QUIC datagram-size ceiling (MTU-DIAG-001),
    /// read fresh from `Connection::max_datagram_size()` at status-query time
    /// -- Quinn's DPLPMTUD ceiling changes over a connection's lifetime, so a
    /// stored/cached value would go stale. `None` if the connection doesn't
    /// currently support datagrams at all. `#[serde(default)]` so an older
    /// daemon's response still decodes.
    #[serde(default)]
    pub max_datagram_size: Option<u64>,
}

/// Per-`DropReason` drop counters (MTU-DIAG-001), surfaced daemon-wide via
/// `StatusResponse` so the exact signal that would have caught the
/// FRAG-001/F-04 live regression -- `FragmentationFailed` counting up --
/// shows in `tetron status` instead of requiring a raw log grep.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DropCounts {
    pub send_failure: u64,
    pub no_peer: u64,
    pub malformed: u64,
    pub backpressure: u64,
    pub spoof: u64,
    pub fragmentation_failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, derive_more::IsVariant)]
pub enum ConnType {
    Direct,
    Relay,
    Tor,
    Unknown,
}

/// Maximum IPC frame size (body). Matches the previous hand-rolled guard;
/// `LengthDelimitedCodec` rejects anything larger so a malformed/hostile peer
/// can't make us allocate an unbounded buffer.
const MAX_FRAME_LEN: usize = 1_048_576;

/// A codec that frames msgpack-serialized `T`s using tokio's
/// [`LengthDelimitedCodec`] (a 4-byte big-endian length prefix — the wire format
/// is unchanged, so this stays compatible with the previous hand-rolled
/// framing). Framing is delegated to the battle-tested tokio codec; this layer
/// only does the msgpack (de)serialization on top of each length-delimited
/// frame.
///
/// Structs are serialized with `to_vec_named` (field-name maps, not positional
/// arrays) — required for correctness when a struct uses `skip_serializing_if`:
/// with positional arrays, skipping an earlier optional field shifts later
/// fields into the wrong slot on decode. The decoder (`from_slice`) handles
/// both named and unnamed representations, so it's forward-compatible with
/// older peers.
pub struct MsgpackCodec<T> {
    framed: LengthDelimitedCodec,
    _t: PhantomData<T>,
}

impl<T> MsgpackCodec<T> {
    pub fn new() -> Self {
        Self {
            framed: LengthDelimitedCodec::builder()
                .length_field_length(4)
                .max_frame_length(MAX_FRAME_LEN)
                .new_codec(),
            _t: PhantomData,
        }
    }
}

impl<T> Default for MsgpackCodec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Serialize> Encoder<T> for MsgpackCodec<T> {
    type Error = anyhow::Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<()> {
        let body = rmp_serde::to_vec_named(&item).context("serialize IPC message")?;
        self.framed
            .encode(Bytes::from(body), dst)
            .context("frame IPC message")?;
        Ok(())
    }
}

impl<T: DeserializeOwned> Decoder for MsgpackCodec<T> {
    type Item = T;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<T>> {
        match self.framed.decode(src).context("frame IPC message")? {
            Some(frame) => Ok(Some(
                rmp_serde::from_slice(&frame).context("decode IPC message")?,
            )),
            None => Ok(None),
        }
    }
}

pub type IpcFramed = Framed<UnixStream, MsgpackCodec<IpcMessage>>;

pub fn socket_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/var/run/tetron.sock")
    } else {
        PathBuf::from("/var/run/tetron/tetron.sock")
    }
}

pub async fn connect() -> Result<IpcFramed> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .context("daemon not running — start it with: sudo tetron daemon")?;
    Ok(Framed::new(stream, MsgpackCodec::new()))
}

pub fn framed(stream: UnixStream) -> IpcFramed {
    Framed::new(stream, MsgpackCodec::new())
}

pub async fn send(framed: &mut IpcFramed, msg: IpcMessage) -> Result<()> {
    use futures::SinkExt;
    framed.send(msg).await
}

pub async fn recv(framed: &mut IpcFramed) -> Result<IpcMessage> {
    use futures::StreamExt;
    framed.next().await.context("connection closed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_roundtrip() {
        let req = IpcMessage::Create {
            mode: GroupMode::Open,
            network_name: None,
            hostname: None,
            transport: None,
            subnet: None,
            nuke_consensus: None,
        };
        let bytes = rmp_serde::to_vec_named(&req).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::Create { mode, .. } => {
                assert_eq!(mode, GroupMode::Open);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_response_roundtrip() {
        let key = iroh::SecretKey::generate().public();
        let resp = IpcMessage::Created {
            network: "test".to_string(),
            network_key: key,
            my_ip: Ipv4Addr::new(10, 88, 10, 5),
            my_ipv6: None,
            warning: None,
            initial_invite_key: Some("bs58key123".to_string()),
            subnet: "10.88.0.0/24".to_string(),
        };
        let bytes = rmp_serde::to_vec_named(&resp).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::Created {
                network,
                network_key,
                my_ip,
                initial_invite_key,
                ..
            } => {
                assert_eq!(network, "test");
                assert_eq!(network_key, key);
                assert_eq!(my_ip, Ipv4Addr::new(10, 88, 10, 5));
                assert_eq!(initial_invite_key, Some("bs58key123".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_join_with_invite_roundtrip() {
        let req = IpcMessage::Join {
            network_key: "abc".to_string(),
            alias: None,
            hostname: None,
            transport: None,
            invite: Some(vec![1, 2, 3]),
        };
        let bytes = rmp_serde::to_vec(&req).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::Join { invite, .. } => {
                assert_eq!(invite, Some(vec![1, 2, 3]));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_invite_create_roundtrip() {
        let req = IpcMessage::InviteCreate {
            network: "my-net".to_string(),
            expires: Some("24h".to_string()),
        };
        let bytes = rmp_serde::to_vec_named(&req).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::InviteCreate { network, expires } => {
                assert_eq!(network, "my-net");
                assert_eq!(expires, Some("24h".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_invite_created_roundtrip() {
        let resp = IpcMessage::InviteCreated {
            invite_key: "bs58key123".to_string(),
            invite_id: "a1b2c3d4e5f6".to_string(),
            expires_at: Some(1719600000),
        };
        let bytes = rmp_serde::to_vec_named(&resp).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::InviteCreated {
                invite_key,
                invite_id,
                expires_at,
            } => {
                assert_eq!(invite_key, "bs58key123");
                assert_eq!(invite_id, "a1b2c3d4e5f6");
                assert_eq!(expires_at, Some(1719600000));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_invite_list_roundtrip() {
        let req = IpcMessage::InviteList {
            network: "my-net".to_string(),
        };
        let bytes = rmp_serde::to_vec_named(&req).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        assert!(matches!(decoded, IpcMessage::InviteList { .. }));
    }

    #[test]
    fn test_invite_list_response_roundtrip() {
        let resp = IpcMessage::InviteListResponse {
            invites: vec![InviteInfo {
                id: "abc".to_string(),
                created_at: 1719000000,
                expires_at: 0,
                revoked: false,
            }],
        };
        let bytes = rmp_serde::to_vec_named(&resp).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::InviteListResponse { invites } => {
                assert_eq!(invites.len(), 1);
                assert_eq!(invites[0].id, "abc");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_invite_revoke_roundtrip() {
        let req = IpcMessage::InviteRevoke {
            network: "my-net".to_string(),
            invite_id: "a1b2c3d4e5f6".to_string(),
        };
        let bytes = rmp_serde::to_vec_named(&req).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        assert!(matches!(decoded, IpcMessage::InviteRevoke { .. }));
    }

    #[test]
    fn test_status_response_roundtrip() {
        let ep_id = iroh::SecretKey::generate().public();
        let peer_id = iroh::SecretKey::generate().public();
        let resp = IpcMessage::StatusResponse {
            endpoint_id: ep_id,
            active: true,
            daemon_version: "0.1.0".to_string(),
            networks: vec![NetworkStatus {
                name: "gaming".to_string(),
                network: "gaming".to_string(),
                role: NetworkRole::Coordinator,
                my_ip: Ipv4Addr::new(10, 88, 10, 5),
                my_ipv6: None,
                my_hostname: Some("alice".to_string()),
                network_key: Some("abc123".to_string()),
                member_count: 2,
                peers: vec![PeerStatus {
                    endpoint_id: peer_id,
                    ip: Ipv4Addr::new(10, 88, 10, 6),
                    ipv6: None,
                    hostname: None,
                    connection: Some(ConnectionInfo {
                        conn_type: ConnType::Direct,
                        remote_addr: Some("1.2.3.4:43737".to_string()),
                        rtt_ms: Some(5.0),
                        bytes_tx: 0,
                        bytes_rx: 0,
                        datagrams_tx: 0,
                        datagrams_rx: 0,
                        lost_packets: 0,
                        max_datagram_size: Some(1162),
                    }),
                    is_coordinator: false,
                }],
                nuke_proposals: vec![],
                tun_name: "tun0".to_string(),
                active: true,
                subnet: "10.88.0.0/24".to_string(),
                nuke_consensus_threshold: 2,
            }],
            packets_rx: 0,
            packets_tx: 0,
            bytes_rx: 0,
            bytes_tx: 0,
            drops: DropCounts {
                fragmentation_failed: 5,
                ..Default::default()
            },
            fragmented_ipv4: 3,
            fragmented_ipv6: 1,
        };
        let bytes = rmp_serde::to_vec(&resp).unwrap();
        let decoded: IpcMessage = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            IpcMessage::StatusResponse {
                endpoint_id,
                networks,
                drops,
                fragmented_ipv4,
                fragmented_ipv6,
                ..
            } => {
                assert_eq!(endpoint_id, ep_id);
                assert_eq!(networks.len(), 1);
                assert_eq!(networks[0].peers[0].endpoint_id, peer_id);
                assert_eq!(
                    networks[0].peers[0]
                        .connection
                        .as_ref()
                        .unwrap()
                        .max_datagram_size,
                    Some(1162)
                );
                assert_eq!(drops.fragmentation_failed, 5);
                assert_eq!(fragmented_ipv4, 3);
                assert_eq!(fragmented_ipv6, 1);
            }
            _ => panic!("wrong variant"),
        }
    }
}
