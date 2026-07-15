# Changelog

All notable changes to Torpedo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Invite-in-blob (BLOB-001)**: invites now ride in the signed `GroupBlob` instead of machine-local files (`InviteStore` superseded). Any network-key holder can mint, list, and revoke invites. The invite code drops the pinned coordinator endpoint (48 B vs 80 B) since every coordinator validates from the blob. Validation happens against the in-memory invite table; on redemption the entry is removed and the blob republished immediately. A narrow replay race window (~30-60 s DHT poll) is accepted for the initial implementation.

- **Peer address cache (CACHE-001)**: persistent transport-address cache at `<config_dir>/peercache.msgpack` so the mesh can re-establish direct QUIC connections without DHT lookups after an all-offline gap. Loaded at startup, seeded from live connections, saved every 5 min and on shutdown. Entries older than 30 days are pruned.

- **Overlap guard**: instead of the upstream rayfish preflight that refused to start if anything used `100.64.0.0/10`, tetron refuses to start only if the *chosen* subnet overlaps an existing local network. This lets tetron run alongside Tailscale or any other overlay without hijacking routing.

### Changed

- **rayfish relay/discovery presets retained (CON-001)**: the `"rayfish"` config keyword and its preset URLs (`relay.iroh.rayfish.xyz`, `dns.iroh.rayfish.xyz`) are kept as-is — they are load-bearing infrastructure references that must match upstream. The default remains n0's neutral infrastructure, but the keyword survives for users who pin to rayfish's hosted services.

### Fixed

- **IPv4 fragmentation for QUIC datagram size limits (FRAG-001)**: when Quinn's
   `max_datagram_size()` is below the TUN MTU (1280), IP packets larger than
   ~1192 bytes were silently dropped by `send_datagram` with a "datagram too
   large" error, stalling TCP connections (SSH key exchange failed at "expecting
   SSH2_MSG_KEX_ECDH_REPLY"). The forwarder now fragments oversize IPv4 packets
   into RFC 791-compliant IP fragments (each sent as a separate QUIC datagram)
   before the receiving kernel reassembles them. IPv6 fragmentation is not yet
   implemented; oversize IPv6 packets are dropped with a warning.

- **`--tor` flag now actually enables Tor transport (TOR-M01)**: previously the
   `--tor` flag on `torpedo create` and `torpedo join` was accepted by the CLI but
   silently ignored — the `transport` field was never threaded through the IPC
   handler, create/join functions, or persisted to config. It is now threaded end
   to end: the flag reaches `create_network`/`join_network`, is saved to the
   per-network `networks/<name>.toml`, and is restored on daemon restart
   (`restore_coordinator_network`) so Tor transport survives restarts as intended.

### Changed

- **Crate identity renamed to tetron (RENAME-M01)**: the library crate is now
  `tetron` (`[package] name = "tetron"`), the helper crate is `tetron-proto`,
  all `use rayfish::…` paths are `use tetron::…`, and the tracing filter is
  `info,tetron=debug`. Internal only — wire format is untouched (D1 preserved,
  mixed tetron/full-torpedo networks still work). The binary, service, and paths
  remain `torpedo`.
- **Hostname is fixed at join (MINIMAL-014)**: a member's hostname is set once
  when it joins (the coordinator still resolves collisions by appending
  `-1`/`-2`/…), and a member adopts that authoritative name from the signed
  roster. There is no longer a way to rename a member after join.
- **Admission is approval-only (MINIMAL-013)**: `torpedo create` always makes a
  closed network — the `--open` (and explicit `--closed`) flag is gone. A
  joiner dials the room id, lands in the pending queue, and a coordinator (or
  any co-coordinator granted with `torpedo admin add`) admits it with
  `torpedo requests` → `torpedo accept`/`deny`. This is now the only way onto a
  tetron-coordinated network.

### Changed

- **Phase 5 complete**: presentation and workspace cleanup. The CLI is now
  plain text (no ANSI colors or spinners; `--json` remains for machine
  output). The Android build (`ray-mobile`, `android/`) is removed, leaving
  a single-product workspace (binary `torpedo`, library `tetron`, helper
  `tetron-proto`). The `desktop` cargo feature is retired.

### Removed

- **Workspace trimmed (MINIMAL-016)**: removed `ray-mobile` workspace member and
  `android/` directory, trimmed `justfile` to the surviving deploy recipes, and
  removed the `desktop` cargo feature (no longer needed without the Android
  library build). Only `tor` remains as an optional feature.
- **Plain CLI (MINIMAL-015)**: removed style.rs, layout.rs, progress.rs and the
  `indicatif`/`crossterm`/`unicode-width` dependencies. CLI output is plain text
  with no colors, spinners, or interactive pickers. `--json` output is
  unaffected and remains on every read command.
