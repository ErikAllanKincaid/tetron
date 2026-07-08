# torpedo — implementation design notes

Fork of rayfish (`upstream` remote, commit `9e142411`) making the overlay IPv4
subnet configurable so the node can run alongside an active Tailscale client,
plus a rebrand of this fork's own identity. Requirements/constraints are in
`spec/design_spec.py` (libspec); this file records the *how* and, importantly,
the two places where implementation reality forced a documented extension of the
original proposal (`SPEC_PROPOSAL_rayfish_fork.md`).

## Subnet representation

- In-memory type is `Option<(Ipv4Addr, u8)>` (base address + prefix length),
  exactly as SUBNET-001 specifies. `None` means "default" — originally
  `100.64.0.0/10` (preserving upstream's behavior for the no-flag case at the
  time this section was written). **Superseded by SUBNET-011**: the default
  is now `10.88.0.0/16`, an uncommon 10.x slice that avoids Tailscale's own
  `100.64.0.0/10` CGNAT claim even with no `--subnet` flag at all — the
  no-flag case is supposed to already avoid the collision the fork exists to
  fix, not merely support avoiding it when asked.
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
- The macOS branch of `route_peer_range` was left untouched here (out of
  scope; Linux only per the proposal). **Superseded**: TODO.md later reversed
  the scope call — "Decision: adapt both [macOS, Android] to torpedo rather
  than rip out" — so this is no longer a permanent exclusion, just an
  unfinished one. The macOS branch (and `route_self_loopback`) still
  hardcodes `100.64.0.0/10` today and ignores `--subnet`, is not compiled or
  type-checked on any Linux host or CI runner, and has never been built or
  run on a real Mac. That combination — untested, known-wrong subnet, real
  risk of misrouting a Mac's network config — is exactly why CI-002 gates the
  release/nightly workflows' macOS job off (`if: false`) rather than shipping
  it: this bullet and CI-002 describe the same open gap from two angles (the
  code isn't ready; therefore the pipeline doesn't publish it).

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
  break it (external infra, same class as the relay preset). **Strengthened
  since this was written**: rather than relying on the operator not to enable
  auto-update, `update::SELF_UPDATE_ENABLED = false` (CON-006) now hard-disables
  every self-update code path at compile time, so the risk this bullet warned
  about can no longer occur even by mistake.
- **OpenTelemetry service name / metrics labels** (`stats.rs`, `main.rs`) —
  §4.2 optional, cosmetic, no functional effect.
- Descriptive doc-comments that mention rayfish as the upstream project.

The following three items were listed here as deliberately-not-renamed when
this section was written, but were **all later renamed** (RENAME-006/007/008)
once their actual risk was better understood — kept below, struck through
their original reasoning, since the *why it changed* is worth keeping:

- ~~`rayfish://` deep-link scheme (`deeplink.rs`) — only parsed, never
  generated/displayed; renaming touches the parser, many tests, and OS URI
  registration for no wire/functional benefit.~~ **Renamed to `torpedo://`
  (RENAME-007).** The "no benefit" call undersold the cost of leaving it: a
  stray `rayfish://` handler is a real collision surface if a genuine rayfish
  install is ever present on the same host (exactly the class of problem this
  fork exists to avoid). `deeplink.rs` now parses `torpedo://` exclusively.
- ~~`dns_config.rs` resolv.conf markers (`# Added by rayfish`, `tun-rayfish`,
  `rayfish-dns.conf`, `.before-rayfish`) — internally consistent and out of
  scope; not wire traffic; left as-is.~~ **Renamed to `torpedo` (RENAME-006)**:
  `# Added by torpedo`, `tun-torpedo`, `torpedo-dns.conf`, `.before-torpedo`.
  This one mattered in practice, not just in theory — a live DNS incident on a
  test machine was traced to a **stale pre-rename install** still writing the
  old `# Added by rayfish` marker and fighting the current `torpedo` daemon
  for `/etc/resolv.conf`; having distinct, current markers is exactly what
  made that misconfiguration diagnosable instead of silently confusing.
- ~~macOS service/plist/SCDynamicStore identifiers (`com.rayfish.vpn`, etc.) —
  out of scope (Linux-only fork).~~ **Renamed to `com.torpedo.vpn`
  (RENAME-008)**, done anyway despite the "Linux-only" framing at the time —
  see the macOS `route_peer_range` note above for the actual current state of
  macOS support (identity renamed; routing logic still not fixed).

## CI / release pipeline (RENAME-012, CI-001..003)

`.github/workflows/{release,nightly,ci}.yml` were inherited from upstream
verbatim and never adapted after the binary rename — nobody had tried to
actually cut a release on this fork until now, so the drift went unnoticed.

- **RENAME-012**: both `release.yml` and `nightly.yml` packaged
  `target/<target>/release/ray` and asset names like `ray-linux-x86_64`,
  `rayfish-android.apk` — the `cp` step would fail outright the moment either
  workflow ran, since this fork's `Cargo.toml` renamed the bin target to
  `torpedo`. Fixed to `torpedo`/`torpedo-linux-x86_64`/`torpedo-android.apk`.
  Deliberately **not** touched: `src/update.rs`'s `release_asset_name`
  (`ray-{os}-{arch}`), which names assets on **upstream's** rayfish/rayfish
  releases for the disabled self-updater (see the REPO_SLUG bullet above) — a
  different `ray` than this one, and out of scope for the same reason.
- **CI-001**: `ci.yml` and `nightly.yml` both triggered on
  `push: branches: [master]`, but this repo's default branch is `main` —
  neither had ever fired on an ordinary push. Confirmed live: before this fix,
  GitHub Actions had likely never executed on this fork at all; `reconcile.py`
  (run locally) was the only gate actually exercised.
- **CI-002**: macOS and Android release jobs are unfinished/unsafe to publish
  (macOS: see the `route_peer_range` note above; Android: the deep-link
  scheme is broken, ironically the mirror image of the RENAME-007 fix above —
  the *Rust* side now parses `torpedo://`, but `AndroidManifest.xml` still
  registers `rayfish://`, so the two ends of the same feature disagree).
  Rather than delete the working job definitions or ship known-broken
  binaries, both are kept (with RENAME-012's identity fixes already applied)
  but gated `if: false` at the job level in both workflows, so re-enabling
  either is a one-line flip once that platform is actually finished and
  tested on real hardware.
- **CI-003**: `nightly.yml` was changed from `push`-triggered to
  `workflow_dispatch`-only. Many pushes to this repo are doc/spec/TODO-only;
  an automatic trigger would rebuild and move the shared `nightly` tag on
  every one of those. A nightly build is now a deliberate action (Actions tab
  -> "Run workflow", or `gh workflow run nightly.yml`) rather than an
  implicit side effect of committing. `release.yml` never had this problem —
  it triggers on tag push / manual dispatch, not a branch push.

Live-verified 2026-07-08: pushing 3 commits to `main` produced exactly one
workflow run (`CI`, `event=push`) and no `Nightly` run — confirming CI-001's
branch fix and CI-003's manual-only change both behave as designed.
