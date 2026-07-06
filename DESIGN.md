# torpedo — implementation design notes

Fork of rayfish (`upstream` remote, commit `9e142411`) making the overlay IPv4
subnet configurable so the node can run alongside an active Tailscale client,
plus a rebrand of this fork's own identity. Requirements/constraints are in
`spec/design_spec.py` (libspec); this file records the *how* and, importantly,
the two places where implementation reality forced a documented extension of the
original proposal (`SPEC_PROPOSAL_rayfish_fork.md`).

## Subnet representation

- In-memory type is `Option<(Ipv4Addr, u8)>` (base address + prefix length),
  exactly as SUBNET-001 specifies. `None` means "default", i.e. `100.64.0.0/10`,
  preserving today's behavior for the no-flag case.
- On the wire / on disk it is serialized as a **CIDR string** (e.g.
  `"10.88.0.0/16"`) via the shared `membership::cidr_opt` serde helper. This is
  required because `AppConfig` is TOML and a `(Ipv4Addr, u8)` tuple would be an
  illegal heterogeneous TOML array; using one string representation everywhere
  (GroupBlob msgpack included) keeps it uniform and human-readable.
- Helpers in `membership.rs`: `default_subnet()`, `resolve_subnet(opt)`,
  `subnet_host_mask(prefix)`, `subnet_netmask(prefix)`, `subnet_gateway(subnet)`.
  All host-bit / netmask / gateway math derives from a single `prefix` so the
  three can never disagree (the one place SUBNET-003 flags for care).

## Where the subnet lives (source of truth vs. operative cache)

- **`GroupBlob.subnet` (source of truth, SUBNET-001).** The signed, network-wide
  record every peer fetches and validates against. Carried in
  `canonical_group_bytes` / `group_blob_hash` so it is part of the signed bytes,
  and read back in `decode_group_blob` to validate member IPs against the
  network's own subnet.
- **`NetworkState.subnet`** mirrors the active network's resolved subnet so the
  daemon call sites (`assign_ip`, tiebreak, publish) have it without re-decoding.

### Documented extension #1 — `AppConfig.subnet` (node-level operative cache)

The proposal (§3.1) says to put the subnet on `GroupBlob` and warns against
`NetworkConfig`. That is correct for the *source of truth*, but it is not
sufficient on its own, because of an architectural fact the proposal did not
account for:

> The node has exactly **one** overlay IP and **one** TUN device, created once
> at daemon bootstrap (`bootstrap.rs`) from `identity.local_ip()` — before any
> `create`/`join` has chosen a network. For the node's TUN and derived IP to
> actually land in a custom subnet (the whole point — avoiding Tailscale's
> `100.64.0.0/10`), the subnet must be known at **bootstrap** time, not only at
> create time.

So the node's operative subnet is cached in `AppConfig.subnet` (node-global, not
per-`NetworkConfig`, so it is not the thing §3.1 warned against). `create
--subnet` and `join` write it; bootstrap reads it to build the
`IrohIdentityProvider` and TUN in the right subnet. `GroupBlob.subnet` remains
the authoritative network-wide value; `AppConfig.subnet` is a local read-through
cache of it. This keeps SUBNET-001 intact while making the fork actually work.

Consequence for the live test (Phase 7): a node picks up a newly-chosen subnet
for its TUN at the next daemon bootstrap. `create --subnet` persists it and
re-derives the creator's own IP in that subnet for the roster/blob immediately;
the single shared TUN reflects it after the daemon (re)starts. This is an
accepted limitation for a personal-test fork and is noted here rather than
papered over.

## IrohIdentityProvider

Gains a `subnet: (Ipv4Addr, u8)` field, set at construction (bootstrap reads it
from `AppConfig.subnet`, default otherwise). `local_ip()` and the trait
`derive_ip(&self, peer)` derive into that subnet, so the three trait call sites
(accept/join/create_join) need no signature change.

## Pure-function threading (SUBNET-003/004/005/007/008)

`derive_ip`, `derive_ip_with_index`, `assign_ip`, `is_reserved_ipv4`,
`validate_member`, `validate_approved`, `ensure_in_cgnat_range`,
`resolve_ip_tiebreak` all take an explicit `subnet: (Ipv4Addr, u8)`. No hidden
global. Test call sites pass `default_subnet()`.

## Conflict check (SUBNET-006)

`check_cgnat_conflict()` + `is_cgnat()` (tun.rs), its call at `bootstrap.rs`, and
the `use` in `daemon/mod.rs` are all removed. A fork deliberately choosing a
subnet outside `100.64.0.0/10` has nothing for this check to protect against, and
it is what currently refuses to start next to Tailscale.