- **`torpedo hostname` and `torpedo ephemeral` (MINIMAL-014)**: hostname rename
  propagation (the durable pending-rename intent and its redelivery) and the
  per-network ephemeral auto-kick TTL (auto-removing members offline longer than
  a configured duration) are gone. Remove a stale member manually with
  `torpedo kick`.
- **Invite minting (MINIMAL-013)**: `torpedo invite` and all its subcommands
  (`create`/`list`/`revoke`, `--reusable`/`--hostname`/`--expires`/`--qr`) are
  gone, along with the single-use invite ledger (`invites/<network>.toml`) and
  reusable-key minting. A tetron node can still **join** a full-torpedo network
  by an invite code or reusable key (`torpedo join <code>`), and a tetron
  coordinator still validates a reusable key that rides a full-torpedo signed
  roster — it just never mints one. Invite-share gossip from a full-torpedo
  co-coordinator is accepted on the wire and ignored (wire compatibility).
- **Magic DNS and all OS DNS mutation (MINIMAL-012)**: the `.ray` name
  resolver, the in-daemon DNS responder + port-53 intercept, and every OS-DNS
  integration (systemd-resolved / NetworkManager / resolvconf / the
  `/etc/resolv.conf` takeover, its inotify re-assert, and the panic-hook
  restore) are gone, along with the `magic-dns` and `dns-upstreams` config
  keys. **Reach peers by mesh IP** from `torpedo status` (or `--json`); host
  naming is `/etc/hosts`' job. Hostnames still ride the roster and show in
  `torpedo status`. The daemon's host footprint shrinks to: TUN device, routes,
  config dir, log dir, unix socket. The `.100.53` resolver address stays
  reserved (never assigned to a member) for wire compatibility with a
  full-torpedo node on a shared network.
- **Userspace firewall (MINIMAL-010)**: the entire `torpedo firewall` command
  (add/remove/show/default/reject/on/off, coordinator `suggest`, and the
  `pending`/`accept`/`deny`/`auto-accept` review flow), the per-device
  `firewall.toml`, the `auto_accept_firewall` per-network setting, and the
  `--auto-accept-firewall` join flag are gone. **Packet filtering is now the
  host firewall's job**: within a shared network every peer reaches every port
  a local service binds (mesh membership still gates *who* can connect).
  Restrict ports with nftables/ufw on the `torpedo` TUN interface — e.g.
  `nft add rule inet filter input iifname "torpedo" tcp dport != 22 drop`. The
  daemon still drops inbound packets whose source IP is not the sending peer's
  assigned mesh address (anti-spoofing). A coordinator running full torpedo can
  still ship firewall suggestions in the signed group blob; a tetron node
  carries them through on republish but does not act on them.
- **Declarative apply layer and local aliases (MINIMAL-011)**: `torpedo apply`
  (with `--example`/`--dry-run`/`--prune`/`--invite-missing`), `torpedo alias`,
  and `torpedo identityof` are gone, along with the per-network `aliases`
  setting shown inline in `torpedo status`. Reconcile a fleet with a script
  over `torpedo status --json`.
- **File sharing and device pairing (MINIMAL-004)**: `torpedo send`/`files`
  (and file auto-accept + download-dir/download-user settings) and
  `torpedo pair`/`unpair` (multi-device identity, encrypted key backup, and
  `--1password` backup/restore) are gone, along with device certificates, the
  `_torpedo_certgen` revocation floor, and the file/pair ALPNs. The identity
  model collapses to one device = one user. Copy files with `scp`/`rsync` over
  the mesh IPs, and back up the identity key yourself (it is one `0600` file
  under the config dir). Nodes on a shared network with a full-torpedo peer
  still decode-and-ignore its pairing/cert control messages, so the mesh keeps
  working across both variants.
- **Direct connect (MINIMAL-005)**: `torpedo connect`, `torpedo connections`,
  and `torpedo contact` (the contact-id friend-request flow) are gone, along
  with the `_torpedo_contact` DHT record and the connect ALPN. A private
  2-peer link is a normal 2-member network: create it and approve the join.
- **Observability export (MINIMAL-009)**: the Prometheus metrics endpoint on
  `:9090` and the `torpedo report` diagnostic-bundle command are gone. The
  daemon still logs to rolling files under the log directory, and traffic
  counters still appear in `torpedo status`.
- **Peripheral surfaces (MINIMAL-008)**: the `otel` cargo feature (OTLP span
  export), `torpedo open` deep links (the `torpedo://` scheme), and the
  append-only peer audit log (`audit.log`) are gone.
