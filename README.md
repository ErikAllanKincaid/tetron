# Torpedo

<img src="images/torpedo2.png" alt="Electric ray (Torpedo)" width="320">

Electric ray *Torpedo californica*

**A P2P mesh VPN that coexists with Tailscale.** Torpedo is a small, focused fork of [rayfish](https://github.com/rayfish/rayfish) that makes the overlay IPv4 subnet configurable, so it can run on the same machine as an active Tailscale client. Connect your machines by cryptographic identity — no servers to run, no ports to forward, no static IPs to manage.

```bash
sudo torpedo up                 # start the node (installs the service)
torpedo create --hostname alice # you're the coordinator of a private network
torpedo invite <network>        # mint a one-time code to hand out
torpedo join <invite-code>      # a second machine joins with the code
ping bob.<network>.ray          # reach each other by name
```

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-brightgreen.svg)](LICENSE)
![Status: experimental](https://img.shields.io/badge/status-experimental-orange.svg)
![Fork of: rayfish](https://img.shields.io/badge/fork%20of-rayfish-blue.svg)

> **This is not original software.** Torpedo is rayfish with a focused set of changes, kept as an honest fork under the same MPL-2.0 license. All credit for the mesh-VPN design belongs to [upstream rayfish](https://github.com/rayfish/rayfish); this fork exists to run it alongside Tailscale as a second ingress. It may need rework as upstream evolves and does not track it automatically.

---

## Why this fork

Upstream rayfish hardcodes its overlay IPv4 range to `100.64.0.0/10` (the CGNAT range) and refuses to start if another interface already holds an address there. That is exactly the range **Tailscale** uses — so stock rayfish and Tailscale cannot run on the same host. Torpedo makes the overlay subnet configurable and defaults it to a range that coexists with Tailscale, so both meshes run side by side. While at it, the fork takes on a distinct identity (binary, service, paths, and wire protocol) so its traffic can never be confused with — or bind the same ports as — genuine rayfish on the same host. The "torpedo" name refers to the Electric ray *Torpedo californica*.

## TL;DR quickstart

Torpedo runs a small root daemon (comparable to Tailscale's `tailscaled`) that owns the TUN device and the iroh endpoint. Everything else is an unprivileged `torpedo` command talking to it over a local socket.

```bash
# Build (needs a recent Rust toolchain, 2024 edition)
cargo build --release
sudo install target/release/torpedo /usr/local/bin/torpedo

# Bring the node online (installs + starts the system service). Needs root once.
sudo torpedo up

# Create a private network. The default subnet 10.88.0.0/16 coexists with Tailscale.
torpedo create --hostname alice     # closed by default; add --open for a public one
torpedo invite <network>            # mint a single-use, expiring invite code

# On a second machine:
sudo torpedo up
torpedo join <invite-code> --hostname bob

# From either machine:
torpedo status                      # networks, peers, your mesh IP, traffic
ping bob.ray                        # reach peers by name (Magic DNS)
torpedo ping bob                    # mesh probe: RTT, loss, direct-vs-relay path
```

Tailscale keeps working throughout — torpedo's default `10.88.0.0/16` does not overlap Tailscale's `100.64.0.0/10`.

### Using a custom subnet

If `10.88.0.0/16` collides with a network you already use, pick another. The node builds its single TUN device at daemon start, so set the subnet **before** it is in use and restart:

```bash
torpedo config set subnet 10.77.0.0/16   # node-wide; applies on restart
sudo torpedo restart
torpedo create --hostname alice          # the network uses your node subnet
```

Do the `config set subnet` + `restart` on **every** node before it creates or joins, so all nodes share one subnet. `torpedo create --subnet <cidr>` records the subnet but only applies it to the live TUN at the next restart — it prints a reminder to run `sudo torpedo restart`, so `config set subnet` + `restart` first is the reliable path. If a requested subnet disagrees with the one the node is already on, or overlaps a real local network, torpedo refuses and tells you to pick another instead of silently breaking your routing.

## What this fork changes

Everything below is the *only* difference from upstream rayfish; every other feature is rayfish's, unchanged.

- **Configurable overlay subnet.** Set it per-network with `create --subnet`, or node-wide with `torpedo config set subnet`. The value rides the network's signed group record so every peer agrees. Default **`10.88.0.0/16`** (was `100.64.0.0/10`), chosen to coexist with Tailscale.
- **Overlap guard.** Instead of the old "refuse if anything uses `100.64`" preflight, torpedo refuses to start only if the *chosen* subnet overlaps an existing local network — so it never hijacks your routing, and never blocks the Tailscale case.
- **Distinct identity.** Binary `torpedo`, service `torpedo.service`, config under `/etc/torpedo`, and every wire identifier (ALPNs `torpedo/…`, DHT records `_torpedo…`, mDNS, and the fixed forwardable UDP port **43737**) are renamed so torpedo can run alongside genuine rayfish without collision.
- **Self-update removed.** Upstream's self-updater pulls from rayfish's release repo; enabling it here would overwrite torpedo with an upstream build. tetron deletes the machinery entirely — [upgrade manually](#upgrading).
- **Kept on purpose:** the `rayfish` relay / discovery-DNS presets (they name upstream's hosted servers; the default is iroh's neutral n0 infrastructure), and the `.ray` Magic-DNS domain, so names are still `host.network.ray`.

See `spec/design_spec.py` for the tracked requirements and the full rationale (this fork is developed spec-first with [libspec](https://github.com/drhodes/libspec)).

## How it works

Each machine runs the `torpedo` daemon, which creates a TUN device, captures IP packets, and tunnels them over [iroh](https://iroh.computer) QUIC connections.

1. **Create.** One peer starts a network and becomes its coordinator. The network's public key is its **room id**: it lets others discover the network but, on a closed network, is not enough to get in.
2. **Join.** On a closed network a peer gets in with a one-time invite code, a reusable fleet key, or live approval. The coordinator is the gatekeeper, and admission survives any one coordinator being offline.
3. **Mesh.** Every peer derives its own stable virtual IPv4 (in the configured subnet) and IPv6 (`200::/7`) from its identity, then connects directly to every other peer — hole-punched where possible, falling back to encrypted relays otherwise.
4. **Use it.** Any TCP/UDP app works, addressed by IP or by `name.network.ray`.

### Who can join

##### Closed network

The **room id** is a discovery key, never an admission credential. On a **closed** network (the default) there are three ways in:

- **Invite code** — `torpedo invite <network>` mints a single-use, expiring code; the holder runs `torpedo join <code>`.
- **Reusable key** — `torpedo invite <network> --reusable` mints a multi-use, revocable key for unattended fleets (`torpedo join <key> --hostname web --auto-accept-firewall`).
- **Live approval** — the holder of just the room id runs `torpedo join <room-id>` and lands in a queue; the coordinator runs `torpedo requests` then `torpedo accept`/`deny`.

##### Open network

An **open** network (`torpedo create --open`) lets anyone with the room id join directly. By default the inbound firewall is default-deny (see `torpedo firewall show`), so ports must be opened explicitly. To let a peer reach a web server on your machine: `torpedo firewall add in allow -p tcp -P 80 --peer <PEERNAME>`.

### Direct 2-peer connections

Skip room ids entirely: everyone has a rotatable **contact id** (`torpedo contact`, also shown atop `torpedo status`) you can share like a phone number. `torpedo connect <contact-id>` asks to link up; `torpedo connections approve <id>` on the other side creates a private 2-peer network.

### DNS on hosts without a resolver manager

Magic DNS works by pointing the OS resolver at torpedo's in-process resolver on `<subnet>.100.53` (e.g. `10.88.100.53`). To do that, torpedo detects the host's DNS stack and picks the least invasive integration it can, in order: systemd-resolved (D-Bus) → NetworkManager (D-Bus) → `resolvectl` → `resolvconf` → as a last resort, **directly rewriting `/etc/resolv.conf`**.

That last path is not hypothetical: a minimal **Debian trixie** install (no desktop task, no systemd-resolved, no NetworkManager, no resolvconf — a common default-server profile) lands there, and it is the scenario a field report was filed against upstream for. When it happens, torpedo:

- backs up the original file to `/etc/resolv.conf.before-torpedo` before touching it,
- writes its own file (marked `# Added by torpedo`), pointing at the subnet-derived resolver, with your original nameservers kept as upstream fallback for non-`.ray` queries,
- prints a visible warning at `sudo torpedo up` naming the backup path and the restore command, so the takeover is never silent,
- restores the original file automatically on `torpedo down` / `sudo torpedo uninstall`, and also after a crash or hard kill (the panic hook and the next daemon start both run the restore).

On hosts with systemd-resolved or NetworkManager (most desktop Linux, and where Tailscale runs its own split-DNS), none of this applies — torpedo registers a scoped `.ray` resolver alongside your existing DNS instead of touching `/etc/resolv.conf` at all.

## Development

Developed with [Specification-driven development](https://en.wikipedia.org/wiki/Specification-driven_development) using [libspec](https://github.com/drhodes/libspec) a Specification Management System.

## Features (inherited from rayfish)

- 🔒 **Closed-by-default networks** with one-time invites, reusable fleet keys, or live approval (`--open` for public ones).
- 🌐 **Magic DNS** — `name.network.ray`, updated live as peers join, leave, or rename.
- 🧱 **Per-device firewall** — a userspace firewall for mesh traffic, layered on top of your host/kernel firewall. Directional, per-port, per-network rules, secure by default. `torpedo firewall --help`.
- 📁 **File sharing** — `torpedo send <file> <peer>`.
- 🧩 **Declarative deploy** — `torpedo apply <spec.yaml>` reconciles networks + suggested firewall rules.
- 📱 **Multi-device identity** — `torpedo pair` shares one identity across your devices.

Run `torpedo --help` (and `torpedo <command> --help`) for the full surface: `invite`, `requests`/`accept`/`deny`, `firewall`, `apply`, `send`, `pair`, `kick`, `ephemeral`, `mdns`, `netcheck`, and more.

## Permissions

Like Tailscale, the daemon authorizes each command by the **caller's UID**, not by file permissions. Read-only commands (`status`, `… show`, `files`) are open to any local user; mutating commands need root or the configured operator. The user who installs the service (`sudo torpedo up`) becomes the operator automatically. Only service-management commands need `sudo`:

```bash
sudo torpedo install | restart | uninstall   # manage the system service
sudo torpedo start | stop                     # stop = fully offline; start = back online
sudo torpedo set-operator <user>              # authorize a user to run torpedo without sudo
```

`torpedo up` / `torpedo down` toggle only the data plane (near-instant standby); the daemon stays connected to peers across `down`.

## Upgrading

There is no self-update (see [What this fork changes](#what-this-fork-changes)); upgrade by replacing the binary:

```bash
git pull
cargo build --release
sudo install target/release/torpedo /usr/local/bin/torpedo
sudo torpedo restart
torpedo version                 # confirm the new build (version + git sha)
```

`torpedo restart` cleanly stops the daemon before the swap, so you avoid replacing a binary that is currently executing.

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

Torpedo currently targets **Linux**. (The macOS and Android paths inherited from rayfish still assume the old range and identity; deferred, tracked as SUBNET-013 in `spec/design_spec.py`.)

## Background and further reading

Torpedo (via rayfish) is one of a family of "identity-based" mesh VPNs — the same category as [Tailscale](https://tailscale.com) and [ZeroTier](https://www.zerotier.com). What they share is the idea that a machine is addressed by a long-lived cryptographic key rather than by whatever IP address its network happens to hand it, and that the software then does the hard work of finding a path between two keys across NATs and firewalls. What differs between them is the transport underneath. This section is background on the pieces torpedo stands on, and on WireGuard, the protocol its Tailscale neighbor uses.

### iroh and n0

Torpedo does not implement its own peer-to-peer networking. It is built on [iroh](https://www.iroh.computer), a Rust library for direct connections between nodes identified by a public key ([source](https://github.com/n0-computer/iroh), [docs](https://www.iroh.computer/docs)). iroh handles the parts that are genuinely hard: discovering where a peer currently is on the internet, punching through NATs so two home machines can talk directly, and falling back to an encrypted relay when a direct path cannot be established. Torpedo uses iroh's QUIC datagrams as the tunnel and layers the mesh, addressing, and firewall on top.

**n0** (the team, also written "number 0", at [n0.computer](https://n0.computer)) is the group that builds iroh and operates the default public infrastructure it uses: the relay servers that bounce traffic when a direct connection fails, and the discovery service (pkarr over `dns.iroh.link`) that maps a public key to a node's current address. These are the "n0 defaults" referred to elsewhere in this README. They are a convenience, not a dependency on a central authority: no n0 server can read your traffic (it is end-to-end encrypted), the relay only ever sees ciphertext, and you can point torpedo at your own relay and discovery servers with `torpedo config set`. For how discovery and hole-punching actually work, the iroh blog is the best source ([iroh blog](https://www.iroh.computer/blog)); the general problem of NAT traversal is explained very well in Tailscale's writeup, which applies equally here ([How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)).

### QUIC, the transport

The actual bytes between peers ride on [QUIC](https://quicwg.org), a modern transport protocol that runs over UDP and was standardized as [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000). QUIC folds in TLS 1.3 encryption, multiplexed streams, and connection migration, which is why it suits a mesh where a peer's address can change mid-session. iroh uses QUIC (via the [Quinn](https://github.com/quinn-rs/quinn) implementation) for both the connection setup and the datagram tunnel, so every torpedo packet is encrypted in transit whether it travels directly or through a relay.

### WireGuard, for comparison

Torpedo does **not** use WireGuard — but Tailscale, the software torpedo is designed to coexist with, does, so it is worth understanding. [WireGuard](https://www.wireguard.com) is a VPN protocol by Jason Donenfeld, notable for being small, fast, and living in the Linux kernel. Where an older VPN like OpenVPN is large and configurable, WireGuard is deliberately minimal: a peer is a public key plus a set of allowed IPs, and the cryptography is fixed rather than negotiated. It is built on the [Noise Protocol Framework](https://noiseprotocol.org) and uses a fixed modern suite — Curve25519 for key exchange, ChaCha20-Poly1305 for encryption, BLAKE2s for hashing. The original design is described in a short, readable paper ([WireGuard whitepaper, PDF](https://www.wireguard.com/papers/wireguard.pdf)).

The key contrast: plain WireGuard gives you the encrypted tunnel but leaves you to manage keys, addresses, and reachability by hand. Tailscale wraps WireGuard with a coordination and NAT-traversal layer to make that automatic ([How Tailscale works](https://tailscale.com/blog/how-tailscale-works)). iroh occupies the same "coordination and traversal" role for torpedo, but with QUIC as the transport instead of WireGuard. So torpedo and Tailscale solve the same problem with a similar shape and different cryptographic plumbing, which is precisely why running them side by side only requires that their overlay IP ranges not collide.

## Relationship to upstream & license

Torpedo is a fork of [rayfish](https://github.com/rayfish/rayfish), licensed under the **Mozilla Public License 2.0** (`LICENSE`), the same as upstream. The entire mesh-VPN design, and the overwhelming majority of the code, is rayfish's work; this fork is in development, but changes what is listed under [What this fork changes](#what-this-fork-changes). If you want the general, upstream-quality version of configurable subnets, that belongs in rayfish itself — this fork is a scrappier, personal-use variant.
