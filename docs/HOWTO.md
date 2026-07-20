# tetron HOWTO

P2P mesh VPN powered by [iroh](https://iroh.computer). This guide covers day-to-day operations for a tetron network where invite keys are the sole enrollment method (LIVE-001).

---

## Overview

tetron runs a root daemon that owns a TUN device and the iroh endpoint. Clients talk to it over a Unix socket. The daemon must be running and active before any mesh operations work.

```bash
sudo tetron install   # install the system service + start + activate data plane
```

---

## 1. Install from GitHub release

Download the binary for your architecture from the [releases page](https://github.com/ErikAllanKincaid/tetron/releases), then install it:

```bash
# Download the latest release binary. Published assets: tetron-linux-x86_64,
# tetron-linux-aarch64, tetron-macos-aarch64, tetron-macos-x86_64 -- swap
# the filename below for your platform:
curl -Lo tetron https://github.com/ErikAllanKincaid/tetron/releases/latest/download/tetron-linux-x86_64
# OR
wget -O tetron https://github.com/ErikAllanKincaid/tetron/releases/latest/download/tetron-linux-x86_64
chmod +x tetron
sudo install tetron /usr/local/bin/tetron

# Start the daemon (runs as a system service)
sudo tetron install

# Verify
tetron version
```

For a specific version instead of the latest, substitute the tag directly: `.../releases/download/v0.2.0/tetron-linux-x86_64`. A rolling pre-release build off the latest commit is also published under the `nightly` tag.

**Building from source:**

```bash
git clone https://github.com/ErikAllanKincaid/tetron.git
cd tetron
cargo build --release
sudo install target/release/tetron /usr/local/bin/tetron
sudo tetron install
```

---

## 2. Create a network and become coordinator

A network is always closed (approval-gated). The creator holds the network key and becomes the coordinator.

```bash
# Create a network. Your hostname is set once at creation. --network-name
# names the network itself (a random three-word name is generated if omitted).
tetron create --network-name mynet --hostname alice

# Output shows the network name, your mesh IP, and an initial invite key
# you can share immediately:
#   Created network "mynet" as 10.88.0.1
#   Invite key: t3tnR1vY3R... (expires in 7 days)
#   Share the invite key with peers so they can join.
```

The invite key printed at creation is a single-use invite that expires in 7 days by default. If you want a permanent invite instead, mint one explicitly with `--expires` (this flag lives on `invite create`, not `create` itself — see [section 3](#3-mint-invite-keys)):

```bash
tetron invite mynet create --expires never
```

**Custom subnet.** Every network gets its own TUN device and its own subnet — one network's subnet has no effect on another's, and there is no restart-required coherence check to satisfy. Override a specific network's subnet directly at create time:

```bash
tetron create --network-name mynet --hostname alice --subnet 10.77.0.0/16
```

Or change the **node-wide default** used by future `create`/`join` calls that don't pass `--subnet` explicitly:

```bash
tetron config set subnet 10.77.0.0/16
sudo tetron restart
tetron create --network-name mynet --hostname alice   # now defaults to 10.77.0.0/16
```

**Tor transport.** Route this network's traffic over Tor from the start:

```bash
tetron create --network-name mynet --hostname alice --tor
```

Requires a running Tor daemon with `ControlPort 9051` — see [Tor transport](#tor-transport) below.

---

## 3. Mint invite keys

As coordinator, you mint single-use invite keys for each new member.

```bash
# Default: 7-day expiry
tetron invite mynetwork create

# Explicit duration:
tetron invite mynetwork create --expires 24h
tetron invite mynetwork create --expires 30d

# Permanent invite (never expires):
tetron invite mynetwork create --expires 0
tetron invite mynetwork create --expires never

# Output:
#   Invite key: t3tnR1vY3R...
#   Invite id: a1b2c3d4e5f6 (use with `invite revoke`)
#   Expires at: 2026-07-21T18:00:00Z (or "never" for permanent invites)
```

**List outstanding invites:**

```bash
tetron invite mynetwork list
# Shows id, created date, expiry, and whether used

tetron invite mynetwork list --json   # machine-readable
```

**Revoke an invite before it is used:**

```bash
tetron invite mynetwork revoke a1b2c3d4e5f6
```

An invite is automatically revoked (marked used) when redeemed by a joiner. Revoked or expired invites cannot be redeemed.

---

## 4. Join a network

On the joining machine (already running `sudo tetron install`), use the invite key:

```bash
tetron join t3tnR1vY3R... --hostname bob

# Optional: give the network a local alias (shows in `tetron status`)
tetron join t3tnR1vY3R... --hostname bob --alias homelab

# Optional: route traffic through Tor
tetron join t3tnR1vY3R... --hostname bob --tor
```

The hostname is set once at join. The coordinator resolves collisions appending `-1`, `-2`, etc. if the name is already taken.

```bash
# If "bob" is taken, you are admitted as "bob-1"
tetron status    # shows your assigned hostname
```

**Bare room-id join is not supported.** tetron is invite-only (LIVE-001). A bare room id (network public key) is discovery-only — it is never an admission credential.

```bash
tetron join <room-id> --hostname bob
# Error: "a valid invite key is required to join"
```

If you only have a room id, ask a coordinator for an invite key.

**After joining, promote the new member to co-coordinator.** Every fully trusted member should hold the network key so there is no single point of failure for administration:

```bash
# On any existing coordinator:
tetron admin mynetwork add <short-id-from-status>
```

The grantee can then mint invites, admit joiners, and kick members independently.

---

## 5. Change your hostname

tetron fixes the hostname at join (MINIMAL-014). There is no `tetron hostname` command. To change it:

```bash
# Leave the network, then re-join with the new name
tetron leave mynetwork
tetron join <new-invite-key> --hostname newname
```

Note: leaving requires a new invite key to re-join because invites are single-use. Ask the coordinator for a fresh invite.

---

## 6. Discover other nodes

```bash
tetron status
```

Shows every network you are on, your mesh IP, and all known peers with their hostnames, mesh IPs, and connection status.

```bash
# Machine-readable JSON for scripting
tetron status --json

# Example: extract all peer IPs
tetron status --json | jq -r '.networks[].peers[].ip'
```

Hostnames ride the signed roster but there is no Magic DNS. Reach peers by their mesh IP from `tetron status`. If you want named access, export IPs to `/etc/hosts`:

```bash
tetron status --json | jq -r '.networks[].peers[] | "\(.ip) \(.hostname)"' | sudo tee -a /etc/hosts
```

---

## 7. Check peer connectivity

```bash
# List peers and see connection states
tetron status

# Direct ping over the mesh (ICMP)
ping 10.88.0.2

# TCP check (any service a peer is listening on)
nc -zv 10.88.0.2 22
curl http://10.88.0.2:8080

# Check which ports a peer can reach: within the mesh there is no
# userspace firewall — every peer can reach every port. Restrict ports
# with the host firewall on the TUN interface. The OS auto-assigns the
# interface name (tun0, tun1, ...) -- find yours with `tetron status`
# (the "interface" line per network) or `ip link show`:
#   nft add rule inet filter input iifname "tun0" tcp dport != 22 drop
```

**Is the daemon running?**

```bash
tetron status          # if the daemon is unreachable you get a connection error
sudo tetron start      # start the installed service
```

---

## 8. Administrative tasks

### Grant co-coordinator (recommended for every trusted member)

Multi-coordinator is the expected default. Every fully trusted member should be granted the network key so there is no single point of failure for admission, invite minting, or member management.

```bash
# List current key-holders:
tetron admin mynetwork list

# Promote a member to co-coordinator:
tetron admin mynetwork add <short-id-from-status>
```

The grantee becomes a co-coordinator immediately. They can mint invites, admit joiners, and kick members independently while the original coordinator is offline. Invites ride the signed `GroupBlob` (BLOB-001), so any coordinator can validate and admit -- the minting machine does not need to be online.

### Kick a member

```bash
tetron kick <network-key-from-status> a1b2c3d4e5  # both args are short ids from `tetron status`
```

`<network-key-from-status>` is the network's own key (the `network_key` line in `tetron status`, or an unambiguous >=10-char prefix of it -- also the same value shown by `tetron status --json`) -- not its local display name (`mynetwork`). The second argument is the target member's `endpoint_id` (from `tetron status --json`), never a hostname. Both values need at least 10 characters; neither accepts a local name, since kick is a destructive action and needs a cryptographic identity, not a mutable, spoofable one.

The kicked member is removed from the roster and disconnected. They cannot re-join without a new invite key.

### Leave or destroy a network

```bash
tetron leave mynetwork   # graceful leave: you disconnect and your config is removed;
                         # <net> here IS the local display name (leave isn't destructive
                         # to the network itself). If you are the network's only
                         # coordinator and other members exist, this auto-promotes every
                         # currently-connected member to co-coordinator first --
                         # --force is only needed if someone is offline right now and
                         # can't be reached (they'd be stranded; the error names them)

tetron nuke <network-key-from-status>    # coordinator only: publish an empty record, then leave.
                                          # Same short-id-only rule as kick -- see above.
```

**With a single coordinator**, `nuke` destroys the network immediately. **With two or more coordinators**, it requires consensus: the first `nuke` proposes instead of destroying outright, and the network is only actually destroyed once a *second, distinct* coordinator has also proposed (or explicitly seconded) within a 24h window. This stops one compromised or reckless coordinator from unilaterally destroying a network nobody else agreed to lose.

```bash
tetron nuke <network-key>              # propose (or second, if already proposed by someone else)
tetron nuke <network-key> --cancel     # withdraw your own pending proposal
tetron nuke <network-key> --second <short-id>   # explicitly second a specific coordinator's proposal
tetron status                     # shows any pending nuke proposal on the network
```

Other members see the network as gone on next reconverge once the tombstone is actually published (immediate on solo-coordinator destroy, or once consensus is reached).

### Create a zombie network (intentionally)

A "zombie" network is one left with no coordinator: the remaining members can still reach each other directly (existing P2P connections keep working), but nobody can ever admit a new joiner, mint an invite, kick anyone, or nuke the network again -- that requires the network's secret key, and once the last coordinator is gone, nobody can obtain it. By default `tetron leave` tries to *prevent* this (it auto-promotes every reachable member to co-coordinator first, so the network survives you leaving); a zombie only happens if you make it happen, on purpose:

```bash
tetron leave mynetwork --force   # skips auto-promotion entirely, even for members
                                  # who are online and reachable right now
```

**This is not reversible.** There is no command, no recovery flow, and no way for anyone -- including you -- to ever regenerate or reclaim the network's secret key once every coordinator is gone. The remaining members are frozen at whatever roster existed at that moment, permanently: no new members, ever; no removals, ever; no destroying it cleanly with `nuke`, ever. The only way out at that point is for every remaining member to abandon the network by hand (`tetron leave` on each of their own machines) and, if they still want a mesh, stand up a new one from scratch. Make sure this is really what you want before running `--force` here -- there is no undo.

`--force` is the only *deliberate* way to do this. (`sudo tetron uninstall` without running `tetron leave` first has the same effect unintentionally -- it tears down the service without ever attempting a handoff, so if you're a sole coordinator, uninstalling first zombifies that network by accident, with the same irreversibility as above. `tetron leave` each network before uninstalling if you want to avoid that.)

**Why you might want a zombie network:**

- **Deliberately freezing membership.** A small, fixed set of trusted peers (e.g. your own devices, or a few IoT nodes) where you want the roster to become permanently unchangeable once set up. Fewer key-holders means a smaller blast radius if any single device is ever compromised -- nobody, including a future you, can add or remove members again. This trades flexibility for a hard security ceiling.
- **Grace-period wind-down.** You're stepping away (leaving a company, retiring a project) and don't want to force an immediate decision on whoever's left, the way `nuke` would. A zombie network keeps existing connections alive while the remaining members figure out separately whether to keep using it, without you having to pick a successor.
- **Throwaway or test networks.** Make a scratch network for a short term project with a known set of participants. Create, mint invites for each, everyone sign in, make it a zombie. No way to get in, no new members ever, when last member leaves gone forever. Nobody cares if a scratch network becomes unreachable afterward, and running `nuke` or picking a successor is unnecessary ceremony.

If you actually want the network gone for everyone, rather than merely ungoverned, use `tetron nuke` instead (see above) -- it publishes an explicit tombstone, so even members who are offline right now detect the destruction cleanly the next time they reconnect, rather than the network just quietly decaying.

### Toggle data plane (standby)

```bash
tetron standby   # standby: TUN and routes go down, but daemon stays connected to peers
tetron resume    # re-activate: near-instant
```

Unlike `standby`, `sudo tetron stop` closes all peer connections (fully offline); `sudo tetron start` reconnects.

**Standby one network at a time** with `--network <name>` (the local display name shown in `tetron status`), instead of the whole VPN:

```bash
tetron standby --network work   # take "work" offline at end of day, "home" stays up
tetron resume --network work    # bring it back
```

`tetron status` shows a `·standby·` marker next to any network whose data plane is currently down.

---

## 9. Belonging to multiple networks

Every network you join gets its **own TUN device and its own subnet** — structurally the same as plugging a second physical NIC into a second physical network, not one shared interface juggling multiple meshes.

```bash
tetron create --network-name work --hostname alice
tetron create --network-name home --hostname alice --subnet 10.77.0.0/16
tetron status   # shows both networks, each with its own mesh IP for this node
```

**Networks do not route traffic to each other.** A node that belongs to both `work` and `home` does **not** automatically forward packets between them — each stays a fully isolated peer mesh, even though both interfaces live on the same machine. This is a real limitation relative to two physical NICs (where the kernel's own routing table would bridge them); building transparent cross-network routing is out of scope for tetron today.

**Jump-hosting already covers the practical need.** A node that's a member of both networks can bridge them at the application layer with zero extra configuration, since each hop is that node's own native connection to a peer it genuinely shares a network with:

```bash
# alice is a member of both `work` (reaching a `work` peer at 10.61.0.5)
# and `home` (reaching bob's laptop at 10.77.0.9). bob wants to reach the
# `work` peer through alice as a jump host:
ssh -J alice@10.77.0.9 user@10.61.0.5

# Port-forward instead of an interactive shell:
ssh -L 8080:10.61.0.5:80 alice@10.77.0.9

# Or run a SOCKS proxy through alice and point any app at it:
ssh -D 1080 alice@10.77.0.9
```

---

## 10. Custom configuration

### Custom relay or discovery servers

Override the default n0 relay and pkarr discovery:

```bash
# Custom relay URLs (comma list of presets, URLs, or IPs)
tetron config set relay my-relay.example.com:443

# Replace defaults entirely (don't augment)
tetron config set relay 203.0.113.1:443 --replace

# Custom pkarr discovery server
tetron config set discovery-dns dns.example.com/pkarr

# Reset to defaults
tetron config set relay
tetron config set discovery-dns

# All apply on daemon restart
sudo tetron restart
```

This only points tetron at a relay/discovery server; it does not stand one up. To run your own:

- **Relay** (NAT-traversal fallback, matches what `tetron config set relay` accepts): iroh's own relay server, `iroh-relay` (crate docs at [docs.rs/iroh-relay](https://docs.rs/iroh-relay/), source and self-hosting instructions at [github.com/n0-computer/iroh/tree/main/iroh-relay](https://github.com/n0-computer/iroh/tree/main/iroh-relay)). Build with `cargo build` from the iroh workspace; supports allow-everyone (default), an endpoint-id allowlist/denylist, a shared auth token, or an HTTP callout to an external auth service.
- **Discovery** (pkarr server, matches what `tetron config set discovery-dns` accepts): the `pkarr-relay` crate (`cargo install pkarr-relay`), source at [github.com/pubky/pkarr/tree/main/relay](https://github.com/pubky/pkarr/tree/main/relay), with an example config at [relay/src/config.example.toml](https://github.com/pubky/pkarr/blob/main/relay/src/config.example.toml) and the underlying design at [design/relays.md](https://github.com/pubky/pkarr/blob/main/design/relays.md). Runs on `http://localhost:6881` by default.

### Tor transport

Requires a running Tor daemon with `ControlPort 9051` enabled in `torrc`:

```bash
# Create a network with Tor transport
tetron create --hostname alice --tor

# Join a network with Tor transport
tetron join <invite-key> --hostname bob --tor
```

Mixing Tor and non-Tor nodes on the same network is supported — each peer uses whatever transport it specified.

---

## 11. Upgrading

```bash
# Replace the binary (no self-update in tetron)
sudo install /path/to/new/tetron /usr/local/bin/tetron
sudo tetron restart
tetron version   # confirm new build
```

---

## Troubleshooting

### "Connection refused" / daemon not running

```bash
sudo tetron start
tetron status
```

The daemon socket is at `/var/run/tetron/tetron.sock` on Linux (`/var/run/tetron.sock` on macOS). If the socket is missing, the daemon is not running.

### "No invite key provided" when joining

You are joining with a bare room id (network public key) but that network uses invite-only admission. Ask the coordinator for an invite key:

```bash
# Correct way:
tetron join <long-invite-key> --hostname bob

# The invite key is the full encoded string starting with
# something like t3tnR1vY3R..., not the short room id.
```

### "Invite rejected" / "invite not valid"

Possible causes:

- **Expired.** Invites default to 7 days. Ask for a fresh one.
- **Already used.** Single-use invites are burned on first redemption. Ask for a new one.
- **Revoked.** The coordinator revoked this invite. Ask for a new one.
- **Wrong network.** Double-check you are using the invite key from the correct coordinator.

### "Failed to parse invite code"

The invite key is malformed (not valid base58 of the expected length). Copy the entire string, no extra whitespace. If it was truncated by the terminal, scroll up to get the full key.

### Hostname collision

The coordinator appends `-1`, `-2`, etc. to resolve collisions. Check your assigned name:

```bash
tetron status    # shows your hostname in the network
```

If you want a different name, leave and re-join with `--hostname`.

### Peer shows "disconnected" in status

- Check that both daemons are running (`tetron status`).
- NAT traversal may take a moment for a direct connection to establish.
- If the peer is behind a restrictive NAT, traffic routes through the relay (still encrypted, higher latency).
- Check for firewall rules blocking UDP on the relay port (43737).

### Direct connection not establishing

tetron binds UDP port 43737 for the iroh endpoint. If this port is blocked by a firewall, forward it for reliable direct connections:

```bash
# Port-forward 43737/UDP on your router to this machine
# Or allow it through the local firewall:
sudo ufw allow 43737/udp
```

Without port forwarding, iroh still connects through its relay fallback (at the cost of higher latency).

### Viewing logs

```bash
# Daemon logs are at /var/log/tetron/ on Linux (/Library/Logs/tetron on
# macOS), rotated daily, 7 most recent kept:
sudo tail -f /var/log/tetron/*.log

# Or filter by our crate:
sudo journalctl -u tetron -f   # systemd journal, Linux only

# Panic traces are saved to panic.log in the log dir
sudo cat /var/log/tetron/panic.log
```

### "Permission denied" on a command

`status` and other read-only network commands are open to any local user. `config` (even `get`) and mutating commands need root or the configured operator:

```bash
# (Re)authorize yourself as operator (requires root):
sudo tetron set-operator $USER

# Commands that always need sudo, regardless of operator status:
sudo tetron install | restart | uninstall | start | stop
```

There is no command to query who the current operator is; `tetron install` auto-grants it to whoever ran it (`$SUDO_USER`), so re-running `set-operator` for the account you're using is always safe if a mutating command unexpectedly asks for root.

### "Address already in use" at daemon start

Port 43737 is taken. The daemon logs a warning and falls back to an ephemeral port. This prevents port forwarding from working reliably. Find the conflicting process and stop it, or change the listen port in the source and rebuild (not configurable at runtime).

---

## Other useful scenarios

### Multi-machine deployment script

```bash
#!/bin/bash
# Install tetron on a fleet of machines and join them all to a network.

NETWORK_NAME="${1:-homelab}"
INVITE_KEY="${2}"

# Step 1: Install the binary and start the daemon on each machine
for host in server1 server2 server3; do
  scp tetron "$host:/usr/local/bin/tetron"
  ssh "$host" sudo tetron install
done

# Step 2: Join each machine to the network using the invite key
for host in server2 server3; do
  ssh "$host" tetron join "$INVITE_KEY" --hostname "$host"
done
```

Each join consumes the invite key (single-use). Mint one invite per joining machine, or use `--expires never` if you batch them and want only one key for the batch.

### Custom subnet with Tailscale coexistence

tetron defaults to `10.88.0.0/24` specifically to avoid Tailscale's `100.64.0.0/10`. Both run side by side with no overlap:

```bash
tetron status                     # tetron's 10.88.x.x IPs
tailscale status                  # Tailscale's 100.x.x.x IPs
ping 10.88.0.2                    # reach a tetron peer
ping 100.x.x.x                    # reach a Tailscale peer
```

If `10.88.0.0/24` is already in use on your LAN, pick another uncommon slice:

```bash
tetron config set subnet 10.77.0.0/16
sudo tetron restart
# All future creates/joins use 10.77.0.0/16
```

### Generate /etc/hosts entries from active peers

```bash
tetron status --json | jq -r '
  .networks[]
  | select(.peers)
  | .peers[]
  | select(.hostname)
  | "\(.ip) \(.hostname)"
' | sudo tee -a /etc/hosts
```

Run this from a cron job or after network changes to keep names resolved.

### Check which invite keys are outstanding

```bash
tetron invite mynetwork list --json | jq '.[] | select(.used == false)'
```

Useful for auditing which invites have not been redeemed before they expire.

### Evaluate peer traffic stats

```bash
tetron status --json | jq '.networks[].peers[] | {hostname: .hostname, ip: .ip, tx_bytes: .tx_bytes, rx_bytes: .rx_bytes}'
```