- **`torpedo ping` and `torpedo netcheck` (MINIMAL-006)**: the mesh echo-probe
  and endpoint-diagnostics commands are gone. Probe reachability with the
  system `ping` against a peer's mesh IP from `torpedo status`. Nodes still
  answer mesh Ping probes from full-torpedo peers.
- **mDNS local discovery (MINIMAL-007)**: the `torpedo mdns` command, the
  `mdns_enabled` setting, and LAN mDNS advertising are gone. Peer discovery is
  relays + pkarr; LAN peers still connect directly once discovered.
- **Embedded mesh SSH server (MINIMAL-003)**: `torpedo firewall ssh` and the
  in-daemon SSH server (with its userspace port-22 NAT) are gone, along with
  the russh/pty-process/uzers/socket2 dependencies. Remote shells are the host
  sshd's job: it listens on the mesh IPs like any other interface, so
  `ssh user@<mesh-ip>` keeps working with your normal keys and config.
- **Self-update (MINIMAL-002)**: the `torpedo update` and `torpedo auto-update`
  commands, the `install --auto-update` flag, the daemon's periodic update
  task, and the `auto_update` status field are gone. Upstream torpedo shipped
  this machinery disabled; tetron deletes it outright (along with the
  reqwest/rustls/self-replace/sha2/semver dependencies). Upgrade by replacing
  the binary and running `sudo torpedo restart`.

### Added

- **DNS takeover notice**: on a host with no systemd-resolved, NetworkManager,
  or resolvconf backend (for example a default Debian server install),
  `sudo torpedo up` now warns that torpedo is managing `/etc/resolv.conf`
  directly. The notice names the `/etc/resolv.conf.before-torpedo` backup and the
  restore path (`torpedo down` or `sudo torpedo uninstall`). Previously the
  takeover happened silently (daemon log only), so users discovered it only by
  noticing their `resolv.conf` had changed.
- **Ephemeral peer auto-kick**: a per-network policy that automatically removes
  members which stay offline longer than a configured time, the same as
  `ray kick`. Set it with `ray ephemeral <net> <duration>` (`12h`, `7d`, `1w`;
  minimum 1 hour), turn it off with `ray ephemeral <net> off`, and read it with
  `ray ephemeral <net> show`. Off by default; the current TTL shows on the
  network's line in `ray status`. Only the coordinator enforces it, and only
  offline peers are pruned, so it applies to open and closed networks alike (a
  removed peer can simply re-join or re-request later).
- **`ray unpair <device>`**: revoke one of your paired devices, for example a
  lost or stolen laptop. Run it from your **primary** device (the one you paired
  the others from). Device certificates now carry a generation; unpairing bumps
  your generation and publishes the new value (a single signed number) under your
  user identity, so every peer rejects any certificate below it — even on
  networks you do not run. Your **other** devices are automatically re-issued
  fresh certificates and keep working; only the removed device is left behind.
  The removed device is dropped from your networks, stops being treated as one of
  your own devices (no silent auto-admit, no own-device file auto-accept), and,
  if online and cooperative, is told to delete its own certificate. List your
  paired devices first with `ray pair list` (`--json` supported). Notes: a device
  that was **offline** while you unpaired is refreshed the next time it reconnects
  to your primary (until then other networks reject it); the generation stays
  published while your primary runs; and to fully retire a device from a network
  someone else runs, ask that network's coordinator to remove it too.
- **Consistent Android device name**: the phone now uses one device name across
  every network instead of a different random name per network. It is seeded from
  your device model on first run and can be changed in the You screen (the change
  applies to all your networks and to any you join later).
- **Android app exclusions and mesh IPv6 on the phone**: apps that break behind a
  VPN (Android Auto, Chromecast/Google Home, RCS messaging, GoPro, Sonos) now
  bypass the tunnel, so wireless Android Auto keeps working with Rayfish on. The
  Android tunnel also routes mesh IPv6 (the `200::/7` range), which previously did
  not work on mobile.
- **Android diagnostics**: the app now captures the mesh core's recent logs and
  reports lightweight health (networks, peers online, transport, and a WARN/ERROR
  count) to crash reporting automatically when the tunnel goes up or down and when
  the connection changes between wifi and cellular. A new "Send diagnostics" button
  in the You screen attaches the full recent log to a report so connection problems
  can be diagnosed. All of this respects the existing crash-reporting toggle; the
  toggle now reads "diagnostics". Diagnostic data (the log lines and recent errors)
  can include network addresses such as relay hosts and your device's public IP, so
  it is only sent while crash reporting is on.
- **Device ownership in `ray status`**: peer rows that are your own paired
  devices are now tagged `(your device)`, and a paired device belonging to
  another user is labelled `(user <id>)` (or shows that user's alias when you
  have set one) so it is clear which user each device belongs to. The `--json`
  output gains an `is_own_device` flag on each peer.
