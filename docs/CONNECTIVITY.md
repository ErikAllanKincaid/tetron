# Tetron Connectivity: Direct, Relay, and Path Selection

## How Peers Connect

Tetron uses iroh as its transport layer. iroh is a P2P networking library designed
for the internet -- NAT traversal, hole punching, relay fallback. Every connection
between two tetron peers goes through one of three path types:

### Direct (UDP)

Two peers exchange packets directly, host to host. No intermediary. On a LAN this
means the switch or AP forwards packets between their IPs. Over the internet this
means their NAT gateways have been traversed (via STUN, UPnP, or hole punching).

- Lowest latency (sub-ms on LAN, 5-50ms across regions)
- Highest throughput (LAN gigabit, no relay bottleneck)
- No middlebox dependency

### Relay (HTTPS/TCP to a relay server)

Traffic between two peers is forwarded through a shared relay server. Both peers
connect outbound to the relay (HTTPS, rarely blocked by firewalls), and the relay
copies packets between the two outbound connections.

- Works through any firewall that allows HTTPS outbound
- Adds relay latency (5-15ms for same-region relay like `usw1-1.relay.n0.iroh.link`)
- Capped at relay server bandwidth
- Default relay: `https://usw1-1.relay.n0.iroh.link./` (n0's public relay, configurable
  via `tetron config set relay`)

### Tor (optional, behind the `--tor` Cargo feature)

Traffic routes through the Tor network. Rarely used on LAN. Requires a local Tor
daemon (`ControlPort 9051`).

## How iroh Decides: Path Selection

iroh does not try direct first. The connection flow is:

```
1. Discover peer via DHT/relay
2. Establish connection via relay (fast, reliable)
3. While relay is live, probe for direct paths
4. If a direct path is verified, switch traffic to it
5. If the direct path fails later, fall back to relay
```

**Relay is the starting state for every connection.** Direct is an upgrade that
may or may not happen, depending on network conditions.

The path selector inside tetron (`src/daemon/mesh/select.rs::choose_path_index`)
ranks paths in three tiers:

| Tier | Criteria | Example |
|---|---|---|
| 1 | Selected by iroh + in-subnet + has activity (or is the only trustworthy path) | Trust the path iroh picked, if it is actually carrying data |
| 2 | Has real activity + in-subnet | Any path that has proven it works, regardless of iroh's selection flag |
| 3 | In-subnet only, by class preference | Direct > Relay > Tor |
| Fallback | Any in-subnet path | Last resort |

## Why Two Peers on the Same LAN Might Stay on Relay

This is the most common connectivity question. Two machines on `192.168.1.0/24`,
same switch, same router, same desk -- and `tetron status` shows `via relay` for
both. The mesh works, but every packet goes out to the internet and back.

### Cause 1: Firewall Blocks Inbound UDP 43737 (Most Likely)

Tetron's daemon listens on UDP port **43737** (compiled default, overridable via
`tetron config set listen-port`). For a direct path to work:

- Peer A must be reachable at `192.168.1.x:43737` from Peer B
- Peer B must be reachable at `192.168.1.y:43737` from Peer A

If either machine's firewall drops incoming UDP on port 43737, iroh's direct
probe packets never reach the target. The relay connection (outbound-only HTTPS)
still works perfectly, so the mesh appears healthy -- just slower.

**Check it:**

```bash
# Linux: list nftables rules mentioning the tetron port
sudo nft list ruleset | grep 43737

# Linux: check ufw status
sudo ufw status | grep 43737

# Linux: quick connectivity test from another machine
nc -u -z -w 2 192.168.1.x 43737 && echo "port open" || echo "port blocked/filtered"

# macOS: check pf rules
sudo pfctl -sr | grep 43737
```

**Fix it:**

```bash
# Linux (nftables)
sudo nft add rule inet filter input udp dport 43737 accept

# Linux (ufw)
sudo ufw allow 43737/udp

# macOS -- the self-capture mitigation already loads pf rules;
# add a pass rule for inbound 43737.
sudo pfctl -a tetron -f /etc/tetron/pf.conf   # if you maintain one
```

### Cause 2: Address Discovery Gap (Less Common)

iroh learns a peer's addresses through the relay and DHT. It does not
automatically discover LAN addresses via mDNS or ARP. If the only addresses
iroh knows for a peer are its public IP (from STUN) and its loopback, it will
never try the LAN IP at all.