## tun::create (SUBNET-005) and DNS (SUBNET-007/008)

- `tun::create` takes the subnet, computes netmask from `subnet_netmask(prefix)`
  and gateway from `subnet_gateway`, replacing the hardcoded `(255,192,0,0)` and
  `100.64.0.1`.
- `MAGIC_DNS_V4` becomes a function of the configured subnet (an offset within
  it) instead of the fixed `100.100.100.53`; assumes `/24` or larger.
- The PTR/reverse-lookup NXDOMAIN range check mirrors `ensure_in_cgnat_range`.
- The macOS branch of `route_peer_range` is left untouched (out of scope; Linux
  only per the proposal).

### Documented extension #2 — reconcile.py MAGIC_DNS grep

`MAGIC_DNS_V4` moves out of a literal into subnet-relative math, so the only
`100.100.100.x` / `100.64.0.0` literals remaining in the touched files are the
`default_subnet()` definition (written `100, 64, 0, 0`, comma form — does not
match reconcile.py's dotted-literal regex) and doc comments containing the
allowed `100.64.0.0/10` substring. CON-002 stays green.

## Rename (RENAME-001..004)

Renamed (this fork's own identity):

- **Binary** `ray` -> `torpedo`: `Cargo.toml` `[[bin]]`, the clap `name = "ray"`
  (what `--help` shows), and the systemd `ExecStart` path (RENAME-001).
- **systemd service** `rayfish.service` -> `torpedo.service` (file renamed) and
  every `systemctl`/`journalctl` unit reference, in `cli/service.rs`,
  `cli/update.rs`, `update.rs` (RENAME-002).
- **Paths / group** `/etc/rayfish` -> `/etc/torpedo`, `/var/log/rayfish` ->
  `/var/log/torpedo`, socket `/var/run/rayfish/rayfish.sock` ->
  `/var/run/torpedo/torpedo.sock`, log-file prefix `rayfish.log` ->
  `torpedo.log` (and the log-collector predicate that matches it), unix group
  `rayfish` -> `torpedo`, TUN device name, and the pre-`/etc` migration
  **repointed** to the fork's own `~/.config/torpedo` (so it never relocates a
  real rayfish install's config tree) (RENAME-003).

### Extension — RENAME-004 covers ALL wire identifiers, not just the net ALPN

The requirement's stated goal is that "this fork's wire traffic can never be
confused with genuine rayfish traffic." The proposal table listed only the
per-network ALPN, but that goal is only met if every wire-level identifier
changes. So `rayfish` -> `torpedo` was applied to: the mesh ALPN
(`torpedo/net/...`), the files/connect/pair ALPNs, the pkarr **DHT record
names** (`_rayfish*` -> `_torpedo*`), and the **mDNS** service name
(`_rayfish._udp.local` -> `_torpedo._udp.local`). The `transport.rs` ALPN test
was updated to match.

### Deliberately NOT renamed (would break a feature or is out of scope)

- **Cargo package/lib name `rayfish`** — the crate is still named `rayfish`;
  `main.rs`/`cli` reach the lib via `use rayfish::...` and the tracing filter
  keys on the `rayfish` crate name. Renaming the package is invasive and
  invisible to users. (Only the *binary* is `torpedo`.)
- **Relay/discovery preset `"rayfish"`** and its URLs (`relay.iroh.rayfish.xyz`,
  `config.rs`) — upstream's own hosted infrastructure (CON-001, §4.2).
- **`update::REPO_SLUG = "rayfish/rayfish"`** and the "rayfish release" messages
  — the auto-updater fetches from upstream's GitHub releases. Renaming would
  break it (external infra, same class as the relay preset). CAVEAT: do NOT
  enable auto-update on this fork — it would fetch and install an upstream
  rayfish binary over `torpedo`.
- **`rayfish://` deep-link scheme** (`deeplink.rs`) — only *parsed* (input),
  never generated/displayed; renaming touches the parser, many tests, and OS
  URI registration for no wire/functional benefit.
- **`dns_config.rs` resolv.conf markers** (`# Added by rayfish`, `tun-rayfish`,
  `rayfish-dns.conf`, `.before-rayfish`) — the tool's own Linux DNS-management
  markers; internally consistent (write and check both say `rayfish`) and out of
  the proposal's listed scope. Not wire traffic; left as-is.
- **OpenTelemetry service name / metrics labels** (`stats.rs`, `main.rs`) —
  §4.2 optional, cosmetic, no functional effect.
- **macOS** service/plist/SCDynamicStore identifiers (`com.rayfish.vpn`, etc.) —
  out of scope (Linux-only fork).
- Descriptive doc-comments that mention rayfish as the upstream project.