- **Opt-in automatic updates**: enable with `sudo ray install --auto-update` or
  `ray auto-update on`, and the daemon checks GitHub about every 6 hours for a
  newer **stable** release, then downloads, verifies (SHA-256), swaps the binary,
  and restarts itself onto the new version — no manual `sudo ray update`. Off by
  default; nightlies are never auto-installed. Applying an update restarts the
  daemon, which briefly drops the VPN (peers reconnect automatically), so it stays
  opt-in. A backoff guard means a bad release is retried at most once a day
  instead of looping. `ray status` shows when auto-update is on.
- **Auto-accept files from your own devices**: turn on
  `ray files auto-accept <network> on` (or join with
  `ray join <net> --auto-accept-files`) and incoming file transfers from your
  own paired devices land automatically in your `~/Downloads`, with no manual
  `ray files accept`. Only offers whose sender is one of your own devices (same
  paired identity) on that network are accepted; files from anyone else still
  queue for review. Turning it on also accepts any offers already waiting from
  your devices. Off by default; `ray files auto-accept <net> off` disables it.
- **Configurable auto-accept download location**: `ray files download-dir <path>`
  sends auto-accepted files to an absolute directory (owned by the dir's owner or
  `download-user`); `ray files download-user <user>` routes them to that user's
  `~/Downloads`, owned by them. With neither set, the operator's `~/Downloads` is
  used; if nothing resolves the offer stays queued rather than being written as
  root. `--clear` unsets; no argument shows the current value.
- **`ray alias <network> <key> <alias>`**: give a peer a friendly, node-local
  name. `ray alias <net> set <key> <name>` binds an alias to a user, where `key`
  is either an identity string (from `ray identityof`) or a currently-joined
  hostname. The alias then shows inline in `ray status` (as `host.net.ray
  [name]`) and seeds `ray apply`'s `aliases:` map, so a spec can reference the
  name without re-declaring it (the spec still wins on a name conflict).
  `ray alias <net> list` and `ray alias <net> rm <name>` manage the set. Aliases
  are local and display-only: they are never published to the network.
- **`ray kick <network> <peer>`**: coordinators can now remove a member from a
  closed network. Identify the peer by hostname, mesh IP, or short id. The member
  is dropped from the network's roster, and every node disconnects from it: the
  kicked peer is severed mesh-wide, not just from the coordinator. It cannot
  re-join the closed network without a fresh invite or approval (to bar it
  permanently, also revoke its invite or reusable key). Kicking is refused on open
  networks (where the peer could immediately re-join) and against another
  coordinator or yourself.
- **`ray firewall off` / `ray firewall on`**: a global switch to disable the
  userspace firewall on a device. `off` allows every mesh packet (rules and the
  secure default are bypassed; mesh membership still gates who can reach you, and
  spoofed source addresses are still dropped), for simple setups that don't want a
  second firewall layered on top of the host/kernel firewall. `on` restores
  enforcement. The disabled state is shown in `ray firewall show`.

### Changed

- **Prometheus metrics and OTLP traces now export under the `torpedo` name.** The
  metrics on `:9090` were published under the `rayfish`/`rayfish_peer` family
  prefixes and OTLP spans (the `otel` feature) under service name `rayfish`; both
  now use `torpedo`/`torpedo_peer`. If you scrape `:9090` or collect traces, update
  any dashboards, alerts, or collector filters that referenced the old names.
- **Magic DNS no longer seizes `/etc/resolv.conf` by default.** New setting
  `torpedo config set magic-dns off|auto|direct` (default `auto`). In `auto`,
  torpedo uses a clean split-DNS backend when one exists (systemd-resolved,
  NetworkManager dnsmasq, resolvconf) but otherwise leaves `/etc/resolv.conf`
  untouched instead of taking it over — so it no longer collides with another
  VPN that manages the same file (e.g. Tailscale on a minimal Debian host, which
  previously caused a mutual DNS forwarding loop that broke all name resolution).
  When it declines, `torpedo up` prints how to enable `.ray`: install
  systemd-resolved, or opt in with `magic-dns direct`. `.ray` names are a
  convenience only — reach peers by their mesh IP from `torpedo status` (the mesh,
  firewall, SSH, and file transfer never use system DNS). `direct` restores the
  old takeover behavior for hosts that want `.ray` and have no clean backend;
  `off` never configures system DNS at all. When `direct` takes over on a host
  running Tailscale, torpedo no longer forwards DNS to Tailscale's own resolver
  (any `100.64.0.0/10` overlay address is skipped) and recovers the real upstream
  from Tailscale's pre-takeover backup, so the two do not form a query loop.
  Note: while `direct` is active torpedo is the sole resolver, so Tailscale's own
  `*.ts.net` names stop resolving on that host — use `auto` (the default) or
  systemd-resolved if you need both.
