# Pitopi

A peer-to-peer mesh VPN that lets you create private virtual networks without any infrastructure. Built on [iroh](https://iroh.computer), it connects peers by cryptographic identity — not IP addresses — so you never need to deal with port forwarding, dynamic DNS, or firewall rules.

## Why?

You want to play Minecraft with friends, but nobody wants to set up port forwarding or pay for a hosted server. With Pitopi, one person creates a network, shares a short code, and everyone joins. Each player gets a virtual IP and the game thinks you're all on the same LAN.

But it's not just for games. Pitopi gives you a private, encrypted network between any set of devices — work machines, home servers, cloud instances — without trusting a third party.

## How it works

1. **Create a network** — one peer starts a network and becomes the coordinator
2. **Share the code** — the creator gets a short room code (like `ybnj-raqe-...`) to share with friends
3. **Join** — peers connect using the room code. iroh handles NAT traversal, hole-punching, and encrypted transport automatically
4. **Full mesh** — the coordinator assigns virtual IPs and broadcasts the peer list. Every peer connects directly to every other peer
5. **Use it** — every peer gets a virtual IP (100.64.x.x). Any app that uses TCP/UDP just works

Under the hood, Pitopi creates a TUN device on each machine, captures IP packets, and tunnels them through iroh's QUIC-based P2P connections. If direct connections aren't possible (~10% of cases), traffic falls back to encrypted relay servers.

## Quick start

```bash
# Build
cargo build

# Start the daemon (on both machines)
sudo pitopi daemon &

# Create a network (on machine A)
pitopi create --name gaming
# > Network 'gaming' created.
# >   IP: 100.64.0.1
# >   Room code: gaming/ybnj-raqe-c5s6-...

# Join the network (on machine B)
pitopi join gaming/ybnj-raqe-c5s6-...
# > Joined network 'gaming'.
# >   IP: 100.64.0.2

# Now you can reach each other
ping 100.64.0.1   # from machine B
ping 100.64.0.2   # from machine A
```

The daemon requires `sudo` (creates TUN devices), but all other commands run unprivileged.

## Commands

```
# Daemon (run once, manages all networks)
sudo pitopi daemon                  Start the daemon (required for all other commands)
sudo pitopi up                      Alias for daemon

# Network management (talks to daemon via IPC)
pitopi create [--name <name>]       Create a new network (you become the coordinator)
pitopi join <code> [--name <name>]  Join a network using a room code or endpoint ID
pitopi leave <name>                 Leave a network (tears down connections, removes config)
pitopi status                       Show active networks, peers, and IPs (live from daemon)
pitopi down                         Shut down the daemon

# Standalone (no daemon needed)
pitopi list                         List saved networks from config file
pitopi completions <shell>          Generate shell completions (bash, zsh, fish)

# Service
pitopi install-service              Install systemd (Linux) or launchd (macOS) service
pitopi uninstall-service            Remove the system service
```

## Daemon architecture

Pitopi uses a daemon/client split similar to Tailscale. The daemon (`pitopi daemon`) is a long-lived root process that owns the iroh endpoint, TUN device, and all peer connections. CLI commands talk to it over a Unix socket (`~/.config/pitopi/pitopi.sock`).

You can dynamically create, join, and leave networks while the daemon is running — no restart needed.

## Multiple networks

You can run multiple isolated networks simultaneously through a single daemon:

```bash
sudo pitopi daemon &               # start the daemon
pitopi create --name gaming         # create first network
pitopi create --name work           # create second network
pitopi status                       # see both networks with live peer info
```

Networks are fully isolated — different ALPN protocols, different peer sets, no cross-talk.

## Running as a service

```bash
sudo pitopi install-service    # installs systemd unit or launchd plist
```

The service runs `pitopi daemon` on boot, restoring all saved networks automatically.

## Configuration

Network memberships are stored at `~/.config/pitopi/networks.toml`. Identity (Ed25519 keypair) persists at `~/.config/pitopi/secret_key` — same endpoint ID across restarts.

## Building

```bash
cargo build
```

Requires Rust 2024 edition. Cross-compile for Linux servers:

```bash
just cross                   # build for x86_64 Linux
just deploy <ip>             # cross-build + rsync + install to server
```

## Architecture

```
App (Minecraft, etc.) → TUN device (100.64.x.x) → pitopi → iroh QUIC datagrams → peer
```

- Full mesh topology — every peer connects directly to every other peer
- Coordinator assigns IPs and broadcasts peer list via a control channel (QUIC bidirectional stream)
- Data flows as QUIC datagrams (low-latency, no head-of-line blocking)
- Routing table dispatches packets by destination IP from the IPv4 header
- Split TUN I/O (TunReader/TunWriter) for lock-free concurrent read/write
- ACL policy engine filters packets at the forwarding layer
- Per-network ALPN isolation on a single shared iroh endpoint

## Roadmap

See [TODO.md](TODO.md) for the full roadmap. Current status:

- [x] Point-to-point tunnel between two peers
- [x] Multi-peer full mesh (N peers in one network)
- [x] Multiple simultaneous networks with isolation
- [x] Persistent network config
- [x] Room codes for easy sharing
- [x] DHT membership publishing for offline coordinator resilience
- [x] ACL policy engine and audit logging
- [x] Systemd/launchd service integration
- [x] Daemon architecture (daemon + thin CLI client via Unix socket IPC)
- [ ] Social discovery (Discord, Slack, Steam)
- [ ] macOS Network Extension (no sudo)
- [ ] Windows, iOS, Android