This typically shows up as a peer that is connected via relay, with no direct
paths ever appearing -- not even failed ones. Tetron currently has no command
to list the addresses iroh has discovered for a peer, which is exactly the
observability gap identified in the TODO (see section below).

**Workaround:** Ensure the tetron endpoint binds to the LAN interface. This is
the default when tetron binds `0.0.0.0:43737`. If you have multiple interfaces,
verify which one iroh is advertising:

```bash
# What addresses does this machine have on the LAN?
ip addr show | grep 'inet ' | grep -v 127.0.0.1
```

### Cause 3: Probation / Timing (Transient)

The direct path probe takes time. iroh needs to:
1. Tell the peer about the candidate addresses
2. Send probe packets on each candidate path
3. Wait for responses
4. Validate the path with QUIC's address validation
5. Signal the path selector

This typically completes within a few seconds, but during that window the
connection shows `relay`. If you check `tetron status` immediately after a
join or reconnect, relay is expected.

**Check:** Run `tetron status` again after 30 seconds. If the path has switched
to `direct`, it was just probation timing.

### Cause 4: Ephemeral Port Fallback

If UDP port 43737 is already in use when the daemon starts, tetron logs:

```
fixed UDP port unavailable; falling back to an ephemeral port
```

An ephemeral port is unpredictable and may not be the same across restarts.
If two peers each bind different ephemeral ports, iroh's address discovery
may advertise stale or mismatched port numbers, preventing direct connectivity.

**Check:**

```bash
# Look for the fallback warning in the daemon log
journalctl -u tetron --since "5 minutes ago" | grep "ephemeral"

# Or check what port the daemon is actually using
sudo ss -tulpn | grep tetron
```

**Fix:**

```bash
# Find what is using port 43737
sudo ss -tulpn | grep 43737

# Either stop the conflicting service, or use a different port
tetron config set listen-port 43738
sudo tetron restart
```

### Cause 5: Symmetric NAT or Port Isolation (Rare on Our Network)

Some managed switches have port isolation (PVLAN) that prevents host-to-host
traffic. Some corporate networks use symmetric NAT even on the internal network.
These are unlikely on a home LAN but worth knowing about if you ever deploy
tetron on a restricted network.

## Current Observability (What You Can See Now)

`tetron status --json` includes a `connection` block per peer with the winning
path's summary, the full candidate list (`paths[]`, `PATH-DIAG-002`), and
`via_detail` explaining why the winner isn't `Direct` (`PATH-DIAG-004`, `null`
when it is):

```json
{
  "conn_type": "Relay",
  "remote_addr": "https://usw1-1.relay.n0.iroh.link./",
  "rtt_ms": 12.5,
  "bytes_tx": 1048576,
  "bytes_rx": 2097152,
  "max_datagram_size": 1200,
  "paths": [
    {
      "conn_type": "Direct",
      "remote_addr": "192.168.1.42:43737",
      "is_selected": false,
      "in_subnet": true,
      "has_activity": false,
      "rtt_ms": null
    },
    {
      "conn_type": "Relay",
      "remote_addr": "https://usw1-1.relay.n0.iroh.link./",
      "is_selected": true,
      "in_subnet": true,
      "has_activity": true,
      "rtt_ms": 12.5
    }
  ],
  "via_detail": "DirectUnvalidated"
}
```