- **`ray firewall show` clarifies the firewall is separate from your host
  firewall**: the output now notes that this is a mesh firewall applied on top of
  your host/kernel firewall (both must allow a packet), so it is not forgotten
  when auditing an OS firewall. Enabling mesh SSH with `ray firewall ssh on` now
  reminds you to authorize a peer with `ray firewall ssh allow` when none is set
  yet (the server rejects all logins until a peer is on the allow list).
- **Bounded pending-join queue** — on a closed network, the coordinator's queue
  of join requests awaiting `ray accept` is now capped (oldest request evicted
  when full), so a peer churning fresh identities can no longer grow it without
  limit. Legitimate queues are far below the cap, so this is invisible in normal
  use.

### Performance

- **Drop-newest under datagram backpressure** — when a peer's QUIC datagram send
  buffer is momentarily full, the new packet is dropped at the application
  boundary instead of letting QUIC evict an older already-queued one (drop-newest
  beats drop-oldest for a VPN), and the QUIC transport is tuned for the one
  datagram stream per peer shape. Keeps the send path non-blocking with no
  cross-peer head-of-line blocking.

### Fixed

- **`torpedo report` and the issue templates now identify as torpedo**: the
  diagnostic bundle (`/tmp/torpedo-report-*.tgz`), its sysinfo banner, and the
  pre-filled GitHub issue title/body said `rayfish`, so every bug report
  mislabeled itself as upstream. The bug-report template also pointed at the
  wrong log directory (`/var/log/rayfish`); it now names the real path
  (`/var/log/torpedo`, or `/Library/Logs/torpedo` on macOS). The changelog
  "Full Changelog" compare link now points at the torpedo repository instead of
  upstream.
- **`ray status` peer traffic counters now line up**: the per-peer up/down
  columns were packed into a single field, so the `↓` counter drifted from row to
  row and the block did not read as a table. Up and down are now their own
  right-aligned columns, so the arrows and digits line up down the list.
- **`ray firewall add --peer` now accepts any peer identifier**: previously it
  only matched a short id / endpoint-id prefix, so the natural things to type
  (`--peer alice`, `--peer alice.homenet.ray`, `--peer 100.x.y.z`) failed with
  "unknown peer". It now resolves a hostname, mesh IPv4/IPv6, short id, full
  endpoint id, or a paired user identity, the same way `ray ping`, `ray send`,
  and `ray firewall ssh allow` already do. It also fixes a case where an
  **inbound** rule scoped to a paired (multi-device) peer never matched: the rule
  is now keyed on the peer's user identity, so `allow in ... --peer alice` covers
  every one of that user's devices (an outbound rule stays scoped to the named
  device).
- **Member network vanished when the coordinator was offline at startup**: a
  member (non-coordinator) whose daemon restarted while its coordinator was
  unreachable would silently drop the network from its running state. `ray
  status` showed "no active networks" and the node rejected inbound mesh
  connections, and it stayed that way until it happened to restart again while
  the coordinator was online (its config was never lost). Restore now registers
  the network immediately from the verified group blob it already holds, whether
  or not the coordinator answers, and hands off to the reconnect loop to dial the
  coordinator back with backoff. The network stays visible in `ray status`
  (peers show offline) and reconnects on its own when the coordinator returns. As
  a side effect, a network no longer takes ~30s to appear in `ray status` after a
  member restart.
- **Mesh SSH host-key mismatch**: enabling `ray firewall ssh on` no longer makes
  `ssh <host>.ray` fail with a "REMOTE HOST IDENTIFICATION HAS CHANGED" warning.
  The embedded SSH server now presents the machine's existing OpenSSH ed25519
  host key (discovered via `sshd -T`) instead of a separate generated key, so
  clients that already trust the host keep matching the fingerprint pinned in
  their `known_hosts`. Hosts without a usable OpenSSH key fall back to a
  generated key as before.

## [0.1.4]

### Added

- **Mesh SSH (`ray firewall ssh`)**: Tailscale-style SSH with no SSH keys to
  manage. `ray firewall ssh on` runs an embedded SSH server on this node's mesh
  IPs (port 22); `ray firewall ssh allow <network> <peer>` authorizes a peer
  (hostname, mesh IP, short id, or `*` for any peer on the network) to log in.
  Connect with a stock client: `ssh user@host.ray`. The connecting peer is
  identified by its mesh identity (already proven by the encrypted mesh link), so
  there are no `authorized_keys` to distribute. Each grant restricts which local
  unix users the peer may log in as: `ray firewall ssh allow <net> <peer>` permits
  any **non-root** user by default, `--user alice,deploy` limits it to named
  accounts, and `--user '*'` permits any user including root. The check is by uid,
  so a uid-0 account under any name is blocked unless root is explicitly granted.
  `ray firewall ssh deny` revokes a peer; `ray firewall ssh show` lists state and
  per-network allow lists with their permitted users. As a security prerequisite,
  inbound mesh packets whose source IP is not the sending peer's assigned mesh
  address are now dropped (ingress anti-spoofing), so no peer can forge another's
  mesh IP.
