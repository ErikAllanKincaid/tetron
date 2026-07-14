# tetron

<img src="images/torpedo2.png" alt="Electric ray (Torpedo californica)" width="320">

Electric ray *Tetronarce californica*

**A standalone P2P mesh VPN.** tetron is a derivative of [rayfish](https://github.com/rayfish/rayfish) that is stripped to a single purpose "Do one thing well.": connect machines into a private overlay with stable identity-derived addresses. It defaults to `10.88.0.0/16` (an uncommon 10.x slice that avoids Tailscale's `100.64.0.0/10`).

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

## Why this fork

Upstream rayfish hardcodes its overlay IPv4 range to `100.64.0.0/10` (the CGNAT range) and refuses to start if another interface already holds an address there. That is exactly the range **Tailscale** uses, so stock rayfish and Tailscale cannot run on the same host. tetron makes the overlay subnet configurable and defaults it to a range that coexists with Tailscale, so both meshes run side by side.

The fork takes on a distinct identity (binary `tetron`, ALPNs `tetron/net/...`, config under `/etc/tetron`, UDP port 43737) so its traffic can never be confused with, or bind the same ports as, genuine rayfish on the same host. Multiple subsystems have been removed — userspace firewall, Magic DNS, invite minting, file sharing, device pairing, hostname rename, the declarative apply layer, self-update, and more — because the purpose is a minimal, single-purpose mesh. The "tetron" name was chosen as a short, distinctive derivative of the *Torpedo californica* electric ray.

## TL;DR quickstart

tetron runs a small root daemon (comparable to Tailscale's `tailscaled`) that owns the TUN device and the iroh endpoint. Everything else is an unprivileged `tetron` command talking to it over a local socket.

```bash
# Build (needs a recent Rust toolchain, 2024 edition)
cargo build --release
sudo install target/release/tetron /usr/local/bin/tetron

# Bring the node online (installs + starts the system service). Needs root once.
sudo tetron up

# Create a private network. The default subnet 10.88.0.0/16 coexists with Tailscale.
tetron create --hostname alice     # always closed (approval-gated)

# On a second machine -- join using the invite key from the create step:
sudo tetron up
tetron join <invite-key> --hostname bob

# From either machine:
tetron status                      # networks, peers, your mesh IP, traffic
ping 10.88.x.y                     # reach a peer by its mesh IP (from status)
```

Tailscale keeps working throughout -- tetron's default `10.88.0.0/16` does not overlap Tailscale's `100.64.0.0/10`.

### Using a custom subnet

If `10.88.0.0/16` collides with a network you already use, pick another. The node builds its single TUN device at daemon start, so set the subnet **before** it is in use and restart:

```bash
tetron config set subnet 10.77.0.0/16   # node-wide; applies on restart
sudo tetron restart
tetron create --hostname alice          # the network uses your node subnet
```

Do the `config set subnet` + `restart` on **every** node before it creates or joins, so all nodes share one subnet. `tetron create --subnet <cidr>` records the subnet but only applies it to the live TUN at the next restart -- it prints a reminder to run `sudo tetron restart`, so `config set subnet` + `restart` first is the reliable path. If a requested subnet disagrees with the one the node is already on, or overlaps a real local network, tetron refuses and tells you to pick another instead of silently breaking your routing.

## How it works

Each machine runs the `tetron` daemon, which creates a TUN device, captures IP packets, and tunnels them over [iroh](https://iroh.computer) QUIC connections.

1. **Create.** One peer starts a network and becomes its coordinator. The network's public key is its **room id**: it lets others discover the network but is not enough to get in.
2. **Join.** A peer gets in using an **invite key** minted by a coordinator. The invite encodes the network pubkey and a one-time secret; the joiner presents the secret to any online coordinator, which validates it against the invite store and admits the peer. Grant a co-coordinator with `tetron admin add` so the network can grow when the original coordinator is offline.
3. **Mesh.** Every peer derives its own stable virtual IPv4 (in the configured subnet) and IPv6 (`200::/7`) from its identity, then connects directly to every other peer -- hole-punched where possible, falling back to encrypted relays otherwise.
4. **Use it.** Any TCP/UDP app works, addressed by the peer's mesh IP (from `tetron status`).

### Who can join

The **room id** is a discovery key, never an admission credential. tetron networks are **always closed** (`--open` was removed in MINIMAL-013), and there is one way in:

- **Invite key** -- a coordinator mints single-use invite keys with `tetron invite <network> create`. The joiner redeems the key with `tetron join <invite-key> --hostname bob`. Any online coordinator can validate the invite. Grant a co-coordinator with `tetron admin add` so multiple members can mint and redeem invites.

tetron has **no userspace firewall** (MINIMAL-010): within a shared network every peer can reach every port a local service binds. Mesh membership still gates *who* can connect, but restricting *which ports* is the host firewall's job (nftables/ufw) on the `tetron` TUN interface -- e.g. `nft add rule inet filter input iifname "tetron" tcp dport != 22 drop`.

### Naming peers (Magic DNS removed)

tetron removed Magic DNS and all OS DNS mutation (MINIMAL-012), so the daemon never touches `/etc/resolv.conf`, systemd-resolved, or NetworkManager. Reach peers by their **mesh IP**, listed with their hostnames in `tetron status` (`tetron status --json` for scripting). If you want names, add the IPs to `/etc/hosts` (or generate it from `status --json`). A hostname is set once at join (`--hostname`, collision-resolved by the coordinator) and is fixed after that -- MINIMAL-014 removed `tetron hostname` rename; hostnames still ride the signed roster, so `tetron kick <hostname>` continues to work.

## Development

Developed with [Specification-driven development](https://en.wikipedia.org/wiki/Specification-driven_development) using [libspec](https://github.com/drhodes/libspec) a Specification Management System.

## Features

**tetron additions:**
- **Configurable overlay subnet** -- default `10.88.0.0/16` avoids Tailscale's `100.64.0.0/10`. Override per-network with `create --subnet` or node-wide with `config set subnet`. The overlap guard refuses to start only if the chosen subnet collides with an existing local network.

**Inherited from rayfish:**
- **Invite-key admission** -- invite-only closed networks. Coordinators mint single-use invite keys with `tetron invite <network> create`; joiners redeem them with `tetron join <invite-key>`.
- **Dual-stack** -- stable IPv4 in the configured subnet (FNV-1a of identity) and stable IPv6 in `200::/7` (blake3 of identity, 120-bit, never rotates).
- **NAT traversal** -- direct connections with hole-punching, relay fallback via iroh. Optional Tor transport (`--tor`).

Run `tetron --help` (and `tetron <command> --help`) for the full surface: `create`/`join`/`leave`/`nuke`, `invite` (create/list/revoke), `admin`/`kick`, `config`, `status` (`--json`), `up`/`down`, and `completions`.

> **tetron removes file sharing and multi-device pairing** (MINIMAL-004). There is no `tetron send`/`files` or `tetron pair`/`unpair`; the identity model is one device = one user. Copy files with `scp`/`rsync` over the mesh IPs, and back up the identity key yourself (it is one `0600` file under the config dir).
>
> **tetron removes the declarative apply layer and local aliases** (MINIMAL-011). There is no `tetron apply`, `tetron alias`, or `tetron identityof`. Reconcile a fleet with a script over `tetron status --json`.
>
> **tetron admits by invite key only** (MINIMAL-013, superseded by LIVE-001 + INVITE-001..009). `tetron create` always makes a closed network (`--open` is gone). Coordinators mint invite keys with `tetron invite <network> create`; joiners redeem them with `tetron join <invite-key>`. The old live-approval path (`tetron requests`/`accept`/`deny`) was removed in LIVE-001. tetron can still *join* a full-tetron network by invite code, and it validates reusable keys presented against a full-tetron roster.
>
> **tetron fixes the hostname at join** (MINIMAL-014). There is no `tetron hostname` rename or `tetron ephemeral` auto-kick. A member's name is set once at join (the coordinator still resolves collisions), and `tetron kick` remains for removing a member.
>
> **tetron removes Magic DNS and all OS DNS mutation** (MINIMAL-012). There is no `.ray` resolver and the daemon never touches system DNS. Reach peers by mesh IP from `tetron status`; name them via `/etc/hosts` if you like. Hostnames still ride the roster and show in `status`.
>
> **tetron removes the userspace firewall** (MINIMAL-010). There is no `tetron firewall`. Within a shared network every peer reaches every port a local service binds -- mesh membership gates *who* can connect, and restricting *which ports* is the host firewall's job (nftables/ufw) on the `tetron` TUN interface. The only in-daemon ingress check is anti-spoofing (a peer may only source packets from its own mesh IP).

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

tetron currently targets **Linux**. (The macOS and Android paths inherited from rayfish still assume the old range and identity; deferred, tracked as SUBNET-013 in `spec/design_spec.py`.)

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
