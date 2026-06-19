# Pitopi Roadmap

## Current state

Multi-peer mesh VPN over iroh QUIC datagrams. Creator acts as coordinator — assigns IPs from 100.64.0.0/24 and broadcasts peer lists. Joiners receive their IP via a control channel (QUIC bidirectional stream), then connect directly to all existing peers (full mesh). Routing table dispatches packets by destination IP. ~400 lines across 8 modules.

---

## Phase 1: Harden the foundation

Make the two-peer case rock-solid before adding complexity.

- [x] Signal handling — catch SIGINT/SIGTERM, tear down TUN device and close iroh connection cleanly
- [x] Root/sudo check — detect missing privileges early with a clear error message instead of a cryptic TUN failure
- [x] Reconnect loop — if the iroh connection drops, retry with exponential backoff instead of exiting
- [ ] ~~Oversized packet handling~~ — not needed, TUN MTU of 1200 ensures packets fit in QUIC datagrams
- [x] Periodic stats logging — packets sent/received, bytes transferred, drops, latency estimate
- [x] Graceful shutdown — flush in-flight datagrams, log session summary on exit
- [ ] Cross-platform testing — verify macOS ↔ Linux, Linux ↔ Linux, macOS ↔ macOS

## Phase 2: Multi-peer mesh

Go from 2 peers to N peers in a single network.

### IP assignment

- [x] Creator becomes the initial coordinator — assigns IPs sequentially from 100.64.0.0/24 (100.64.0.1, .2, .3, ...)
- [x] Control channel over a bidirectional QUIC stream (separate from datagram path) for IP assignment, peer list exchange, and keep-alives
- [x] Joiner requests an IP via control channel; coordinator responds with assignment + current peer list

### Mesh connectivity

- [x] Accept multiple incoming connections — creator listens in a loop, not just once
- [x] Full mesh — when a new peer joins, coordinator broadcasts the updated peer list; each peer connects to every other peer directly
- [x] Routing table — `HashMap<Ipv4Addr, Connection>` to dispatch packets to the right peer based on destination IP
- [x] Forwarding layer reads destination IP from each packet and routes to the correct connection (or drops if unknown)

### Resilience

- [x] Peer disconnect detection — remove from routing table, notify remaining peers via control channel
- [x] Replicate peer list to all members — any peer holds full state, not just the coordinator
- [ ] If coordinator goes offline, existing mesh stays connected; any peer can accept new joiners
- [x] Any peer can share the network ID (creator's EndpointId) to invite others

## Phase 3: Multi-network support

Run multiple independent virtual networks simultaneously.

### Persistent config

- [ ] Network config file at `~/.config/pitopi/networks.toml` — stores network memberships, assigned IPs, peer lists
- [ ] Each network identified by creator's EndpointId + network name (e.g., `<endpoint_id>/gaming`)
- [ ] CLI: `pitopi create --name <name>`, `pitopi join <ticket>`, `pitopi list`, `pitopi leave <name>`

### Network isolation

- [ ] Each network gets its own TUN device and /24 subnet (100.64.1.0/24, 100.64.2.0/24, ...)
- [ ] ALPN per network: `pitopi/net/<network-hash>` — iroh multiplexes connections by ALPN on a single endpoint
- [ ] Single shared iroh Endpoint across all networks (one port, one identity)
- [ ] Strict isolation — packets from one network never cross to another

### Daemon mode

- [ ] `pitopi up` — connect to all saved networks from config
- [ ] `pitopi down` — disconnect from all networks, tear down TUN devices

## Phase 4: UX polish

Make it pleasant to use day-to-day.

### Usability

- [ ] Short room codes instead of raw 52-character EndpointIds (e.g., `pitopi join abc-def-ghi`)
- [ ] `pitopi status` — show active networks, connected peers, IPs, connection quality (direct vs relay, latency, throughput)
- [ ] Colored terminal output — connection events, errors, status
- [ ] Shell completions (bash, zsh, fish) via clap

### Networking

- [ ] Graceful reconnection on network change (wifi switch, sleep/wake, IP change)
- [ ] LAN game auto-discovery — mDNS proxy over the virtual network so apps find each other without manual IP entry
- [ ] Split tunneling — only route specific subnets through pitopi, leave the rest on the default route

### Service integration

- [ ] Systemd unit file for Linux servers
- [ ] launchd plist for macOS
- [ ] `pitopi install-service` / `pitopi uninstall-service` to manage service files

## Phase 5: Daemon architecture

Separate the long-running network engine from the CLI.

- [ ] `pitopid` — background daemon process that owns the iroh endpoint, TUN devices, and all connections
- [ ] `pitopi` CLI becomes a thin client that talks to `pitopid` over a Unix domain socket (or gRPC)
- [ ] IPC protocol: create/join/leave/status/list commands as request/response messages
- [ ] Daemon auto-starts on boot via launchd (macOS) / systemd (Linux)
- [ ] macOS Network Extension (NEPacketTunnelProvider) — unprivileged TUN operation, no sudo required
- [ ] macOS System Extension distribution — standalone .app outside the App Store
- [ ] App Store distribution variant (Network Extension required by sandbox)
- [ ] Graceful fallback: use Network Extension if available, raw utun if running as root
- [ ] Menu bar app (macOS) / system tray (Linux) for quick status and network management

## Phase 6: Social discovery & auth

Let people find each other through platforms they already use.

### Discord integration

- [ ] Discord OAuth login — authenticate users, map Discord identity to EndpointId
- [ ] Lightweight coordination server — stores identity mappings (Discord user ↔ EndpointId)
- [ ] Discover peers by shared Discord servers — see who's online and has pitopi
- [ ] Create/join networks scoped to a Discord server or role
- [ ] "Play together" flow: pick a Discord server → see members with pitopi → one-click network creation → they get notified
- [ ] Discord bot companion — `/pitopi create` slash command posts a join link in the channel

### Other platforms

- [ ] Slack OAuth + bot — discover coworkers, create work networks scoped to a workspace, `/pitopi join` slash command
- [ ] Steam integration — discover friends, auto-create gaming networks
- [ ] GitHub integration — team-based networks for development
- [ ] Generic model: any social provider maps groups/servers/workspaces → network discovery

## Phase 7: Access control

Fine-grained control over who can reach what.

- [ ] ACL policy engine — rules like `user:alice can access server:gamehost on port 25565`
- [ ] Role-based access — map Discord roles / Slack groups to ACL groups
- [ ] Admin controls — org admins grant/revoke access per user, per resource, per port
- [ ] Resource tagging — peers advertise services (e.g., "minecraft:25565", "ssh:22") on the control channel
- [ ] Default policies: deny-all, allow-same-org, allow-all
- [ ] Policy format: human-readable TOML (inspired by Tailscale ACLs)
- [ ] Enforcement at the forwarding layer — filter packets against ACLs before writing to TUN
- [ ] Audit log — who connected to what, when, how much traffic

## Future / Ideas

- [ ] Windows support via Wintun driver
- [ ] iOS app (Network Extension / NEPacketTunnelProvider)
- [ ] Android app (VpnService API)
- [ ] DNS over the virtual network — resolve peer hostnames (e.g., `alice.pitopi` → 100.64.0.3)
- [ ] Bandwidth throttling / QoS per peer or per network
- [ ] Encrypted peer-to-peer file transfer over the mesh (`pitopi send <file>`)
- [ ] Web dashboard for network management and monitoring
- [ ] Headless/embedded mode for IoT devices and servers
- [ ] Exit node support — route internet traffic through a designated peer (like a traditional VPN)