- **Aliases and groups in `ray apply`**: a spec can now define optional
  top-level `aliases:` (a friendly name to a user's identity string) and
  `groups:` (a name to a list of aliases and/or hostnames), then reference them
  as firewall subjects or peers instead of listing every hostname. An alias
  names a person and expands to all of that person's currently-joined devices;
  a group expands to the union of its members. Expansion happens client-side at
  apply time, so the published rules are plain per-host suggestions. Aliases
  resolve only for members that have already joined (a `note:` is printed and
  the rule skipped until they do); literal hostnames still work before a host
  joins. `ray apply --dry-run` shows the fully expanded result.
- **`ray identityof <net> <host>`**: print a host's identity string (the value
  to paste into a spec's `aliases:`). Resolves to the user identity if the
  device is paired, else the device's transport identity. `--json` supported.

### Fixed

- **Accepted firewall suggestions no longer pile up duplicates.** Any change to a
  network's signed blob (a join, a rename, a new reusable key) re-materialized the
  whole suggested-firewall set and re-queued it for review, even the rules this
  node had already accepted. Accepting one of those repeats via the picker then
  appended a second identical rule. Already-installed suggestions are now kept out
  of the pending queue, and the picker merges by selector (newest wins), so a
  re-suggested rule replaces its predecessor instead of stacking.
- **`ray update` no longer bricks the system service.** After swapping its own
  binary, `ray update` rewrote the service unit using the path of the running
  executable, which Linux reports with a trailing `" (deleted)"` once the old
  binary is unlinked. The unit ended up as `ExecStart=/usr/local/bin/ray (deleted)
  daemon`, so the daemon crash-looped with `unrecognized subcommand '(deleted)'`
  and the node went offline until a manual reinstall. The path is now sanitized,
  making remote self-update safe.

## [0.1.3]

### Added

- **Custom relay, discovery, and DNS-upstream servers (`ray config`)**: override
  the default iroh relay and discovery servers, or the upstream resolvers used for
  non-`.ray` queries, with `ray config set relay|discovery-dns|dns-upstreams
  <value>`. Values are a comma list of presets (`rayfish`/`n0`), URLs, or IPv4s;
  the default augments the n0 defaults, `--replace` swaps them out, and `n0`/empty
  resets. `ray config get`/`unset` read and clear overrides. Applied on
  `sudo ray restart`.
- **`ray ping <peer>`**: active mesh diagnostics: sends live echo probes to a
  peer (by hostname, mesh IP, or short id) and reports per-probe round-trip
  latency, packet loss, and whether the path is direct or relayed. `-c/--count`
  and `-i/--interval` tune the probe run; `--json` emits the per-probe array.
  Unlike `ray status` (a passive snapshot), this verifies the round-trip works
  end to end.
- **`ray netcheck`**: local network diagnostics: bound UDP port (and whether
  it is the fixed forwardable port or an ephemeral fallback), home relay and its
  latency, public IPv4/IPv6 addresses, and whether UDP is working. `--json`
  supported.
- **Release notes on `ray update`**: before swapping the binary (and in
  `ray update --check` when behind), print what the update brings: the stable
  channel walks every release in `(current, latest]` newest-first, while
  `--nightly`/`--version` show the resolved release's notes. Best-effort, so a
  fetch failure never blocks the update.
- **Standby control plane (`ray up`/`down`)**: `ray down` now takes only the
  data plane offline (TUN, routes, Magic DNS, inbound forward gate) while staying
  connected to peers, so the node keeps receiving roster/blob/firewall updates and
  `ray up` is near-instant with no re-dial. `sudo ray start`/`stop` remain the
  fully-offline switch.
- **Fail-fast firewall REJECT mode**: `ray firewall reject on|off` (opt-in,
  default off): a denied packet gets a TCP RST / ICMP-unreachable reply in both
  directions so the initiator fails immediately ("connection refused") instead of
  hanging. Off keeps the stealthy silent-drop posture.
- **`ray start` / `ray stop`** service commands to bring the whole daemon online
  or fully offline.
- **Comma-list firewall ports + short CLI aliases**: `--port`/`-P` takes a
  single port, a `start-end` range, or a comma list (`80,443`, `22,8000-9000`)
  expanded to one rule per item.
- **Control-plane abuse defense**: per-connection token-bucket rate limiting that
  closes sustained flooders, with a per-network debounced reconverge worker so a
  trigger burst coalesces into a single pkarr resolve + reconverge.

### Changed

- **Richer daemon log files**: the rolling daily logs (bundled by `ray report`)
  now capture `debug`-level detail for Rayfish itself while the console stays at
  `info`, so diagnostics like hostname propagation are traceable in a report
  without re-running with `RUST_LOG`. Dependency logs stay at `info`; `RUST_LOG`
  still overrides everything.
- **Additive firewall suggestions**: each suggested token becomes one allow/deny
  rule with no synthesized catch-all (allow-list relies on the node's own inbound
  default-deny; denies-only = blacklist). `ray status` ends with a `pending`
  summary of things awaiting the user.

### Fixed

- **`ray hostname` rename now reliably propagates.** A member's rename is kept as
  a durable pending intent and re-delivered to a coordinator on every reconnect
  and reconverge until the signed roster confirms it, so the new name reaches the
  coordinator and all peers instead of sticking only on the renamed node. The
  renamed node keeps showing its new name across reconverges rather than briefly
  reverting to the old one.
- **`ray status` no longer shows `?` for a live connection's path.** A connection
  that is up but whose path iroh hasn't marked "selected" yet (during holepunch or
  migration) now reports its actual `direct`/`relay`/`tor` path instead of `?`.
