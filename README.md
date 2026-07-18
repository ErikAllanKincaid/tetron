# tetron

<img src="images/torpedo2.png" alt="Electric ray (Torpedo californica)" width="320">

Electric ray *Tetronarce californica*

**A standalone P2P mesh VPN.** tetron is a derivative of [rayfish](https://github.com/rayfish/rayfish), stripped to a single purpose: connect machines into a private overlay with stable identity-derived addresses. It defaults to `10.88.0.0/24` (an uncommon 10.x slice that avoids Tailscale's `100.64.0.0/10`).

### TL;DR

```bash
sudo tetron up                          # start the node (installs the service)
tetron create --hostname alice          # you are the coordinator; output includes an invite key
# next: tetron join <invite-key>        # <- copy this from the output, run it on the next machine

sudo tetron up                          # on a second machine:
tetron join <invite-key> --hostname bob # paste the invite key from step 1's output

tetron status                           # either machine: mesh IPs, hostnames, traffic
ping 10.88.x.y                          # reach the other node by its mesh IP
```

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-brightgreen.svg)](LICENSE)
![Status: experimental](https://img.shields.io/badge/status-experimental-orange.svg)
![Fork of: rayfish](https://img.shields.io/badge/fork%20of-rayfish-blue.svg)

> **This is not original software.** tetron is rayfish with a focused set of changes, kept as an honest fork under the same MPL-2.0 license. All credit for the mesh-VPN design belongs to [upstream rayfish](https://github.com/rayfish/rayfish).

**Want more?** This README covers getting started. For detailed walkthroughs, troubleshooting, and less-common scenarios (custom subnets, Tor transport, multi-machine deployment scripts), see **[docs/HOWTO.md](docs/HOWTO.md)**. For the ideas tetron is built on -- iroh, QUIC, WireGuard -- see **[docs/BACKGROUND.md](docs/BACKGROUND.md)**.

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
tetron create --network-name mynet --hostname alice

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
ping <other-ip>                     # reach the other node by its mesh IP (from status)
```

Tailscale keeps working throughout -- tetron's default `10.88.0.0/24` does not overlap Tailscale's `100.64.0.0/10`. If that default collides with a network you already use, [docs/HOWTO.md](docs/HOWTO.md) covers setting a custom subnet.

## Why this fork

Upstream rayfish hardcodes its overlay IPv4 range to `100.64.0.0/10` (the CGNAT range) and refuses to start if another interface already holds an address there -- exactly the range **Tailscale** uses, so stock rayfish and Tailscale cannot run on the same host. tetron makes the overlay subnet configurable and defaults it to a range that coexists with Tailscale, so both meshes run side by side. It also takes on a distinct identity (binary `tetron`, ALPNs `tetron/net/...`, config under `/etc/tetron`, UDP port 43737) so its traffic can never collide with rayfish on the same host, and strips several subsystems -- userspace firewall, Magic DNS, file sharing, device pairing, hostname rename, the declarative apply layer, self-update, and more -- because the purpose is a minimal, single-purpose mesh. Invite-key admission was re-added as the sole enrollment method.

## How it works

Each machine runs the `tetron` daemon, which creates a TUN device, captures IP packets, and tunnels them over [iroh](https://iroh.computer) QUIC connections.

1. **Create.** One peer starts a network and becomes its coordinator. The network's public key is its **room id**: it lets others discover the network but is not enough to get in.
2. **Join.** A peer gets in using an **invite key** minted by a coordinator. The invite encodes the network pubkey and a one-time secret; the joiner presents the secret to any online coordinator, which validates it against the signed blob and admits the peer.
3. **Mesh.** Every peer derives its own stable virtual IPv4 (in the configured subnet) and IPv6 (`200::/7`) from its identity, then connects directly to every other peer -- hole-punched where possible, falling back to encrypted relays otherwise.
4. **Use it.** Any TCP/UDP app works, addressed by the peer's mesh IP (from `tetron status`).

## Co-coordinators and admission

By default only the node that ran `tetron create` holds the network key -- a **single point of failure**: if it's offline, nobody else can admit joiners, mint invites, or kick members. Grant the key to every trusted member so the network stays operational no matter who's online:

```bash
tetron admin mynet add bob        # by hostname, mesh IP, or short id (from `tetron status`)
```

Networks are **invite-only**: the only way in is an invite key, single-use by default (7-day expiry, `--expires` to change it). `tetron create` auto-mints the first one; any coordinator can mint more:

```bash
tetron invite mynet create
```

A bare room id is not enough to join. tetron has **no userspace firewall** -- within a shared network every peer reaches every port a local service binds. Restrict *which ports* with the host firewall (nftables/ufw) on the `tetron` TUN interface.

See [docs/HOWTO.md](docs/HOWTO.md) for kicking a member, listing key-holders, revoking invites, and more.

## Naming peers

Reach peers by their **mesh IP**, listed with their hostnames in `tetron status` (`--json` for scripting). A hostname is set once at join (`--hostname`) and is fixed after that -- there is no rename command; see [docs/HOWTO.md](docs/HOWTO.md) for the leave-and-rejoin workaround. `--hostname` names your node within the network; the network itself has its own name (random three words, or `--network-name` on `create`), used with `tetron leave <network-name>` etc. `tetron kick` requires an endpoint id (short id from `tetron status`), not a hostname or IP -- a deliberate restriction on a destructive command.

## Features

**tetron additions:**

- **Invite-key admission** -- invite-only closed networks. Coordinators mint single-use invite keys with `tetron invite <network> create`; joiners redeem them with `tetron join <invite-key>`.
- **Configurable overlay subnet** -- default `10.88.0.0/24` avoids Tailscale's `100.64.0.0/10`. Override per-network with `create --subnet` or node-wide with `config set subnet`.

**Inherited from rayfish:**

- **Dual-stack** -- stable IPv4 in the configured subnet (FNV-1a of identity) and stable IPv6 in `200::/7` (blake3 of identity, 120-bit, never rotates).
- **NAT traversal** -- direct connections with hole-punching, relay fallback via iroh. Optional Tor transport (`--tor`).

Run `tetron --help` (and `tetron <command> --help`) for the full surface: `create`/`join`/`leave`/`nuke`, `invite` (create/list/revoke), `admin`/`kick`, `config`, `status` (`--json`), `up`/`down`, and `completions`.

> **Removed from upstream rayfish**: file sharing and multi-device pairing, declarative apply layer (`apply`/`alias`), Magic DNS and all OS DNS mutation, userspace firewall, permissionless ("open") networks, hostname renaming, ephemeral members, and self-update. Packet filtering is the host firewall's job; name resolution is `/etc/hosts`'s job; copy files with `scp`/`rsync` over mesh IPs; upgrade by replacing the binary.

## Permissions

Like Tailscale, the daemon authorizes each command by the **caller's UID**, not file permissions. Read-only commands (`status`, `... show`) are open to any local user; mutating commands need root or the configured operator. The user who installs the service (`sudo tetron up`) becomes the operator automatically.

```bash
sudo tetron install | restart | uninstall   # manage the system service
sudo tetron start | stop                     # stop = fully offline; start = back online
sudo tetron set-operator <user>              # authorize a user to run tetron without sudo
```

`tetron up` / `tetron down` toggle only the data plane (near-instant standby); the daemon stays connected to peers across `down`.

## Upgrading

There is no self-update; upgrade by replacing the binary:

```bash
git pull && cargo build --release
sudo install target/release/tetron /usr/local/bin/tetron
sudo tetron restart
tetron version                 # confirm the new build (version + git sha)
```

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
tetron leave <network-name>       # optional: leave gracefully first, so the coordinator can prune you
sudo systemctl stop tetron
sudo systemctl disable tetron
sudo rm -rf /etc/tetron/          # wipe config + identity (back up first if you need the key)
sudo rm /etc/systemd/system/tetron.service
sudo systemctl daemon-reload
```

Don't run `sudo tetron nuke <network-name>` when uninstalling -- that destroys the network for everyone, not just your machine.

## Development

Developed with [Specification-driven development](https://en.wikipedia.org/wiki/Specification-driven_development) using [libspec](https://github.com/drhodes/libspec). Each requirement is a documented class in `spec/design_spec.py`; the `reconcile.py` gate enforces automatable constraints. Commits are recorded with `libspec link` so the spec keeps a complete history alongside the code.

## Relationship to upstream & license

tetron is a derivative of [rayfish](https://github.com/rayfish/rayfish), licensed under the **Mozilla Public License 2.0** (`LICENSE`), the same as upstream. The entire mesh-VPN design, and the overwhelming majority of the code, is rayfish's work; see the [changelog](CHANGELOG.md) for what this fork changes. If you want the general, upstream-quality version of configurable subnets, that belongs in rayfish itself -- this fork is a scrappier, personal-use variant.
