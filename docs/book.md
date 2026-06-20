# The Pitopi Book

A complete guide to pitopi's architecture, protocols, and internals.

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Getting Started](#2-getting-started)
3. [Identity](#3-identity)
4. [Membership](#4-membership)
5. [Transport](#5-transport)
6. [Control Protocol](#6-control-protocol)
7. [TUN Device](#7-tun-device)
8. [Packet Forwarding](#8-packet-forwarding)
9. [Peer Table](#9-peer-table)
10. [Configuration](#10-configuration)
11. [Room Codes](#11-room-codes)
12. [Access Control](#12-access-control)
13. [Audit Logging](#13-audit-logging)
14. [Statistics](#14-statistics)
15. [Shutdown](#15-shutdown)
16. [Network Lifecycle](#16-network-lifecycle)
17. [Security Model](#17-security-model)

---

## 1. Introduction

Pitopi is a peer-to-peer mesh VPN that creates private virtual networks without any centralized infrastructure. It is built on top of [iroh](https://iroh.computer), a library that provides encrypted QUIC-based peer-to-peer connectivity with automatic NAT traversal, hole-punching, and relay fallback.

The core idea is simple: every peer gets a virtual IP address derived from their cryptographic identity. When an application on your machine sends a packet to that virtual IP, pitopi captures it through a TUN device, looks up which peer owns that IP, and tunnels the packet over an encrypted QUIC connection to the right machine. To the application, it looks like all peers are on the same local network.

### The data path

```
Application (e.g., Minecraft)
    |
    v
TUN device (100.64.x.x)
    |
    v
pitopi forwarding loop
    |  reads IPv4 packets from TUN
    |  extracts destination IP from header bytes 16-19
    |  looks up the peer connection in the routing table
    v
iroh QUIC datagram
    |  encrypted, NAT-traversed
    v
Remote peer's pitopi
    |  receives datagram
    |  writes packet to local TUN device
    v
Remote application
```

Pitopi uses QUIC datagrams (not streams) for data packets. Datagrams are unreliable and unordered -- just like UDP -- which means low latency and no head-of-line blocking. This makes pitopi well-suited for real-time applications like games.

### Address space

All peers live in the `100.64.0.0/10` range, which is the IANA-assigned Carrier-Grade NAT (CGNAT) block. This range is reserved for internal use by ISPs and is extremely unlikely to collide with any real network your machine participates in. The /10 prefix gives 22 bits of host address space, allowing roughly 4 million unique addresses.

### Why not WireGuard?

WireGuard is excellent for static, pre-configured tunnels between known endpoints. Pitopi solves a different problem: you don't know your peers' IP addresses, you don't want to configure port forwarding, and you want peers to find each other by cryptographic identity alone. iroh handles the hard part -- discovering peers through relay servers, punching through NATs, and falling back to relayed connections when direct paths aren't possible.

---

## 2. Getting Started

### Building

```bash
cargo build
```

Requires Rust 2024 edition.

### Creating a network

One peer creates a network and becomes the coordinator:

```bash
sudo pitopi create --name gaming
```

This produces output like:

```
network created: gaming (mode: Restricted)
your virtual IP: 100.64.23.142
share this room code: ybnj-raqe-c5s6-...
```

The coordinator's IP is deterministically derived from their cryptographic identity. The room code is a human-friendly encoding of the coordinator's EndpointId.

### Joining a network

Other peers join by providing the room code:

```bash
sudo pitopi join ybnj-raqe-c5s6-... --name gaming
```

The joiner connects to the coordinator, receives approval and a member list, creates a TUN device, and establishes direct connections to every other peer in the mesh.

### Why sudo?

TUN devices are virtual network interfaces. Creating them requires root privileges on both Linux and macOS. All commands that create TUN devices (`create`, `join`, `up`) check for root at startup and exit with a clear error if not running as root.

### Other commands

| Command | Description | Requires root |
|---------|-------------|:---:|
| `pitopi create --name NAME [--mode open\|restricted]` | Create a network | Yes |
| `pitopi join CODE --name NAME` | Join a network | Yes |
| `pitopi list` | Show saved networks | No |
| `pitopi status` | Show network details and members | No |
| `pitopi leave NAME` | Remove a network from config | No |
| `pitopi up` | Connect to all saved networks | Yes |
| `pitopi down` | Disconnect (stop the process) | No |
| `pitopi install-service` | Install systemd/launchd service | Yes |
| `pitopi uninstall-service` | Remove system service | Yes |
| `pitopi completions SHELL` | Generate shell completions | No |

### Cross-compilation

For deploying to Linux servers from a macOS development machine:

```bash
just cross          # build for x86_64 Linux
just deploy <ip>    # cross-build + rsync + install to server
```

---

## 3. Identity

**Module:** `src/identity.rs`

Every pitopi node has a persistent Ed25519 keypair stored at `~/.config/pitopi/secret_key`. This keypair is the node's cryptographic identity -- it determines the node's EndpointId and, by extension, its virtual IP address.

### Key generation and persistence

The first time pitopi runs, it generates a random Ed25519 secret key and writes the raw 32 bytes to disk:

```
~/.config/pitopi/secret_key  (32 bytes, binary)
```

On subsequent runs, it loads the existing key. This means a node always has the same EndpointId and the same virtual IP across restarts, reboots, and even machine migrations (as long as you copy the key file).

### EndpointId

The public half of the Ed25519 keypair is the node's `EndpointId`. This is what iroh uses to identify and route to the node. It's a 32-byte value that can be displayed as a hex string or encoded as a z-base-32 room code.

The EndpointId serves dual purpose:

1. **Network address:** iroh uses it to locate and connect to the peer, handling NAT traversal and relay automatically.
2. **Identity root:** the virtual IP is deterministically derived from it, and all membership records reference it as the peer's identity string.

### Security properties

The secret key never leaves the machine. All authentication happens at the QUIC transport layer -- when two peers connect, iroh performs a mutual TLS handshake using their Ed25519 keys. A peer's EndpointId is the public key from this handshake, so a peer cannot impersonate another peer's identity at the transport level.

---

## 4. Membership

**Module:** `src/membership.rs`

The membership module is the heart of pitopi's identity and authorization system. It defines how peers are identified, how their IP addresses are assigned, and who is allowed to join a network.

### Identity-derived IP addresses

Rather than assigning IPs sequentially (first joiner gets .2, second gets .3), pitopi derives each peer's IP deterministically from their identity string using FNV-1a hashing:

```
identity string  ->  FNV-1a hash  ->  lower 22 bits  ->  100.64.0.0/10 address
```

The FNV-1a algorithm was chosen for its simplicity (no external dependencies), good distribution, and determinism. The 22-bit host space maps directly to the /10 CGNAT range.

Two host addresses are reserved:
- `100.64.0.0` -- network address (host bits = 0)
- `100.64.0.1` -- TUN gateway address (host bits = 1)

If the hash lands on either of these, the address is shifted to host bits 2 or 3.

The key property: **a peer always gets the same IP, in every network, on every run.** This makes the address a stable identifier that other peers and applications can rely on.

#### Collision handling

With 22 bits of address space and a hash function, collisions are possible. The birthday problem gives roughly a 50% collision probability around 2,000 peers. For pitopi's target use case (small groups of friends or coworkers), this is extremely unlikely, but the system handles it gracefully at two levels:

1. **Coordinator-side check:** Before broadcasting a `MemberApproved` message, the coordinator checks the derived IP against both the member list and the approved list. If a collision is found with a different identity, the peer receives a `JoinDenied` with the reason "IP collision" and the approval is never broadcast.

2. **Joiner-side check:** When a peer receives a `Welcome` message containing the member list, it checks its own derived IP against all existing members. If a collision is detected, the joiner bails out with an error rather than entering the mesh. This serves as a defense-in-depth check and is the primary collision guard when the joiner connects via a non-coordinator peer.

The `MemberList::add()` method enforces collision detection at the data structure level: it rejects any addition where a *different* identity already occupies the same IP. Re-adding the same identity with the same IP is allowed (idempotent update).

### The IdentityProvider trait

All identity operations are abstracted behind a trait to allow swapping the identity backend:

```rust
pub trait IdentityProvider: Send + Sync {
    fn local_ip(&self) -> Ipv4Addr;
    fn local_identity(&self) -> String;
    fn derive_ip(&self, peer_identity: &str) -> Ipv4Addr;
    fn verify_peer(&self, claimed_identity: &str, transport_identity: &str) -> bool;
}
```

The current implementation, `IrohIdentityProvider`, wraps an iroh `EndpointId`:

- `local_ip()` returns the FNV-1a-derived IP for this node's EndpointId.
- `local_identity()` returns the EndpointId as a string.
- `derive_ip(peer)` hashes any peer identity string to an IP.
- `verify_peer(claimed, transport)` checks that a peer's claimed identity matches their transport-level identity (simple string equality for iroh, since the QUIC handshake already authenticates the EndpointId).

This trait is designed to be replaceable. A future Discord-based identity provider could derive IPs from Discord user IDs and verify peers through OAuth tokens, without changing any of the networking or membership logic.

### MemberList

The `MemberList` is an in-memory registry of all members in a network:

```rust
pub struct Member {
    pub identity: String,       // EndpointId as string
    pub ip: Ipv4Addr,           // derived from identity
    pub is_coordinator: bool,   // whether this member created the network
}
```

The list is stored as a `HashMap<String, Member>` keyed by identity. It supports:

- `add(member)` -- insert with IP collision detection
- `remove(identity)` -- remove a member
- `get(identity)` -- lookup by identity string
- `get_by_ip(ip)` -- lookup by virtual IP
- `is_member(identity)` -- membership check
- `all()` -- list all members

### ApprovedList

The `ApprovedList` tracks peers that have been approved by the coordinator but haven't connected to the mesh yet. This is the key data structure behind the "coordinator as gatekeeper" model -- it decouples authorization from welcome.

```rust
pub struct ApprovedEntry {
    pub identity: String,   // EndpointId as string
    pub ip: Ipv4Addr,       // derived from identity
}
```

The list is stored as a `HashMap<String, ApprovedEntry>` keyed by identity. It supports:

- `approve(entry, &MemberList)` -- add with collision check against both the member list and existing approved entries
- `is_approved(identity)` -- check if a peer is pre-approved
- `remove(identity)` -- remove an entry (used when promoting to full member)
- `all()` -- list all approved entries
- `from_entries(entries)` -- bulk load from a vector

The approve-then-promote lifecycle:

1. Coordinator approves a peer → entry added to `ApprovedList`
2. Coordinator broadcasts `MemberApproved` → all peers add the entry to their local approved lists
3. Approved peer connects to any peer → welcoming peer removes from `ApprovedList`, adds to `MemberList`
4. Welcoming peer broadcasts `MemberSync` → all peers update their member lists

### NetworkState

The `MemberList` and `ApprovedList` are bundled into a `NetworkState` struct and shared across async tasks using `Arc<std::sync::RwLock<NetworkState>>`:

```rust
struct NetworkState {
    members: MemberList,
    approved: ApprovedList,
}
```

The standard library `RwLock` (not tokio's) is used because all operations are fast, non-blocking, and never hold the lock across an `.await` point.

### Group modes

Networks can operate in one of two membership modes:

**Restricted (default):** Only the coordinator can authorize new members. However, any peer can *welcome* an already-approved member. This means the coordinator doesn't need to be online when the approved peer actually connects -- it just needs to have broadcast the approval beforehand.

**Open:** Any member can both authorize and welcome new joiners. When a connection comes in, any peer that receives it checks the `MembershipPolicy` and, if they're authorized (which all members are in Open mode), they can approve and welcome the peer directly.

The mode is selected at network creation:

```bash
sudo pitopi create --name gaming --mode open
```

### The MembershipPolicy trait

Authorization is abstracted behind a trait:

```rust
pub trait MembershipPolicy: Send + Sync {
    fn can_authorize(&self, acceptor: &Member) -> bool;
}
```

Two implementations exist:

- `OpenPolicy` -- always returns `true`. Any member can accept new peers.
- `RestrictedPolicy` -- returns `true` only if `acceptor.is_coordinator` is `true`.

The `policy_for_mode()` function creates the right policy for a given `GroupMode`:

```rust
pub fn policy_for_mode(mode: GroupMode) -> Box<dyn MembershipPolicy> {
    match mode {
        GroupMode::Open => Box::new(OpenPolicy),
        GroupMode::Restricted => Box::new(RestrictedPolicy),
    }
}
```

---

## 5. Transport

**Module:** `src/transport.rs`

The transport layer wraps iroh's `Endpoint` to provide peer-to-peer QUIC connectivity with automatic NAT traversal.

### Endpoints

An iroh `Endpoint` is the local end of the P2P network. It binds to a UDP socket, registers with iroh's relay infrastructure for peer discovery, and handles NAT hole-punching. A single endpoint can serve multiple networks simultaneously by accepting connections on different ALPNs.

The endpoint is created with:
- The node's `SecretKey` (Ed25519 private key)
- A list of ALPNs (one per network the node participates in)

### ALPN-based network isolation

Each pitopi network gets its own ALPN (Application-Layer Protocol Negotiation) string:

```
pitopi/net/<network-name>
```

For example, `pitopi/net/gaming` or `pitopi/net/work`. When a connection arrives, pitopi checks the ALPN to determine which network it belongs to and routes it accordingly.

This allows a single iroh endpoint to participate in multiple networks without interference. Connection attempts for one network are invisible to another.

### Connection model

Pitopi uses two types of QUIC channels:

1. **Bidirectional streams** -- for control messages (join requests, member syncs, mesh hellos). These are reliable and ordered, suitable for the structured JSON messages that coordinate membership.

2. **Datagrams** -- for data packets (the actual network traffic tunneled through the VPN). These are unreliable and unordered, providing the lowest possible latency.

### NAT traversal

iroh handles NAT traversal automatically. The typical flow:

1. Peers register with relay servers, which store their contact information.
2. When peer A wants to connect to peer B, it looks up B's relay information.
3. iroh attempts direct UDP hole-punching between the two peers.
4. If direct connection fails (about 10% of cases), traffic flows through the relay server, still fully encrypted end-to-end.

This means pitopi works without any port forwarding, dynamic DNS, or firewall configuration.

---

## 6. Control Protocol

**Module:** `src/control.rs`

The control protocol handles all coordination between peers: join requests, membership updates, and mesh formation. Messages are sent as length-prefixed JSON over QUIC bidirectional streams.

### Wire format

```
[4 bytes: big-endian u32 length] [N bytes: JSON body]
```

The 4-byte length prefix allows the receiver to know exactly how many bytes to read for each message. Maximum message size is 64 KB.

### Message types

#### JoinApproved

Sent by the coordinator (or any authorizing peer in Open mode) to a newly joining peer:

```json
{
    "JoinApproved": {
        "your_ip": "100.64.23.142",
        "members": [
            { "identity": "abc123...", "ip": "100.64.10.5", "is_coordinator": true },
            { "identity": "def456...", "ip": "100.64.23.142", "is_coordinator": false }
        ]
    }
}
```

The `your_ip` field confirms the joiner's identity-derived IP. The `members` list contains every current member of the network, including the joiner.

#### JoinDenied

Sent when a join is rejected:

```json
{
    "JoinDenied": {
        "reason": "IP collision"
    }
}
```

Reasons include "not authorized" (policy rejection) and "IP collision" (hash collision with an existing member).

#### MemberSync

Broadcast to all existing peers when the member list changes. Also sent to reconnecting peers:

```json
{
    "MemberSync": {
        "members": [
            { "identity": "abc123...", "ip": "100.64.10.5", "is_coordinator": true },
            { "identity": "def456...", "ip": "100.64.23.142", "is_coordinator": false },
            { "identity": "ghi789...", "ip": "100.64.7.42", "is_coordinator": false }
        ]
    }
}
```

This is the primary mechanism for keeping all peers' view of the network in sync.

#### MeshHello

Sent by a newly joining peer to each existing mesh member to establish a direct connection:

```json
{
    "MeshHello": {
        "identity": "def456...",
        "ip": "100.64.23.142"
    }
}
```

The receiving peer adds the sender to its routing table and spawns a datagram reader for the connection.

#### MeshWelcome

The response to a `MeshHello`:

```json
{
    "MeshWelcome": {
        "identity": "abc123...",
        "ip": "100.64.10.5"
    }
}
```

#### ReconnectRequest

Sent by a peer that was previously a member and is reconnecting after a disconnection:

```json
{
    "ReconnectRequest": {
        "identity": "def456...",
        "ip": "100.64.23.142"
    }
}
```

The receiving peer checks whether the sender is in the known member list. If so, it adds them to the routing table and sends back a `MemberSync` with the current member list. If not, the request is rejected.

#### AdvertiseServices

Allows peers to announce services they're running:

```json
{
    "AdvertiseServices": {
        "ip": "100.64.10.5",
        "services": [
            { "name": "minecraft", "port": 25565 }
        ]
    }
}
```

This is defined in the protocol but not yet integrated into the CLI.

### Identity verification

When a peer sends a `MeshHello` or `ReconnectRequest`, the receiving peer verifies that the claimed `identity` field matches the QUIC connection's transport-level identity (`conn.remote_id()`). This prevents a peer from impersonating another member by sending a forged identity in the control message.

---

## 7. TUN Device

**Module:** `src/tun.rs`

A TUN (network TUNnel) device is a virtual network interface that operates at the IP layer. Unlike a TAP device (which works at the Ethernet layer), a TUN device sends and receives raw IPv4 packets without Ethernet framing.

### Creation

Pitopi creates a TUN device with:

- **Address:** the peer's identity-derived IP (e.g., `100.64.23.142`)
- **Gateway/destination:** `100.64.0.1` (fixed for point-to-point interface on macOS)
- **Netmask:** `255.192.0.0` (/10, covering the entire CGNAT range)
- **MTU:** 1200 bytes

The /10 netmask means the operating system routes all traffic destined for `100.64.0.0` through `100.127.255.255` to this TUN device. Any application sending to a peer's virtual IP will have its packets captured by pitopi.

### MTU

The MTU is set to 1200 bytes, which is conservative but ensures packets fit within QUIC datagram limits. QUIC datagrams are themselves carried over UDP, which sits on top of IP. With typical path MTUs of 1280-1500 bytes, 1200 leaves comfortable room for QUIC, UDP, and IP headers without fragmentation.

### Async I/O

The TUN device is wrapped in an `AsyncDevice` from the `tun` crate, which integrates with tokio's event loop. The device is protected by a `tokio::sync::Mutex` and shared between the read loop (which reads outgoing packets from the TUN) and the write path (which writes incoming packets to the TUN).

```rust
pub struct TunDevice {
    device: Arc<Mutex<AsyncDevice>>,
}
```

The `share()` method creates a cheap clone by sharing the inner `Arc`, allowing both the reader and writer to hold references to the same device.

### Platform differences

**macOS (utun):** TUN devices are point-to-point interfaces that require a destination address. The `destination(100.64.0.1)` configuration satisfies this requirement.

**Linux (/dev/net/tun):** TUN devices are created through the standard Linux TUN/TAP driver. The `ensure_root_privileges(true)` platform configuration is set.

### Single TUN architecture

Pitopi uses a single TUN device per node, shared across all networks. Since all networks use the flat `100.64.0.0/10` address space and each peer has a globally unique identity-derived IP, there is no address conflict between networks. Packets are demultiplexed by looking up the destination IP in a shared routing table.

---

## 8. Packet Forwarding

**Module:** `src/forward.rs`

The forwarding module is the data plane of pitopi. It moves packets between the TUN device and peer QUIC connections.

### Architecture

Three concurrent tasks handle forwarding:

```
TUN device                    Peer connections
    |                              |
    v                              v
run_mesh()                    spawn_peer_reader() [one per peer]
  reads packets from TUN        reads datagrams from QUIC
  looks up dest IP               sends packets via tun_tx channel
  sends datagram to peer              |
                                      v
                              spawn_tun_writer()
                                writes packets to TUN
```

### TUN read loop (`run_mesh`)

This is the main forwarding loop. It reads packets from the TUN device in a tight loop:

1. Read a packet from TUN into a 1500-byte buffer.
2. Extract the destination IP from bytes 16-19 of the IPv4 header.
3. Look up the destination IP in the `PeerTable`.
4. If found, send the packet as a QUIC datagram on that peer's connection.
5. If not found (unknown destination), record a dropped packet in stats.

The function signature reveals its dependencies:

```rust
pub async fn run_mesh(
    tun: TunDevice,
    peers: PeerTable,
    _tun_tx: mpsc::Sender<Vec<u8>>,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()>
```

### Peer readers (`spawn_peer_reader`)

Each peer connection gets a dedicated reader task that receives QUIC datagrams and forwards them to the TUN device:

1. Wait for a datagram from the peer's QUIC connection.
2. Send the raw packet bytes through the `tun_tx` channel to the TUN writer.
3. Record received bytes in stats.

If the connection drops or the channel closes, the reader task exits silently.

### TUN writer (`spawn_tun_writer`)

A single task reads from the `tun_rx` channel and writes packets to the TUN device. This serializes writes to the TUN, avoiding concurrent access.

### Destination IP extraction

The `dest_ip()` function reads the destination address directly from the IPv4 header:

```rust
fn dest_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 { return None; }    // minimum IPv4 header
    if packet[0] >> 4 != 4 { return None; }  // must be IPv4
    Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
}
```

Bytes 16-19 of an IPv4 header contain the 32-bit destination address. The function also validates that the packet is long enough to contain a complete header and that the version nibble indicates IPv4.

---

## 9. Peer Table

**Module:** `src/peers.rs`

The `PeerTable` is the routing table that maps virtual IP addresses to QUIC connections. When the forwarding loop needs to send a packet, it looks up the destination IP here to find the right connection.

### Structure

```rust
pub struct PeerTable {
    inner: Arc<RwLock<HashMap<Ipv4Addr, PeerEntry>>>,
}

pub struct PeerEntry {
    pub conn: Connection,
    pub endpoint_id: String,
}
```

Each entry maps an IP address to:
- The QUIC `Connection` object used to send datagrams to that peer.
- The peer's `endpoint_id` string for identification.

### Thread safety

The table is wrapped in `Arc<RwLock<...>>` using the standard library's `RwLock`. This allows:
- Multiple concurrent readers (the forwarding loop and any task checking membership)
- Exclusive writes (adding or removing peers)

The `PeerTable` implements `Clone` by cloning the `Arc`, so all clones share the same underlying data. This is how it's shared between the forwarding loop, accept loop, and mesh acceptor.

### Operations

- `add(ip, conn, endpoint_id)` -- insert or replace a peer entry.
- `remove(ip)` -- remove a peer and return their connection.
- `lookup(ip)` -- find the connection for a given IP (used by the forwarding loop on every packet).
- `all_connections()` -- list all peers (used for broadcasting MemberSync messages).
- `all_peer_ids()` -- list all peers with their identity strings.

### Shared across networks

In the `cmd_up` path (connecting to all saved networks), a single `PeerTable` is shared across all networks. Since the address space is flat (`100.64.0.0/10`) and each peer has a globally unique identity-derived IP, there is no ambiguity -- a given IP always maps to exactly one peer.

---

## 10. Configuration

**Module:** `src/config.rs`

Pitopi persists network memberships to `~/.config/pitopi/networks.toml` so that networks survive restarts. The `pitopi up` command reads this file to reconnect to all saved networks.

### File format

```toml
[[networks]]
name = "gaming"
coordinator_id = "abc123def456..."
group_mode = "open"
my_ip = "100.64.23.142"

[[networks.members]]
identity = "abc123def456..."
ip = "100.64.10.5"
is_coordinator = true

[[networks.members]]
identity = "def456ghi789..."
ip = "100.64.23.142"
is_coordinator = false

[[networks]]
name = "work"
coordinator_id = "xyz789..."
group_mode = "restricted"
```

### Data model

**AppConfig** -- the top-level container:
```rust
pub struct AppConfig {
    pub networks: Vec<NetworkConfig>,
}
```

**NetworkConfig** -- a single network membership:
```rust
pub struct NetworkConfig {
    pub name: String,              // human-readable name
    pub coordinator_id: String,    // EndpointId of the coordinator
    pub group_mode: GroupMode,     // Open or Restricted
    pub my_ip: Option<Ipv4Addr>,  // our IP (None if we're the coordinator)
    pub members: Vec<MemberEntry>, // known members
}
```

**MemberEntry** -- a known member:
```rust
pub struct MemberEntry {
    pub identity: String,      // EndpointId as string
    pub ip: Ipv4Addr,          // identity-derived IP
    pub is_coordinator: bool,  // whether this member is the coordinator
}
```

### Coordinator vs. member

The `my_ip` field distinguishes the coordinator from members:
- **Coordinator:** `my_ip` is `None`. The coordinator doesn't need to store their own IP separately because they know it from their identity.
- **Member:** `my_ip` is `Some(ip)`. This is the IP confirmed during the join handshake.

This distinction drives `cmd_up` behavior -- coordinators start an accept loop, members connect to the coordinator.

### Operations

- `load()` -- read the config file, or return a default empty config if it doesn't exist.
- `save(config)` -- write the config to disk as pretty-printed TOML.
- `upsert_network(config, network)` -- add a network or replace an existing one with the same name.
- `remove_network(config, name)` -- remove a network by name.

### When config is written

Config is written at several points:
1. **Create:** when the coordinator creates a network (saves self as the only member).
2. **Join:** when a peer receives `JoinApproved` or `MemberSync` (saves the full member list).
3. **Leave:** when the user runs `pitopi leave` (removes the network entry).

---

## 11. Room Codes

**Module:** `src/room_code.rs`

EndpointIds are 32-byte binary values. Their hex representation is 64 characters long -- not something you'd want to read aloud to a friend. Room codes solve this by encoding the EndpointId in z-base-32 with dashes for readability.

### Encoding

z-base-32 is a human-oriented encoding that uses only lowercase letters and digits, avoiding visually ambiguous characters (no 0/O, 1/l confusion). Pitopi adds dashes every 4 characters:

```
ybnj-raqe-c5s6-k7mp-...
```

This produces a room code that's easy to read, type, and share verbally.

### Parsing

The `parse_node_id()` function accepts both raw EndpointId strings and room codes:

```rust
pub fn parse_node_id(input: &str) -> Result<EndpointId> {
    if let Ok(id) = input.parse::<EndpointId>() {
        return Ok(id);
    }
    decode(input).context("could not parse as EndpointId or room code")
}
```

This flexibility means users can paste either format in the `join` command.

---

## 12. Access Control

**Module:** `src/acl.rs`

The ACL module provides a packet-level firewall for filtering traffic at the forwarding layer. While not yet wired into the main forwarding loop, it defines the data structures and logic for rule-based packet filtering.

### Policy structure

An ACL policy has a default action and a list of rules:

```rust
pub struct AclPolicy {
    pub default: DefaultPolicy,  // DenyAll, AllowSameNetwork, or AllowAll
    pub rules: Vec<AclRule>,
}

pub struct AclRule {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub port: Option<u16>,
    pub allow: bool,
}
```

### Evaluation

Rules are evaluated in order. The first rule matching the packet's source, destination, and port determines the action. If no rule matches, the default policy applies.

Port matching works for TCP and UDP packets. The destination port is extracted from bytes 2-3 of the transport header (after the IP header). For ICMP and other protocols without ports, port-based rules don't match.

### Example use cases

**Block all traffic except Minecraft:**
```rust
let mut policy = AclPolicy::deny_all();
policy.rules.push(AclRule {
    src: Ipv4Addr::new(100, 64, 0, 2),
    dst: Ipv4Addr::new(100, 64, 0, 1),
    port: Some(25565),
    allow: true,
});
```

**Allow all traffic except from a specific peer:**
```rust
let mut policy = AclPolicy::allow_all();
policy.rules.push(AclRule {
    src: Ipv4Addr::new(100, 64, 0, 3),
    dst: Ipv4Addr::new(100, 64, 0, 1),
    port: None,
    allow: false,
});
```

### Serialization

The policy is serializable to TOML, enabling future file-based ACL configuration:

```toml
default = "deny-all"

[[rules]]
src = "100.64.0.2"
dst = "100.64.0.1"
port = 25565
allow = true
```

---

## 13. Audit Logging

**Module:** `src/audit.rs`

The audit module provides an append-only log of peer connection events at `~/.config/pitopi/audit.log`.

### Format

Each line is a tab-separated record:

```
1719835423  connect     100.64.23.142  abc123def456...
1719835430  disconnect  100.64.23.142  abc123def456...
```

Fields:
1. Unix timestamp (seconds since epoch)
2. Event type (`connect` or `disconnect`)
3. Peer's virtual IP
4. Peer's EndpointId

### Thread safety

The log file is wrapped in a `std::sync::Mutex` to allow safe concurrent writes from multiple async tasks:

```rust
pub struct AuditLog {
    file: Mutex<std::fs::File>,
}
```

Writes use `OpenOptions::append(true)`, so even if multiple processes write concurrently, individual records won't be corrupted (append writes are atomic on most filesystems for small writes).

### Current status

The audit log infrastructure is built but not yet wired into the main connection lifecycle. The API is ready:

```rust
audit.log_connect(peer_ip, &endpoint_id);
audit.log_disconnect(peer_ip, &endpoint_id);
```

---

## 14. Statistics

**Module:** `src/stats.rs`

The stats module tracks packet and byte counters for monitoring forwarding performance.

### Counters

Five atomic counters track activity:

| Counter | Meaning |
|---------|---------|
| `packets_rx` | Packets received from peers |
| `packets_tx` | Packets sent to peers |
| `bytes_rx` | Total bytes received |
| `bytes_tx` | Total bytes sent |
| `drops` | Packets that couldn't be routed (unknown destination or send failure) |

All counters use `AtomicU64` with `Ordering::Relaxed`, since exact ordering between counters isn't important for monitoring.

### Periodic logging

The `spawn_logger` method starts a background task that logs stats every 30 seconds as deltas (not cumulative totals). This shows recent activity rather than all-time totals:

```
INFO rx=42 tx=38 bytes_rx=48.2KB bytes_tx=44.1KB drops=0 (30s)
```

### Session summary

When the shutdown signal is received, the logger prints a final summary:

```
INFO duration=12m34s total_rx=3421 total_tx=3198 total_bytes=7.2MB session complete
```

### Byte formatting

Byte counts are formatted for readability:
- Under 1 KB: `512B`
- Under 1 MB: `85.2KB`
- Over 1 MB: `1.1MB`

---

## 15. Shutdown

**Module:** `src/shutdown.rs`

Pitopi uses a `CancellationToken` from `tokio-util` for coordinated shutdown. Every long-running task (forwarding loops, accept loops, peer readers, stats logger) checks this token and exits cleanly when it's cancelled.

### Signal handling

The `token()` function creates a `CancellationToken` and spawns a task that waits for a shutdown signal:

- **Unix (macOS/Linux):** listens for both `SIGINT` (Ctrl+C) and `SIGTERM`.
- **Windows:** listens for Ctrl+C only.

When the signal arrives, the token is cancelled, and all tasks that are `tokio::select!`-ing on `token.cancelled()` exit their loops and clean up.

### Shutdown flow

```
SIGINT/SIGTERM received
    |
    v
CancellationToken cancelled
    |
    +-- run_mesh() returns Ok(())
    +-- run_accept_loop() returns Ok(())
    +-- spawn_peer_reader() returns
    +-- spawn_logger() prints session summary, returns
    |
    v
main() returns
```

The shutdown is cooperative, not forceful. Each task exits at its next `tokio::select!` checkpoint, which ensures no packets are lost mid-send and all resources are released cleanly.

---

## 16. Network Lifecycle

This chapter ties the modules together by walking through the complete lifecycle of a network.

### Creating a network

When a user runs `sudo pitopi create --name gaming --mode open`:

1. **Load identity.** Read or generate the Ed25519 keypair from `~/.config/pitopi/secret_key`.

2. **Create identity provider.** Wrap the public key in `IrohIdentityProvider`, which derives the coordinator's virtual IP.

3. **Bind endpoint.** Create an iroh `Endpoint` with the ALPN `pitopi/net/gaming`.

4. **Initialize membership.** Create a `MemberList` with self as the only member (marked `is_coordinator: true`). Create the membership policy based on the mode (`OpenPolicy` for open, `RestrictedPolicy` for restricted).

5. **Save config.** Write the network to `~/.config/pitopi/networks.toml`.

6. **Create TUN.** Create a TUN device with the coordinator's IP and /10 netmask.

7. **Start forwarding.** Spawn the TUN read loop, TUN writer, and begin the accept loop.

8. **Display room code.** Print the z-base-32 room code for sharing.

### Joining a network

When a user runs `sudo pitopi join ybnj-raqe-... --name gaming`:

1. **Parse room code.** Decode the room code to an `EndpointId`.

2. **Load identity and bind endpoint.** Same as create.

3. **Connect to coordinator.** Use iroh to establish a QUIC connection.

4. **Receive approval.** Wait for a `JoinApproved` message on the control stream. This contains the joiner's confirmed IP and the full member list.

5. **Save config.** Write the network membership to disk.

6. **Create TUN.** Create a TUN device with the joiner's IP.

7. **Connect to mesh.** For each member in the list (excluding self and the coordinator, who we're already connected to), open a QUIC connection and send `MeshHello`.

8. **Start forwarding.** Spawn per-peer readers, the TUN writer, and the TUN read loop.

9. **Start listeners.** Spawn a control listener (for MemberSync from the coordinator) and a mesh acceptor (for MeshHello from future peers).

### Coordinator's accept loop

The coordinator runs continuously, accepting incoming connections:

1. **Accept connection.** Wait for an incoming QUIC connection with the right ALPN.

2. **Check identity.** Derive the peer's IP from their EndpointId.

3. **Known member reconnecting?** If the peer is already in the member list, send them a `MemberSync` with the current member list, add them to the routing table, and spawn a reader.

4. **New member -- check policy.** Ask the `MembershipPolicy` whether this peer can accept new members. If not, send `JoinDenied`.

5. **Check IP collision.** Verify no existing member with a different identity has the same derived IP.

6. **Accept.** Add the new member to the `MemberList`, send `JoinApproved`, broadcast `MemberSync` to all existing peers, and spawn a reader.

### Reconnecting after disconnection

When a peer's connection drops (network glitch, machine sleep, etc.):

1. **Detect disconnection.** The peer reader task exits when `conn.read_datagram()` returns an error.

2. **Reconnect loop.** The `cmd_join` function runs in a loop with exponential backoff. On disconnection, it:
   - Tries the coordinator first.
   - If the coordinator is unavailable, tries every known peer from the saved config.
   - On successful connection, re-enters the mesh (receives MemberSync, reconnects to peers).

3. **Any peer can help.** Known peers accept reconnection requests because they hold the current member list. This is the "offline coordinator resilience" feature -- if the coordinator goes down, existing members can still reconnect to each other.

### Bringing all networks up

When a user runs `sudo pitopi up`:

1. **Load all saved networks** from config.

2. **Create a single shared TUN device** using the node's identity-derived IP.

3. **Create a shared PeerTable** and forwarding loop.

4. **For each network:**
   - If we're the coordinator: start an accept loop.
   - If we're a member: connect to the coordinator and join the mesh.

All networks share the same TUN device and routing table, since the address space is flat and each peer has a globally unique IP.

---

## 17. Security Model

### Transport security

All communication is encrypted end-to-end by iroh's QUIC implementation. Connections use TLS 1.3 with Ed25519 certificates derived from each peer's keypair. No traffic -- including relayed traffic -- can be read or modified by intermediaries.

### Identity authentication

Peers authenticate at two levels:

1. **Transport level:** The QUIC handshake verifies each peer's Ed25519 public key. A peer's `EndpointId` is cryptographically bound to their connection. You cannot connect to a peer without them proving they hold the corresponding private key.

2. **Application level:** When peers send `MeshHello` or `ReconnectRequest` messages, pitopi verifies that the claimed identity matches the transport-level identity (`conn.remote_id()`). This prevents a connected peer from claiming to be someone else.

### Membership authorization

Who can join a network is controlled by the `MembershipPolicy`:

- **Restricted mode:** Only the coordinator can approve new members. Members cannot add peers on their own.
- **Open mode:** Any member can approve new peers. This trades security for convenience.

### IP address integrity

Virtual IPs are derived from cryptographic identities, not assigned by the coordinator. The coordinator verifies the derivation and checks for collisions, but cannot assign a different IP than what the identity hash produces. This means a peer's IP is a stable, verifiable identifier.

### What is NOT protected

- **Traffic analysis:** An observer on the network can see that two peers are communicating (via packet timing and size), even though they can't read the content.
- **Denial of service:** A peer can flood the network with packets. No rate limiting is currently implemented.
- **Member list confidentiality:** The member list (identities and IPs) is shared with all members. A member can see who else is in the network.
- **Stale connections:** Disconnected peers' entries remain in the routing table. Packets sent to them fail silently.