This example is exactly the "why relay" case this doc is about: a Direct
candidate exists at the right LAN address and is in-subnet (trustworthy, not
a bled overlay address -- `PATHBLEED-STATUS-003`), but has never actually
received traffic yet, so the already-proven Relay path wins for now --
`via_detail: "DirectUnvalidated"` says exactly that. Each `paths[]` entry's
`in_subnet`/`has_activity` are the same trust signals `choose_path_index`
itself uses (see `select.rs`'s own doc comment for the full tiering), not a
separate summary of them.

What this still does **not** tell you:
- What addresses iroh has *discovered* for the peer but never offered as a
  candidate at all -- a peer missing from `paths[]` entirely (not merely
  `has_activity: false`) is a discovery gap, not a validation-timing one, but
  there's no field naming iroh's raw address list to confirm that directly.
- How long the connection has been in its current state.
- Whether a direct probe was attempted and failed outright, vs. never
  attempted (both currently look like "no Direct entry in `paths[]`").

## Planned Observability (Not Yet Built)

Still open from the original TODO item ("Observability and control around
relay vs. direct connections", 2026-07-29) -- `paths[]`/`via_detail` above
covers the rest of that item's original field list:

| Field/command | Purpose |
|---|---|
| `known_addrs[]` | All addresses iroh has discovered for this peer (relay, DHT, STUN, local), independent of whether any became a `paths[]` candidate. Answers "does iroh even know the LAN IP?" directly instead of inferring it from a missing candidate. |
| `connection_age_secs` | How long the connection has been established. Scoped out once (`PATH-DIAG-003`) as not worth the added state for what it would answer; revisit if a real diagnosis needs it. |
| `direct_probe_state` | Has a direct probe been attempted? Succeeded? Failed? Never tried? |
| `tetron paths <peer>` | Dump all known paths and addresses for a specific peer outside `status --json`. |
| `tetron paths <peer> --force-switch` | Trigger path re-evaluation. |

None of these have been implemented yet.

## How to Tell the Difference (Field Guide)

| You see... | Likely cause | Action |
|---|---|---|
| `via relay`, port 43737 confirmed open on both sides | Probation timing or address discovery gap | Wait 30s, re-check status |
| `via relay`, port 43737 blocked by firewall on one side | Firewall | Allow UDP 43737 |
| `via relay`, port 43737 in use by another process | Port conflict | `tetron config set listen-port <alt>` |
| `via relay`, no `Direct` entry in `paths[]` at all, even after minutes, both ports open | Address discovery gap (iroh likely does not know the LAN IP) | No direct way to confirm yet (needs `known_addrs[]`, not built). Workaround: restart the daemon on both sides |
| `via relay`, a `Direct` entry exists in `paths[]` but `has_activity` stays `false` | Probe sent but never validated -- firewall or NAT blocking the direct path, not a discovery gap | Re-check causes 1 and 5 above |
| Path alternates between direct and relay | Connection migration or unstable network | Check for packet loss on the LAN |
| `via (you)` | That is you | Nothing to diagnose |

## References

### iroh documentation
- iroh relay overview: https://iroh.computer/docs/relay
- iroh endpoint API: https://docs.rs/iroh/latest/iroh/endpoint/index.html
- iroh connection paths: https://docs.rs/iroh/latest/iroh/endpoint/struct.Connection.html#method.paths
- iroh path events: https://docs.rs/iroh/latest/iroh/endpoint/enum.PathEvent.html
- iroh address discovery: https://docs.rs/iroh/latest/iroh/endpoint/struct.Endpoint.html#method.discovery

### iroh source (in-repo for tetron's pinned version 1.0.3)
- Connection API: `~/.cargo/registry/src/.../iroh-1.0.0/src/endpoint/connection.rs`
- Remote state / path tracking: `~/.cargo/registry/src/.../iroh-1.0.0/src/socket/remote_map/remote_state/`
- Net report / STUN: `~/.cargo/registry/src/.../iroh-1.0.0/src/net_report/`
- Relay transport: `~/.cargo/registry/src/.../iroh-relay-1.0.3/src/`
- NAT traversal strategies (NoQ): `vendor/noq-udp-1.1.0/` (tetron's patched copy)

### Tetron source
- Path classification: `src/daemon/mesh/diagnostics.rs::gather_conn_info`
- Path selection: `src/daemon/mesh/select.rs::choose_path_index`
- Connection info wire format: `tetron-proto/src/ipc.rs (ConnectionInfo)`
- Transport / endpoint setup: `src/transport.rs`
- Relay config override: `src/config.rs (ServerOverride, build_relay_mode)`
- Connectivity TODO: `DO-NOT-COMMIT/TODO.md` "Observability and control around relay vs. direct connections"

### External
- QUIC datagram RFC 9221: https://datatracker.ietf.org/doc/rfc9221/
- NAT traversal overview: https://tailscale.com/blog/how-tailscale-works
- n0 relay server: `https://usw1-1.relay.n0.iroh.link./` (tetron's compiled-in default)
- Alternative relay presets: `n0`, `rayfish`, or any `https://` URL via `tetron config set relay`