- **`ray status` no longer glues a network's `join <room-id>` onto the last peer
  row.** The room-id line now prints on its own line.
- Publish the contact record regardless of data-plane state, so `ray connect`
  resolves a peer that is on standby (`ray down`).

## [0.1.2]

### Changed

- **Magic DNS reworked to TUN interception**: `.ray` queries are intercepted in
  the TUN read loop and answered in-daemon via the magic IP `100.100.100.53`, so
  the resolver never binds the host's port 53. Non-`.ray` queries forward to the
  captured upstreams.
- **Direct-mode DNS takeover (Tailscale-style)**: on hosts without split-DNS,
  take over `/etc/resolv.conf` with an inotify re-assert loop that repairs it in
  ~ms when NetworkManager/dhclient overwrites it, plus a `dns=none` NM drop-in so
  NM stops regenerating it. Both are marker-guarded and crash-safe (panic hook +
  next-start cleanup restore the host's DNS).
- **Sharded, atomic per-network config**: globals in `settings.toml`, each
  network in `networks/<name>.toml`, all written via temp-file + atomic rename.
  Replaces the single `networks.toml` whose non-atomic rewrites raced and silently
  dropped networks; legacy files auto-migrate on first load.
- Retain only the 7 most recent daily log files.
- Authenticate GitHub API calls in `ray update` with a `gh` token (lifts the
  anonymous rate limit).

### Fixed

- Scope suggested firewall rules to non-joined networks correctly, and default a
  suggestion's peer to "any" so rules propagate instantly.
- Point systemd-resolved (`SetLinkDNS`) at the magic IP; fix the NetworkManager
  mode read on Linux.

## [0.1.1]

### Added

- **Direct connections (`ray connect`)**: link two peers with no shared room id
  or invite via a rotatable, published **contact id**. `ray connect <contact-id>`
  sends a friend request; `ray connections [approve <id>]` reviews and admits it,
  minting a 2-peer network with the requester pre-approved. `ray contact
  [id|rotate]` prints or rotates the contact key.
- **Reusable invite keys**: `ray invite <net> --reusable [--expires]` mints a
  multi-use, expiring key that rides the signed `GroupBlob`, for unattended
  fleets (`ray join <key> --hostname H --auto-accept-firewall`). Revocation
  propagates via the blob.
- **Cross-coordinator invite gossip**: single-use invites are gossiped
  (`InviteShare`/`InviteUsed`) so any coordinator can validate and burn a
  cross-minted invite; combined with dial-fallback across the published
  coordinator set, fresh joins survive any single coordinator being offline.
- **Self-update (`ray update`)**: update from GitHub releases with SHA-256
  verification and atomic binary swap; `--check`, `--list`, `--force`,
  `--nightly` (rolling pre-release), and `--version V` (pinned, downgrades
  allowed). `ray version` / `--version` print the compiled version + git SHA.
- **Stable listen port**: the shared endpoint binds a fixed UDP port (41383) so
  it survives restarts and can be manually port-forwarded for guaranteed direct
  reachability, falling back to an ephemeral port if the port is in use.
- **CLI polish**: ANSI-aligned tables, progress spinners, an interactive
  `ray firewall pending` picker, and a global `--json` flag for machine-readable
  output.
- **Per-node firewall auto-accept**: `ray join --auto-accept-firewall` /
  `ray firewall auto-accept <net> on|off` to auto-install suggested rules.
- **IPv4 collision handling**: per-member `collision_index` with `assign_ip`
  rotation, index-aware validation, duplicate-IP rejection, and a deterministic
  reconverge tiebreak.
- **Opt-in QR invites**: `ray invite --qr` prints a scannable code.

### Changed

- **Secure-by-default inbound firewall**: unsolicited inbound TCP/UDP is now
  denied by default (inbound ICMP allowed, outbound allowed), with a stateful
  conntrack letting return traffic pass. `ray firewall default allow|deny` flips
  the inbound default.
- **Removed `trusted` networks** in favor of per-device, per-network firewall
  auto-accept; coordinators suggest rules on any network and nodes consent
  per-node (auto-accept or manual `ray firewall accept`/`deny`).
- **`ray apply` is YAML-only** (previously YAML/TOML/JSON), with each network
  mapping directly to its firewall subjects.
- **Mesh ALPN is versioned as the protocol-compatibility gate**: peers on
  different mesh versions share no common ALPN and can't connect. `ray join`
  pre-checks the coordinator's signed mesh version and dials surface an
  incompatible-version hint suggesting `ray update`.
- Roster and firewall state reconverge from the network-key-signed pkarr record,
  not from peer control messages (which are payload-free triggers).

### Fixed

- **ICMP conntrack** is now echo-type-aware, closing an inbound leak where reply
  packets could be treated as solicited.
- macOS routing: assert the IPv4 `100.64.0.0/10` route on activate, and install
  a loopback self-route so you can ping your own `*.ray` IP.
- Flush control-protocol QUIC streams and the pairing device-cert response so
  messages always reach the peer before the connection drops.
- `AdminGrant` keys are self-authenticated against the network public key.

### Performance

- Zero-copy TUN read and datagram forwarding path, with Criterion microbenchmarks
  (`benches/forward.rs`) over the per-packet data path.

## [0.1.0]

First public release.

### Added

- **P2P mesh VPN** over [iroh](https://iroh.computer): peers connect by
  cryptographic identity (EndpointId), not IP. NAT traversal, hole-punching, and
  end-to-end encryption are handled by iroh, with encrypted relay fallback.
- **Dual-stack addressing** derived from identity: stable IPv4 in `100.64.0.0/10`
  (FNV-1a) and stable IPv6 in `200::/7` (blake3, 120-bit, never rotates).
- **Networks & access modes**: closed by default; `--open` for public networks.
  Closed networks admit via one-time **invite codes** (`ray invite`) or **live
  approval** (`ray requests` / `ray accept` / `ray deny`). The room id is a
  discovery key, never an admission credential.
- **Coordinator / membership model**: single signed `GroupBlob` per network
  published to a per-network pkarr record; gatekeeper admission, member roster,
  and `MemberApproved` broadcast so the coordinator need not be online for a
  member's later reconnects.
- **Co-coordinators**: `ray admin add` grants the network key over the
  authenticated mesh, enabling multiple machines to publish the signed blob.
- **Magic DNS**: reach peers at `name.network.ray` (A/AAAA/PTR/SOA), rebuilt
  from the roster on every membership change.
- **Per-device firewall**: directional, protocol-, port-, and network-scoped
  rules with a stateful conntrack; `firewall.toml`.
- **Trusted networks**: coordinators can suggest firewall rules that ride the
  signed blob; nodes auto-take (`--allow-trusted`) or queue them for manual
  `ray firewall accept` / `deny`.
- **Declarative provisioning**: `ray apply <spec>` reconciles trusted networks +
  suggested firewalls from a YAML/TOML/JSON spec, with `--prune`, `--dry-run`,
  `--invite-missing`, and `--example`.
- **Multi-device identity**: `ray pair` (ticket-based), plus encrypted
  backup/restore, including optional 1Password storage of the encrypted blob via
  the `op` CLI (`ray pair backup --1password` / `ray pair restore --1password`).
- **File sharing**: `ray send` / `ray files accept` over iroh-blobs.
- **mDNS local discovery** (`ray mdns on|off`, default on).
- **Service management**: `ray up`/`down`, `ray install`/`restart`/`uninstall`,
  and the Tailscale-style operator model (`ray set-operator`).
- **Audit log**: append-only peer connect/disconnect events at
  `~/.config/rayfish/audit.log`.
- **Diagnostics**: Prometheus metrics on `:9090`, rolling daily logs, and
  `ray report` to bundle logs + metrics + sanitized status.
- **Optional transports / export**: `--features tor` (Tor transport) and
  `--features otel` (OTLP span export).

[Unreleased]: https://github.com/ErikAllanKincaid/tetron/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/rayfish/rayfish/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/rayfish/rayfish/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/rayfish/rayfish/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/rayfish/rayfish/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rayfish/rayfish/releases/tag/v0.1.0
