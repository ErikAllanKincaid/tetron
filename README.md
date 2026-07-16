# tetron

<img src="images/torpedo2.png" alt="Electric ray (Torpedo californica)" width="320">

Electric ray *Tetronarce californica*

**A standalone P2P mesh VPN.** tetron is a derivative of [rayfish](https://github.com/rayfish/rayfish) that is stripped to a single purpose "Do one thing well.": connect machines into a private overlay with stable identity-derived addresses. It defaults to `10.88.0.0/24` (an uncommon 10.x slice that avoids Tailscale's `100.64.0.0/10`).

### TL;DR

```bash
sudo tetron up                        # start the node (installs the service)
tetron create --hostname alice        # you are the coordinator; prints an invite key
tetron join <invite-key> --hostname bob # a second machine joins using the invite key
ping 10.88.x.y                        # reach each other by mesh IP from `tetron status`
```

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-brightgreen.svg)](LICENSE)
![Status: experimental](https://img.shields.io/badge/status-experimental-orange.svg)
![Fork of: rayfish](https://img.shields.io/badge/fork%20of-rayfish-blue.svg)

> **This is not original software.** tetron is rayfish with a focused set of changes, kept as an honest fork under the same MPL-2.0 license. All credit for the mesh-VPN design belongs to [upstream rayfish](https://github.com/rayfish/rayfish). It may need rework as upstream evolves and does not track it automatically.

---

## How to quickstart

tetron runs a small root daemon (comparable to Tailscale's `tailscaled`) that owns the TUN device and the iroh endpoint. Everything else is an unprivileged `tetron` command talking to it over a local socket.

```bash
# 1. Install the binary and bring the node online (needs root once):
curl -Lo tetron https://github.com/ErikAllanKincaid/tetron/releases/download/nightly/tetron-linux-x86_64
chmod +x tetron
sudo install tetron /usr/local/bin/tetron
sudo tetron up

# 2. Create a network. Output includes your mesh IP and invite key.
tetron create --name mynet --hostname alice --subnet 10.88.0.0/24

#    Sample output:
#      created mynet        ← network name
#        address  10.88.0.1  ·  abcd…1234
#      next: tetron join <invite-key>    single-use invite (one more available)
#            tetron invite <net> create  mint another invite
#            tetron up                   activate the VPN

# 3. On a second machine, join using the invite key from step 2:
sudo tetron up
tetron join <invite-key> --hostname bob

# 4. From either machine:
tetron status                      # networks, peers, mesh IPs, hostnames, traffic
ping <other-ip>                    # reach the other node by its mesh IP (from status)
```

Tailscale keeps working throughout -- tetron's default `10.88.0.0/24` does not overlap Tailscale's `100.64.0.0/10`.

## Why this fork

Upstream rayfish hardcodes its overlay IPv4 range to `100.64.0.0/10` (the CGNAT range) and refuses to start if another interface already holds an address there. That is exactly the range **Tailscale** uses, so stock rayfish and Tailscale cannot run on the same host. tetron makes the overlay subnet configurable and defaults it to a range that coexists with Tailscale, so both meshes run side by side.

The fork takes on a distinct identity (binary `tetron`, ALPNs `tetron/net/...`, config under `/etc/tetron`, UDP port 43737) so its traffic can never be confused with, or bind the same ports as, rayfish on the same host. Multiple subsystems from upstream have been removed — userspace firewall, Magic DNS, file sharing, device pairing, hostname rename, the declarative apply layer, self-update, and more — because the purpose is a minimal, single-purpose mesh. Invite-key admission was re-added as the sole enrollment method. The "tetron" name was chosen as a short, distinctive derivative of the *Tetronarce californica* electric ray.

### Using a custom subnet

If `10.88.0.0/24` collides with a network you already use, pick another. Set the subnet before creating or joining and restart the daemon:

```bash
tetron config set subnet 10.99.0.0/24   # node-wide; applies on restart
sudo tetron restart
tetron create --hostname alice          # the network uses your node's subnet
```

The subnet is per-node, so every node must agree before they can mesh. A mismatch is caught at join time with a clear error. If a subnet overlaps a real local network, tetron refuses at daemon start to avoid breaking your routing.

## How it works

Each machine runs the `tetron` daemon, which creates a TUN device, captures IP packets, and tunnels them over [iroh](https://iroh.computer) QUIC connections.

1. **Create.** One peer starts a network and becomes its coordinator. The network's public key is its **room id**: it lets others discover the network but is not enough to get in.
2. **Join.** A peer gets in using an **invite key** minted by a coordinator. The invite encodes the network pubkey and a one-time secret; the joiner presents the secret to any online coordinator, which validates it against the signed blob and admits the peer. Every fully trusted member should be granted the network key with `tetron admin add` -- this eliminates the single-point-of-failure where only one machine can admit, mint, or kick.
3. **Mesh.** Every peer derives its own stable virtual IPv4 (in the configured subnet) and IPv6 (`200::/7`) from its identity, then connects directly to every other peer -- hole-punched where possible, falling back to encrypted relays otherwise.
4. **Use it.** Any TCP/UDP app works, addressed by the peer's mesh IP (from `tetron status`).

### Making a co-coordinator

By default only the node that ran `tetron create` holds the network key. That machine is a **single point of failure**: if it is asleep or offline, no other node can admit new joiners, mint invites, or kick departed members. Every trusted member of the network should be made a **co-coordinator** by granting them a copy of the network key.

The command is `tetron admin <network> add <identity>`, where identity can be
a member's hostname, mesh IP, or short id (from `tetron status`):

```bash
# 1. Find the member you want to promote in `tetron status`:
tetron status

# 2. Grant them the network key (by hostname, mesh IP, or short id):
tetron admin mynet add bob
#    Sample output:
#     granted network key to bob on mynet

# 3. The new co-coordinator receives the key over the authenticated mesh
#    connection (no manual copy needed). After a few seconds `tetron status`
#    on their machine shows `coordinator` as their role.
```

The new co-coordinator can then mint invites, admit joiners, and kick members
just like the original coordinator. Run this for **every** fully trusted
member so the network stays operational even when any one machine is offline.

To see who currently holds the network key:

```bash
tetron admin mynet list
#    (c)  a1b2c3d4e5  alice  10.88.0.1
#    (c)  f6e7d8c9b0  bob    10.88.0.2
```

Each row marked `(c)` is a co-coordinator. The original creator is always
a coordinator.

### Who can join

The tetron networks are **invite-only**. The only way in is with an invite key:

- A coordinator mints **single-use invite keys** with `tetron invite <network> create`. The joiner redeems the key with `tetron join <invite-key> --hostname bob`. The invite is validated against the signed blob, so any online coordinator can admit the joiner.
- `tetron create` auto-mints the first invite key and prints it in its output, so you can share immediate access.
- Grant a co-coordinator with `tetron admin add` so another member can mint invites (and admit joiners) when the original coordinator is offline. Every trusted member should be a co-coordinator to avoid a single-point-of-failure.
- Invite keys default to 7-day expiry. Use `--expires never` for permanent invites, or `--expires 24h` / `--expires 30d` for custom durations.

Joining with a bare room id is not supported (tetron removed live approval).

tetron has **no userspace firewall** — within a shared network every peer can reach every port a local service binds. Mesh membership still gates *who* can connect, but restricting *which ports* is the host firewall's job (nftables/ufw) on the `tetron` TUN interface -- e.g. `nft add rule inet filter input iifname "tetron" tcp dport != 22 drop`.

### Naming peers

Reach peers by their **mesh IP**, listed with their hostnames in `tetron status` (`tetron status --json` for scripting). If you want names, add the IPs to `/etc/hosts` (or generate it from `status --json`). A hostname is set once at join (`--hostname`, collision-resolved by the coordinator) and is fixed after that, there is no rename command.

Note: `--hostname` is your node's name within the network, not the network's name. The network itself gets a random three-word name (or one you set with `--name` on `create`). You refer to networks by their name (`tetron leave <network-name>`, `tetron invite <network-name> create`). `tetron kick` requires an endpoint id (short id from `tetron status`), not a hostname.

## Features

**tetron additions:**

- **Invite-key admission** -- invite-only closed networks. Coordinators mint single-use invite keys with `tetron invite <network> create`; joiners redeem them with `tetron join <invite-key>`. `tetron create` auto-mints the first invite key so you can share immediate access.
- **Configurable overlay subnet** -- default `10.88.0.0/24` avoids Tailscale's `100.64.0.0/10`. Override per-network with `create --subnet` or node-wide with `config set subnet`. The overlap guard refuses to start only if the chosen subnet collides with an existing local network.

**Inherited from rayfish:**

- **Dual-stack** -- stable IPv4 in the configured subnet (FNV-1a of identity) and stable IPv6 in `200::/7` (blake3 of identity, 120-bit, never rotates).
- **NAT traversal** -- direct connections with hole-punching, relay fallback via iroh. Optional Tor transport (`--tor`).

Run `tetron --help` (and `tetron <command> --help`) for the full surface: `create`/`join`/`leave`/`nuke`, `invite` (create/list/revoke), `admin`/`kick`, `config`, `status` (`--json`), `up`/`down`, and `completions`.

> **Removed from upstream rayfish** (these features are not in tetron): file sharing and multi-device pairing, declarative apply layer (`tetron apply`/`alias`), Magic DNS and all OS DNS mutation, userspace firewall, permissionless ("open") networks, hostname renaming, and ephemeral members. Packet filtering is the host firewall's job; name resolution is `/etc/hosts`'s job; copy files with `scp`/`rsync` over mesh IPs.

## Permissions

Like Tailscale, the daemon authorizes each command by the **caller's UID**, not by file permissions. Read-only commands (`status`, `... show`) are open to any local user; mutating commands need root or the configured operator. The user who installs the service (`sudo tetron up`) becomes the operator automatically. Only service-management commands need `sudo`:

```bash
sudo tetron install | restart | uninstall   # manage the system service
sudo tetron start | stop                     # stop = fully offline; start = back online
sudo tetron set-operator <user>              # authorize a user to run tetron without sudo
```

`tetron up` / `tetron down` toggle only the data plane (near-instant standby); the daemon stays connected to peers across `down`.

## Upgrading

There is no self-update; upgrade by replacing the binary:

```bash
git pull
cargo build --release
sudo install target/release/tetron /usr/local/bin/tetron
sudo tetron restart
tetron version                 # confirm the new build (version + git sha)
```

`tetron restart` cleanly stops the daemon before the swap, so you avoid replacing a binary that is currently executing.

## Build & install

```bash
cargo build --release           # or `cargo -q build` for a debug build
cargo test                      # unit + integration tests
cargo clippy --all-targets      # lints (kept warning-free)
```

Cross-compiling / deploying to another Linux host (via the `justfile`):

```bash
just cross                      # build for x86_64 Linux
just deploy <ip>                # cross-build release + install + start on a remote host
```

tetron currently targets **Linux** only. (macOS and Android support is deferred.)

## Uninstall

```bash
sudo systemctl stop tetron              # stop the daemon                                                      
sudo tetron nuke <network-name>         # tear down each network first                                
sudo systemctl disable tetron           # disable auto-start                                                   
sudo rm -rf /etc/tetron/                # wipe config + identity (backup if needed)                            
sudo rm /etc/systemd/system/tetron.service                                                                 
sudo systemctl daemon-reload    
```

## Development

Developed with [Specification-driven development](https://en.wikipedia.org/wiki/Specification-driven_development) using [libspec](https://github.com/drhodes/libspec), a specification management system. Each requirement is a documented class in `spec/design_spec.py`; the `reconcile.py` gate enforces automatable constraints. Commits are recorded with `libspec link` so the spec keeps a complete history alongside the code.

## Background and further reading

tetron (via rayfish) is one of a family of "identity-based" mesh VPNs -- the same category as [Tailscale](https://tailscale.com) and [ZeroTier](https://www.zerotier.com). What they share is the idea that a machine is addressed by a long-lived cryptographic key rather than by whatever IP address its network happens to hand it, and that the software then does the hard work of finding a path between two keys across NATs and firewalls. What differs between them is the transport underneath. This section is background on the pieces tetron stands on, and on WireGuard, the protocol its Tailscale neighbor uses.

### iroh and n0

tetron does not implement its own peer-to-peer networking. It is built on [iroh](https://www.iroh.computer), a Rust library for direct connections between nodes identified by a public key ([source](https://github.com/n0-computer/iroh), [docs](https://www.iroh.computer/docs)). iroh handles the parts that are genuinely hard: discovering where a peer currently is on the internet, punching through NATs so two home machines can talk directly, and falling back to an encrypted relay when a direct path cannot be established. tetron uses iroh's QUIC datagrams as the tunnel and layers the mesh and addressing on top.

**n0** (the team, also written "number 0", at [n0.computer](https://n0.computer)) is the group that builds iroh and operates the default public infrastructure it uses: the relay servers that bounce traffic when a direct connection fails, and the discovery service (pkarr over `dns.iroh.link`) that maps a public key to a node's current address. These are the "n0 defaults" referred to elsewhere in this README. They are a convenience, not a dependency on a central authority: no n0 server can read your traffic (it is end-to-end encrypted), the relay only ever sees ciphertext, and you can point tetron at your own relay and discovery servers with `tetron config set`. For how discovery and hole-punching actually work, the iroh blog is the best source ([iroh blog](https://www.iroh.computer/blog)); the general problem of NAT traversal is explained very well in Tailscale's writeup, which applies equally here ([How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)).

### QUIC, the transport

The actual bytes between peers ride on [QUIC](https://quicwg.org), a modern transport protocol that runs over UDP and was standardized as [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000). QUIC folds in TLS 1.3 encryption, multiplexed streams, and connection migration, which is why it suits a mesh where a peer's address can change mid-session. iroh uses QUIC (via the [Quinn](https://github.com/quinn-rs/quinn) implementation) for both the connection setup and the datagram tunnel, so every tetron packet is encrypted in transit whether it travels directly or through a relay.

### WireGuard, for comparison

tetron does **not** use WireGuard -- but Tailscale, the software tetron is designed to coexist with, does, so it is worth understanding. [WireGuard](https://www.wireguard.com) is a VPN protocol by Jason Donenfeld, notable for being small, fast, and living in the Linux kernel. Where an older VPN like OpenVPN is large and configurable, WireGuard is deliberately minimal: a peer is a public key plus a set of allowed IPs, and the cryptography is fixed rather than negotiated. It is built on the [Noise Protocol Framework](https://noiseprotocol.org) and uses a fixed modern suite -- Curve25519 for key exchange, ChaCha20-Poly1305 for encryption, BLAKE2s for hashing. The original design is described in a short, readable paper ([WireGuard whitepaper, PDF](https://www.wireguard.com/papers/wireguard.pdf)).

The key contrast: plain WireGuard gives you the encrypted tunnel but leaves you to manage keys, addresses, and reachability by hand. Tailscale wraps WireGuard with a coordination and NAT-traversal layer to make that automatic ([How Tailscale works](https://tailscale.com/blog/how-tailscale-works)). iroh occupies the same "coordination and traversal" role for tetron, but with QUIC as the transport instead of WireGuard. So tetron and Tailscale solve the same problem with a similar shape and different cryptographic plumbing, which is precisely why running them side by side only requires that their overlay IP ranges not collide.

## Relationship to upstream & license

tetron is a derivative of [rayfish](https://github.com/rayfish/rayfish), licensed under the **Mozilla Public License 2.0** (`LICENSE`), the same as upstream. The entire mesh-VPN design, and the overwhelming majority of the code, is rayfish's work; this fork is in development, but changes what is listed in the [changelog](CHANGELOG.md). If you want the general, upstream-quality version of configurable subnets, that belongs in rayfish itself -- this fork is a scrappier, personal-use variant.
