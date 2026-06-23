# Rayfish

P2P mesh VPN powered by [iroh](https://iroh.computer). Connects peers by cryptographic identity (EndpointId), not IP address. Networks use dual-stack addressing: stable IPv4 in 100.64.0.0/10 (CGNAT, FNV-1a of identity) and stable IPv6 in 200::/7 (blake3 of identity, 120-bit, never rotates).

## Build

```bash
cargo -q build                 # add --features tor for Tor transport
cargo -q check
cargo -q test
cargo -q clippy
```

## Run

The daemon (`ray daemon`) owns the TUN device and iroh endpoint and runs as a system service. CLI commands talk to it over Unix-socket IPC.

```bash
sudo ray up                    # install+start the service, then activate the VPN
ray create [--name n] [--hostname h] [--tor]   # create network, prints join code (public key)
ray join <public-key> [--name alias] [--hostname h] [--tor]
ray leave <net> | nuke <net>   # nuke = publish empty record then leave
ray hostname <net> <name>      # change hostname on existing network
ray status                     # all networks (works without daemon)
ray up | down                  # activate / standby (TUN + DNS), daemon stays running

ray acl <net> tag|untag|allow|remove|show|apply ...   # coordinator-only network ACL
ray firewall show|default|add|remove ...               # per-device local firewall
ray mdns on|off                # local peer discovery (default on)
ray send <file> <peer>         # file sharing; ray files [accept <id> [--output dir]]
ray pair [<ticket>|backup|restore <code>]              # multi-device identity
ray completions <shell>
```

**Privilege & access (Tailscale operator model):** the always-root daemon does privileged work; clients are unprivileged. The IPC socket is mode `0666`; authority comes from a per-request `SO_PEERCRED` UID check in `DaemonState::check_authorized()`, not socket permissions. Reads (`status`, `*… show`, `files`) are open to any local user; mutating commands need root or the configured `operator_uid`; `set-operator` is root-only. Only `install`, `restart`, `uninstall`, `set-operator`, and `daemon` need `sudo`; everything else (incl. `up`/`down`) is IPC. `ray up`/`install` auto-grant operator to `$SUDO_USER`.

```bash
sudo ray install | restart | uninstall      # manage the service unit/plist
sudo ray set-operator <user>                 # authorize a user to run ray without sudo
```

### Cross-compile & deploy

```bash
just cross                     # build for x86_64 Linux
just deploy <ip>               # cross-build release + install + start daemon
just deploy-dev <ip>           # same, debug build
```

## Architecture

```
App → TUN (100.64.x.x / 200::x) → rayfish → iroh QUIC datagrams → peer
```

A single iroh Endpoint and TUN device are shared across all networks. Each network gets its own ALPN (`rayfish/net/<pubkey-prefix>`); the `ProtocolRouter` dispatches incoming connections by ALPN to per-network handlers.

### Modules

- `src/main.rs` — thin clap CLI + IPC client; service install/start (`cmd_up`, `install_and_start_service`), `cmd_install`/`cmd_restart`/`cmd_uninstall_service` (root-gated), `cmd_set_operator`, `cmd_pair`. `ray daemon` (hidden) runs the foreground daemon loop.
- `src/daemon.rs` — daemon process: `DaemonState` (endpoint + TUN + PeerTable + ProtocolRouter), `NetworkHandle` per active network, IPC server, accept handling (`CoordinatorAcceptState`/`MemberAcceptState` via `AcceptHandler`), reconnect loop, DHT publisher, group poller, activate/deactivate (up/down), nuke, ACL/firewall/file/pairing IPC handlers, DNS table updates.
- `src/ipc.rs` — `IpcMessage` enum (requests + responses), `MsgpackCodec` (length-prefixed msgpack over Unix socket), socket at `/var/run/rayfish/rayfish.sock`.
- `src/identity.rs` — persistent Ed25519 keypair (`~/.config/rayfish/secret_key`); device certs (`create/store/load_device_cert`).
- `src/membership.rs` — `IdentityProvider`, IPv4/IPv6 derivation, `MemberList`/`ApprovedList`, `GroupBlob { members, approved, acl }` with canonical msgpack + blake3 hashing; `Member`/`ApprovedEntry` carry optional `user_identity` + `device_cert`.
- `src/transport.rs` — iroh endpoint setup, per-network ALPN; optional Tor transport (`tor` feature).
- `src/tun.rs` — async dual-stack TUN (IPv4 /10 + IPv6 /128), split into `TunReader`/`TunWriter`; `configure_ipv6()` assigns the TUN's own IPv6 address at creation (Linux netlink via rtnetlink, macOS ifconfig); `route_peer_range()` installs the `200::/7` peer-range route into the TUN and **must run after link-up** (called from `DaemonState::activate()` post-`set_link_up`) — on Linux the kernel won't install an IPv6 connected route while the link is down, so peer traffic would otherwise leak out the host's default IPv6 route (Linux: rtnetlink `RouteMessageBuilder`; macOS: explicit `route add -inet6 -net 200::/7`). Idempotent across `up`/`down` cycles.
- `src/forward.rs` — TUN ↔ peer forwarding via dual-stack routing lookup; ACL + firewall enforcement; labeled drop counters; resolves transport keys to user identities via `DeviceUserMap`.
- `src/dht.rs` — one pkarr record per network (blob hash + seed peers); only the coordinator (per-network secret key) can publish.
- `src/control.rs` — length-prefixed msgpack control protocol over QUIC streams (Welcome, MemberApproved, MeshHello, BlobUpdated, …); `DeviceCert`, `PairMsg`.
- `src/peers.rs` — `PeerTable` (dual v4/v6 DashMaps), `DeviceUserMap`, ACL-aware `lookup_full()`.
- `src/config.rs` — network config (`~/.config/rayfish/networks.toml`): per-network secret/public key, `my_hostname`; `AppConfig.operator_uid`.
- `src/acl.rs` — identity/tag-based ACL engine + `.acl` file format; no rules = allow-all, any rules = deny-all except explicit allows.
- `src/firewall.rs` — per-device firewall (direction/proto/port/peer), `ArcSwap` for lock-free reads, dual-stack packet parsing; `firewall.toml`.
- `src/dns.rs` — Magic DNS server on `127.0.0.1:53` (A/AAAA/PTR/SOA for `*.ray`); forward `HostnameTable` + `ReverseLookupTable`.
- `src/dns_config.rs` — OS DNS config (`DnsConfigurator` trait). macOS: SCDynamicStore. Linux detection chain: systemd-resolved D-Bus → NetworkManager D-Bus → resolvectl → resolvconf → `/etc/resolv.conf`.
- `src/hostname.rs` / `src/network_name.rs` — hostname + local-alias generation and collision resolution.
- `src/stats.rs` — iroh-metrics `ForwardMetrics`/`PeerMetrics`, Prometheus export on `:9090`.
- `src/shutdown.rs` — SIGINT/SIGTERM via `CancellationToken`. `src/audit.rs` — append-only audit log (not yet wired in).

### Key flows

- **Create:** generate per-network `SecretKey` → derive addresses → build initial `GroupBlob` → publish blob + signed pkarr record → persist keys → print public key as join code.
- **Join:** resolve pkarr record → fetch + verify `GroupBlob` from a seed peer → apply members/approved/ACL → connect to peers with `MeshHello` → poll pkarr for blob updates.
- **Gatekeeper:** coordinator approves identities and broadcasts `MemberApproved`; any peer can then welcome an approved identity, so the coordinator need not be online when it joins.
- **DHT (single-record):** one pkarr record per network signed by the per-network secret key. The pkarr address *is* the network public key, so records can't be spoofed (MITM-resistant). `spawn_group_poller()` refetches the blob every 60s when the hash changes.
- **ACL / firewall:** ACL is coordinator-managed, distributed in the GroupBlob, enforced at the routing layer; firewall is per-device, first-match-wins, enforced after ACL. Paired devices resolve to one user identity via `DeviceUserMap`.
- **File sharing:** `ray send` adds the file to iroh-blobs and sends a `FileOffer` over `FILES_ALPN`; receiver queues it; `ray files accept` fetches the blob by hash and verifies it.
- **Pairing:** primary issues a ticket (`bs58(endpoint_id || secret)`) over `PAIR_ALPN`; secondary authenticates and receives a `DeviceCert` binding its transport key to the primary's user identity. Backup/restore encrypts the identity key (argon2 + chacha20poly1305).
- **Reconnection:** per-peer reader detects drop → coordinator removes the dead peer; joiner reconnects with exponential backoff (1s–30s) then re-sends `MeshHello`.
- **Leave:** `ray leave` gracefully closes its connections with `forward::LEAVE_CODE` before local teardown. Peers see `DisconnectEvent.intentional = true`: the coordinator prunes the member from the roster, republishes the blob, and reuses the join-time `broadcast_member_sync` so other members drop it immediately (the `MemberSync` receiver replaces the whole list); the 60s group poller is the backstop. A plain timeout/reset is *not* intentional, so an offline (but not departed) peer stays a known member.
- **up/down:** the daemon (endpoint, IPC, blob store, metrics) is always-on; the active VPN state (TUN up + system DNS + connected networks) is toggled by `activate()`/`deactivate()` tracked in `DaemonState.active`.
- **Tor (optional):** `--tor` adds `TorCustomTransport` alongside relay; onion address derived from the iroh `SecretKey`. Needs a Tor daemon (`ControlPort 9051`).

## Conventions

- Use `cargo -q` for all cargo commands; `tracing` for logging (INFO default, `RUST_LOG` to override).
- Never share I/O resources (TUN, sockets, streams) behind a Mutex — split into read/write halves. Avoid Mutex generally: prefer channels, atomics, or `RwLock`/`ArcSwap` for fast non-async state.
- ALPN per network: `rayfish/net/<pubkey-prefix>` (first 16 hex chars). File ALPN `rayfish/files/1`, pairing ALPN `rayfish/pair/1`.
- TUN MTU 1200. Wire format (control + IPC): 4-byte BE length + msgpack body.
- Join code = per-network public key string; local aliases (adjective-noun-noun) are display-only.
- Config under `~/.config/rayfish/`: `secret_key`, `device_cert`, `networks.toml`, `firewall.toml`, `acl/<network>.acl`.
- Always update docs (CLAUDE.md, docs/book.md, README.md) after finishing a feature or significant change.
