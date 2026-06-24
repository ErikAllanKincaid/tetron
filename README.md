# Rayfish

**A peer-to-peer mesh VPN with zero infrastructure.** Spin up a private network, share a code, and your machines behave like they're on the same LAN — no servers to run, no ports to forward, no static IPs to manage.

```bash
ray create                 # you're now the coordinator of a private network
ray invite gaming          # mint a one-time code to hand out
ray join <invite-code>     # a friend joins with the code
ping alice.gaming.ray      # reach each other by name
```

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-brightgreen.svg)](LICENSE)
![Status: experimental](https://img.shields.io/badge/status-experimental-orange.svg)

---

## Why Rayfish

- **No infrastructure.** There's no control server to host or trust. Peers find each other through a DHT and connect directly; the only "server" is whoever ran `ray create`, and they can be offline once everyone's admitted.
- **Identity, not IP.** Every machine has a cryptographic identity, and its addresses are *derived* from that identity — stable, collision-free, and assigned without any coordinator handing them out.
- **Private by default.** Networks are closed unless you say otherwise. The code you share to *discover* a network isn't enough to *join* it.
- **It just works over NAT.** Hole-punching and end-to-end encryption come from [iroh](https://iroh.computer). When a direct path isn't possible (~10% of the time), traffic falls back to encrypted relays.
- **Reach peers by name.** Magic DNS gives you `name.network.ray` so you never memorize a virtual IP.

## How it works

Each machine runs a small daemon (think Tailscale's `tailscaled`) that creates a TUN device, captures IP packets, and tunnels them over iroh's QUIC connections. Everything else — `create`, `join`, `status`, file sharing — is an unprivileged command that talks to the daemon over a local socket.

1. **Create** — one peer starts a network and becomes its *coordinator*. The network's public key is its **room id**: it lets others discover the network but, on a closed network, is not enough to get in.
2. **Join** — on a closed network a peer gets in with a **one-time invite code** (`ray invite`) or by **requesting approval** (`ray requests` / `ray accept`). The coordinator is the gatekeeper.
3. **Mesh** — every peer derives its own stable virtual IPv4 (`100.64.0.0/10`) and IPv6 (`200::/7`) straight from its identity, then connects directly to every other peer.
4. **Use it** — any TCP/UDP app just works, addressed by IP or by `name.network.ray`.

## Features

- 🔒 **Closed-by-default networks** with one-time invites, reusable fleet keys, or live approval (`--open` for public ones)
- 🌐 **Magic DNS** — `name.network.ray`, updated live as peers join, leave, or rename
- 🧱 **Per-device firewall** — directional, per-port, per-network rules with stateful return traffic
- 🤝 **Coordinator firewall suggestions** — on any network the coordinator can *suggest* firewall rules that ride the signed network record (`*` targets all hosts); each node reviews them or opts into auto-install with `--auto-accept-firewall`
- 📜 **Declarative provisioning** — `ray apply deploy.yaml` (YAML) to stand up networks and firewall rules from a spec
- 👥 **Multi-device identity** — pair your laptop and phone under one identity; encrypted key backup (optionally to 1Password)
- 📁 **File sharing** — `ray send file.zip bob`
- 📡 **mDNS** local discovery, and optional **Tor** transport
- 🛠 **Operator model** — like Tailscale, run day-to-day commands without `sudo`

## Quick start

### 1. Install & start

```bash
cargo build
sudo ray up    # installs the system service if needed, then activates the VPN
```

The **first** `ray up` needs root — it installs the system service and starts the daemon (which owns the TUN device and the iroh endpoint). After that the daemon stays running and **every command, including `ray up`/`ray down`, runs unprivileged** over a local socket.

### 2. Create a network

```bash
ray create --hostname alice          # closed by default; add --open for a public network
# ✓ network created  gentle-amber-fox
#   IPv4  100.64.23.142
#   IPv6  200:ab3f:d92c:1e4a::1
```

### 3. Invite someone

```bash
ray invite gentle-amber-fox          # mint a single-use, expiring code
# ✓ invite ab3f9c01
#   <invite-code>
#   single-use, expires in 7d
```

Hand the code to a friend. (On a closed network they can also just run `ray join <room-id>` to land in your approval queue — see [Who can join](#who-can-join).)

### 4. Join from another machine

```bash
ray join <invite-code> --name gaming --hostname bob
# ✓ joined gaming
#   IPv4  100.64.7.201
#   IPv6  200:7c10:5e8b:33a1::1
```

### 5. Reach each other

```bash
ray status               # networks, peers, and traffic
ping alice.gaming.ray    # by name
ping bob.ray             # flat lookup
ping 100.64.23.142       # or just the IP
```

### 6. Leave or pause

```bash
ray leave gaming         # leave a network
ray down                 # standby: TUN + DNS torn down, daemon keeps running
ray up                   # reactivate (no root needed)
```

Run `ray --help` to discover the rest: `invite`, `requests`/`accept`/`deny`, `firewall`, `apply`, `send`, `pair`, `mdns`, and more.

## Who can join

The **room id** (a network's public key) is a *discovery* key — it's published so peers can find the network, but on a closed network it is **not** an admission credential. Admission is always the coordinator's job:

- **Closed (default)** — three ways in:
  - **Invite code** — `ray invite <network>` mints a single-use, expiring code. The holder runs `ray join <code>`; the coordinator verifies and **burns** it. Manage with `ray invite <network> list` / `revoke <id>`.
  - **Reusable key** — `ray invite <network> --reusable` mints a multi-use, expiring key for unattended fleets. Its hash rides the network's signed record, so it admits many machines and `revoke` propagates to every key-holder. A server joins non-interactively with `ray join <key> --hostname web --auto-accept-firewall`. The name isn't authoritative, so two servers asking for `web` become `web` and `web-1` — for stable per-host names give each a unique `--hostname` (e.g. a cloud instance id), and prefer the `*` wildcard subject for fleet firewall suggestions (a rule keyed to one hostname can retarget as servers come and go). **Key expiry ≠ member expiry:** expiry/revoke only blocks *new* joins; machines already admitted stay members.
  - **Live approval** — the holder of just the room id runs `ray join <room-id>` and lands in a queue. The coordinator runs `ray requests <network>`, then `ray accept <network> <id>` (or `ray deny`).
- **Open** (`ray create --open`) — anyone with the room id joins directly. Good for public or community networks.

Either gate runs through a coordinator. The full coordinator set is published in the network's signed record (`Member.is_coordinator`), so a fresh joiner dials the invite minter first, then falls back across the other coordinators — admission survives any one coordinator being offline. Once admitted, a member reconnects by cryptographic identity and no coordinator needs to be online.

## Permissions

Like Tailscale, the daemon authorizes each command by the **caller's UID**, not by file permissions:

- Read-only commands (`status`, `*… show`, `files`) are open to any local user.
- Mutating commands need root or the configured **operator**.
- The user who installs the service (`sudo ray up` / `ray install`) becomes the operator automatically, so they keep working without `sudo`. Authorize someone else with `sudo ray set-operator <user>`.

Only a handful of commands need root, because they manage the system service itself:

```bash
sudo ray install | restart | uninstall   # manage the service unit / launchd plist
sudo ray set-operator <user>              # let a user run ray without sudo
```

## Troubleshooting

```bash
ray report               # bundle logs + metrics, open a pre-filled GitHub issue
```

The daemon writes rolling logs to `/var/log/rayfish/` (Linux) or `/Library/Logs/rayfish/` (macOS). `ray report` collects those logs, current metrics, and a **sanitized** status snapshot (no private keys) into a `.tgz`, then opens a pre-filled GitHub issue for you to attach. The bundle is written locally first, so you can review it before sharing.

## How it compares

Rayfish sits closest to [Tailscale](https://tailscale.com), but without a coordination server: there's no account, no control plane, and nothing to self-host — the network's signed record on a public DHT is the only shared state. Unlike raw [WireGuard](https://www.wireguard.com), you don't hand-manage keys, IPs, or peer configs. Unlike [Nebula](https://github.com/slackhq/nebula), there's no certificate authority to run; identity *is* the key.

## Status

Rayfish is **experimental, pre-1.0 software** and has not had an independent security audit. The wire format and on-disk config may still change between releases. Try it, break it, and please [file issues](https://github.com/rayfish/rayfish/issues) — but don't bet anything critical on it yet.

## Building

```bash
cargo build                  # debug build
cargo build --features tor   # optional Tor transport
cargo build --features otel  # optional OTLP span export
```

Requires the Rust 2024 edition (Rust 1.85+).

## Contributing & security

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and [SECURITY.md](SECURITY.md) to report vulnerabilities privately.

## License

Rayfish is licensed under the [Mozilla Public License 2.0](LICENSE) (MPL-2.0).
