from libspec import Requirement, Constraint, UserStory

class ForkIntent(UserStory):
    """USER-STORY: FORK-INTENT

    Fork rayfish so its overlay IPv4 subnet is configurable at network-creation
    time, instead of hardcoded to 100.64.0.0/10, so it can run alongside an
    already-active Tailscale client on the same host.

    Priority: high.
    User journey: create a network with a custom --subnet -> join it from a
    second machine also running Tailscale -> both machines reach each other over
    the fork's mesh while Tailscale keeps working unaffected on both.
    Acceptance: `torpedo create --subnet <cidr>` succeeds on a host with an
    active Tailscale client; a second host joins successfully; `torpedo status`
    on both shows a live peer; Tailscale connectivity is unaffected throughout.
    """
    brief_title = "Configurable overlay subnet"
    priority = "high"


# ==========================================================================
# tetron: the minimal variant (MINIMAL-*, CON-M*)
#
# This repository is tetron, a stripped-down P2P mesh VPN. The original
# docs/PROPOSAL.md (rationale/design decisions) and docs/PLAN.md
# (commit-by-commit execution order) were retired 2026-07-27, migration long
# complete; see AGENTS.md for the current canonical description. Inherited
# SUBNET-*/RENAME-*/CON-* specs above remain
# valid until a MINIMAL removal commit retires them explicitly. New
# constraints use the CON-M* namespace so future full-torpedo CON-0xx
# numbers never collide on cherry-pick.
# ==========================================================================


class MinimalIntent(UserStory):
    """USER-STORY: MINIMAL-INTENT

    Strip torpedo to a single-purpose tool that connects machines into a
    private mesh network, delegating firewalling, name resolution, file
    transfer, remote shells, and updates to the host tools that already do
    those jobs well, and rename the product identity to tetron.

    Priority: high.
    User journey: install tetron on two machines -> create a network on
    one -> join from the other -> approve the join -> reach the peer by its
    mesh IP from `torpedo status` -> filter traffic with nftables on the TUN
    interface if desired.
    Acceptance: the CLI exposes exactly the surface in docs/PROPOSAL.md; the main
    crate is roughly 15k lines; a tetron node and a full torpedo node
    interoperate on one network; the trimmed e2e harness is green.
    """
    brief_title = "Minimal connect-only variant"
    priority = "high"


# --------------------------------------------------------------------------
# Requirements: scope and removals (MINIMAL-*)
# --------------------------------------------------------------------------

class MinimalScope(Requirement):
    """REQUIREMENT-ID: MINIMAL-001

    tetron provides exactly: identity, membership, mesh transport, TUN
    forwarding, closed-network admission with live approval, and a plain CLI
    (create/join/leave/nuke/requests/accept/deny/admin/kick/status/up/down/
    config/completions/version plus the sudo service verbs). Policy
    enforcement, naming, file transfer, remote shells, diagnostics probes,
    self-update, and multi-device identity are out of scope. Wire
    compatibility with full torpedo was preserved until RENAME-M02 severed
    it by changing the ALPN prefix; prior to that commit, protocol version 1
    and unchanged ALPNs allowed mixed networks.
    """
    req_id = "MINIMAL-001"


class RemoveSelfUpdate(Requirement):
    """REQUIREMENT-ID: MINIMAL-002

    Remove the self-update machinery entirely: src/update.rs,
    src/cli/update.rs, the `update`/`auto-update` CLI and the
    `install --auto-update` flag, and the deps it alone pulls (reqwest, the
    direct rustls handle, self-replace, sha2, semver). Full torpedo already
    ships it disabled (CON-006); in tetron absence replaces the gate,
    so CON-006 and reconcile.py's `self_update` value check retire in the
    same commit (replaced by the CON-M01 dependency-absence gate).
    """
    req_id = "MINIMAL-002"


class RemoveEmbeddedSsh(Requirement):
    """REQUIREMENT-ID: MINIMAL-003

    Remove the embedded mesh SSH server: src/ssh.rs, the userspace
    22<->30022 NAT in src/forward.rs, the `firewall ssh` CLI surface, the
    ssh_enabled/ssh_allow config keys, deps russh/pty-process/uzers, and
    tests/e2e/ssh. Remote shells are the host sshd's job, reached over the
    mesh IPs.
    """
    req_id = "MINIMAL-003"


class RemoveFilesAndPairing(Requirement):
    """REQUIREMENT-ID: MINIMAL-004

    Remove file transfer and multi-device pairing: daemon/mesh/files.rs,
    daemon/file_service.rs, cli/files.rs, cli/pair.rs, onepassword.rs,
    revocation.rs, the FILES_ALPN/PAIR_ALPN accept arms, the _torpedo_certgen
    pkarr record, and DeviceUserMap (identity model collapses to one device =
    one user). iroh-blobs STAYS: it transports the signed GroupBlob
    (reconverge.rs fetches it by hash over the blobs ALPN) and is core
    infrastructure, not a file-sharing extra. File copying is scp/rsync's
    job; key backup is the operator's job (the key is one file).
    """
    req_id = "MINIMAL-004"


class RemoveDirectConnect(Requirement):
    """REQUIREMENT-ID: MINIMAL-005

    Remove the direct-connect (friend request) flow: daemon/connect_service.rs,
    daemon/mesh/connect.rs, cli/connect.rs, CONNECT_ALPN, the _torpedo_contact
    pkarr publisher, and contact_secret_key. A 2-peer link is a 2-member
    network created and approved the normal way.
    """
    req_id = "MINIMAL-005"


class RemoveDiagnostics(Requirement):
    """REQUIREMENT-ID: MINIMAL-006

    Remove `torpedo ping` and `torpedo netcheck` plus
    daemon/mesh/diagnostics.rs. Reachability probing is ping/mtr's job over
    the mesh IPs. For wire compat (D1) a min node keeps a passive
    ControlMsg::Ping -> Pong responder so probes from full nodes still work.
    """
    req_id = "MINIMAL-006"


class RemoveMdns(Requirement):
    """REQUIREMENT-ID: MINIMAL-007

    Remove mDNS local discovery: spawn_mdns_discovery, the `torpedo mdns`
    CLI, the mdns_enabled config key, and the iroh-mdns-address-lookup dep.
    Discovery is relays + pkarr.
    """
    req_id = "MINIMAL-007"


class RemovePeripherals(Requirement):
    """REQUIREMENT-ID: MINIMAL-008

    Remove peripheral surfaces: the `otel` cargo feature and its optional
    deps, deep links (deeplink.rs, cli/open.rs, the torpedo:// scheme), and
    the audit log (audit.rs).

    The `tor` cargo feature is explicitly KEPT (see TOR-M01 for why and for
    the flexible per-network policy roadmap): Tor carries only TCP streams,
    so an iroh QUIC/UDP mesh can not be torified externally (torsocks,
    TransPort redirection, and gateway setups all drop UDP); the in-endpoint
    iroh-tor-transport glue is the only working integration, and it already
    delegates onion routing to the system Tor daemon (ControlPort 9051).
    It stays compile-time gated and off by default, so default builds carry
    zero Tor code. The existing per-network `--tor` flag and its semantics
    (endpoint-wide additive transport, effective after daemon restart) are
    kept unchanged through the MINIMAL phases.
    """
    req_id = "MINIMAL-008"


class RemoveObservabilityExport(Requirement):
    """REQUIREMENT-ID: MINIMAL-009

    Remove the observability export surface: the stats.rs Prometheus
    exporter on :9090 and `torpedo report` (build_report, the .tgz bundle,
    the pre-filled GitHub issue). Per-peer counters that status display or
    forward.rs logging still need are kept as plain fields. Logs stay
    (logdir.rs, rolling files); shipping them anywhere is out of scope.
    """
    req_id = "MINIMAL-009"


class ProactiveDropMonitor(Requirement):
    """REQUIREMENT-ID: LOG-002

    Add a proactive drop-rate monitor that warns when the number of drops
    per `DropReason` exceeds a configurable threshold within a rolling
    window, replacing the informational void left by the removed 30s
    ticker (LOG-001) with a genuinely event-driven alert that fires only
    when something is wrong.

    Design (Approach A, chosen 2026-07-30):

    A background task runs every `window_secs` seconds, reading and
    resetting a per-reason atomic counter. If the count for any reason
    meets or exceeds `threshold`, and the `cooldown_secs` has elapsed
    since the last warn for that reason, a single `tracing::warn!` is
    emitted with the reason name, count, window, and computed rate.

    This keeps the hot path free (one atomic increment per drop, already
    paid by the existing ForwardMetrics counter) and bounds log volume to
    at most one warn per reason per cooldown period during a sustained
    storm.

    Config surface (defaults = disabled):
      - `drop-monitor.window` — window in seconds (default 60)
      - `drop-monitor.threshold` — drops in window to trigger warn (default 0 = disabled)
      - `drop-monitor.cooldown` — seconds between warns for same reason (default 300)

    The monitor is off by default (threshold=0); a user who sets no
    `drop-monitor.*` keys gets zero new log lines.
    """
    req_id = "LOG-002"


class ConfigurableLogLevelAndConsoleFileDecoupling(Requirement):
    """REQUIREMENT-ID: LOG-003

    Found 2026-08-03 (`DO-NOT-COMMIT/Memory_bug_notes.md`): a real machine
    saw ~20% continuous CPU across Tokio worker threads, traced to
    `init_tracing()` (`src/main.rs`)'s file-layer default
    (`info,tetron=debug`) emitting a `tracing::debug!` for every single
    TUN packet read, fragment, and mesh reconvergence poll tick --
    formatting/writing that volume across 8 Tokio threads caused real
    logging overhead and thread lock contention. The same spam also
    defeats the file log's own diagnostic purpose: a log this verbose is
    not usable for finding anything in it. The `info,tetron=debug` split
    was set up for chasing a specific past bug, not a permanent
    architectural stance -- too much log is bad regardless of why the
    default was originally chosen.

    **Three parts:**

    1. **`forward.rs`'s five per-packet log lines reclassified from
       `debug!` to `trace!`**: `"TUN read"`, `"not IP, dropping"`,
       `"no peer for dst"`, `"routing to peer"`, `"datagram send
       failed"` -- every one of these fires on every packet (or every
       dropped/failed one), on the single busiest loop in the daemon.
       `reconverge.rs`'s periodic poller-tick `debug!` is untouched --
       once per poll interval per network, negligible by comparison, and
       still a legitimate "is the poller alive" signal at `debug`.
       Reclassifying these five is what makes `debug` genuinely useful
       again once turned back on: mesh/connection/admission-level
       detail (path opened/closed, reconverge, join/kick) survives at
       `debug` with no per-packet flood; `trace` remains available as
       the full per-packet dump for the rare case it's actually needed.

    2. **`tetron config set log-level <trace|debug|info|warn|error>`**
       (`CONFIG-AUDIT-002` style key): persisted override for the file
       layer's default, read once at daemon startup same as
       `log-retention`. Compiled default `info` (not `debug` --
       reclassifying (1) alone would still leave every TUN packet's
       classification/routing decision logged at `debug` by default,
       which is still needless log volume for a production default with
       nothing currently wrong). An operator chasing a live issue sets
       this, restarts, gets the detail, then unsets it -- no systemd
       drop-in required, matching how every other `CONFIG-AUDIT-002` key
       already works.

    3. **Console/file filter decoupling.** `init_tracing` builds
       `registry().with(global_filter).with(console_layer).with(file_layer)`
       -- `global_filter` is the *registry-level* ceiling (must be as
       permissive as the most verbose consumer, since `file_layer` has no
       filter of its own and gets whatever passes it); `console_layer`
       additionally narrows to its own `console_filter` on top of that.
       That architecture already supports independent console/file
       verbosity -- except both filters independently called
       `EnvFilter::try_from_default_env()`, and both read the *same*
       `RUST_LOG` variable, so setting `RUST_LOG` at all silently
       overrode `console_filter`'s own hardcoded `"info"` fallback too.
       Fixed by no longer having `console_filter` consult `RUST_LOG` (or
       the new config key) at all -- it is unconditionally `"info"`.
       `global_filter` (the file layer's effective level) now resolves,
       in order: `RUST_LOG` if set (kept as the raw, ecosystem-standard
       manual override, e.g. for a foreground `cargo run` session) → the
       `log-level` config key → compiled default `info`. Console/journal
       output stays a clean, readable summary regardless of either;
       verbose diagnosis happens in the file, which is written even for
       a foreground `tetron daemon` run (`to_file` is true for
       `Command::Daemon` specifically) -- so nothing is lost, a
       developer just tails the file instead of stdout when they want
       `trace`-level detail interactively.
    """
    req_id = "LOG-003"


class RemovePeriodicStatsLogger(Requirement):
    """REQUIREMENT-ID: LOG-001

    Remove the 30-second periodic stats logger from src/stats.rs
    (ForwardMetrics::spawn_logger's 30s ticker) that unconditionally emits
    `tracing::info!("(30s)")` every 30 seconds regardless of activity.
    Since the daemon runs as a systemd service this line hits the journal
    every 30s even when every delta is zero (all counters flat), producing
    persistent journald spam with no informational value.

    The counter infrastructure (ForwardMetrics, DropReason, record_*
    methods, drop_count, fragmented counters) is KEPT -- it feeds `tetron
    status --json`'s live on-demand traffic/drops/fragmentation display
    (STATUS-002/MTU-DIAG-001) and is not redundant. Only the unconditional
    periodic emit is removed.

    The shutdown summary ("session complete" with duration/rx/tx/total_bytes)
    is KEPT -- it is a meaningful bookend event (one line at daemon exit,
    no periodic noise) with no replacement from any on-demand command.

    The total_drops() helper that existed only to feed the ticker's delta
    computation is removed alongside the ticker (no other caller).

    Rationale: the daemon's remaining logging is already event-driven (peer
    up/down/reconnect, network lifecycle, errors). This ticker was the one
    leftover periodic-poll logger, predating `tetron status --json` now
    exposing the same counters on demand (STATUS-002/MTU-DIAG-001).
    """
    req_id = "LOG-001"


class RemoveFirewall(Requirement):
    """REQUIREMENT-ID: MINIMAL-010

    Remove the userspace firewall: firewall.rs, cli/firewall.rs,
    daemon/mesh/firewall.rs, reject.rs, picker.rs, firewall.toml, the
    auto_accept_firewall config key, the firewall benches, and
    tests/e2e/firewall. forward.rs keeps only the upstream anti-spoof
    ingress check. The IP-header parser the forwarder still needs
    (PacketInfo/parse_packet_info, for peer routing, anti-spoof, and the
    port-53 Magic-DNS intercept) is relocated out of firewall.rs into a new
    neutral src/packet.rs — it is packet parsing, not firewall logic.
    Packet filtering is nftables/ufw's job on the TUN interface; README
    states the posture change (every mesh peer reaches every port) loudly,
    with the nftables equivalent. Wire compat (D1): GroupBlob keeps its
    suggested_firewall field; reconverge ignores it and coordinator republish
    preserves it verbatim; ray-proto policy.rs/firewall.rs wire types stay.

    **Follow-up, 2026-07-17:** `GroupBlob.suggested_firewall` and
    `tetron-proto`'s `policy.rs` (`SuggestedFirewall`/`HostSuggestions`) were
    kept at the time for D1 wire compat -- a full-torpedo coordinator's
    suggestions carried through the blob verbatim, never acted on. RENAME-M02
    subsequently severed D1 (see that requirement's own addendum), which
    initially got this pair classified as "lower-confidence, lower-urgency"
    to remove (their justification rested on a weaker, contrived
    cross-product-key-migration scenario rather than RENAME-M02's flat ALPN
    impossibility). On reflection that distinction didn't hold up: the
    feature these fields served was already fully gone by this requirement,
    so neither one did anything in tetron regardless of D1 -- keeping them
    only added a wire-format field and a whole crate module for no purpose
    tetron itself has. Removed as part of the same follow-up pass as
    RENAME-M02's D1 cleanup: `GroupBlob.suggested_firewall` (and its
    threading through `canonical_group_bytes`/`group_blob_hash`,
    `NetworkState`, `JoinParams`, `RestoredRoster`, restore/reconverge
    adoption), plus `tetron-proto/src/policy.rs` in its entirety (deleted --
    nothing else in the workspace consumed `SuggestedFirewall`/
    `HostSuggestions`) and its `lib.rs` re-export.

    `tetron-proto/src/firewall.rs` (`Action`/`Direction`/`Protocol` enums)
    was audited at the same time and found to be a *separate*, independently
    dead remnant of this same requirement -- its own doc comment names the
    firewall IPC types (`FirewallState`, `FirewallRuleView`, `FirewallAdd`,
    `FirewallDefault`) this requirement already removed, and `policy.rs`
    never actually imported from it (`HostSuggestions.allows` used raw
    strings, not these enums). Flagged but deliberately not removed in this
    pass, matching the same scope discipline applied to `membership.rs`'s
    already-`#[allow(dead_code)]`-marked `policy_for_mode`/`OpenPolicy` --
    both are pre-existing, unrelated dead code discovered as a side effect,
    not part of what was being cleaned up.
    """
    req_id = "MINIMAL-010"


class RemoveApplyLayer(Requirement):
    """REQUIREMENT-ID: MINIMAL-011

    Remove the declarative apply layer (which exists to push firewall specs
    and dies with MINIMAL-010): apply.rs, cli/alias.rs, daemon/mesh/alias.rs,
    the `torpedo apply` / `torpedo alias` / `torpedo identityof` CLI (and their
    orchestrators, previously co-located in cli/firewall.rs), EXAMPLE_SPEC, the
    `Alias{Set,Remove,List,ListResponse}` IPC ops, the per-network `aliases`
    config field + its `NetworkStatus.aliases` projection + the inline
    `[alias]` status display, and the tests/e2e/apply scenario. Fleet
    reconciliation is a script over `torpedo status --json`.

    Sequencing (see PROPOSAL/PLAN): this lands BEFORE MINIMAL-010 even though
    the numeric order is the reverse. `apply`/`identityof` code lived in
    cli/firewall.rs and consumed the firewall-suggest IPC, so removing the
    consumer first keeps every commit compiling AND behaviorally coherent (the
    firewall is still fully present after this commit; a broken intermediate is
    avoided). The GroupBlob `suggested_firewall` field and ray-proto
    policy.rs/firewall.rs wire types are untouched here (D1).

    Follow-up (D-01, 2026-07-23): the removed `apply.rs` was the sole consumer
    of the external `config` crate (Cargo.toml's `config = { version = "0.15",
    ... }`, not this crate's own `src/config.rs` module) — confirmed via a full
    workspace search (no `use config::...`, `config::Config::builder()`, or any
    other symbol from the external crate anywhere in `src/`/`tetron-proto/src`,
    and `Cargo.lock` shows it resolved only as a direct dependency of the root
    package). Removed from `Cargo.toml`; added to `CON-M01`'s banned-dependency
    list (`reconcile.py`) so it cannot silently creep back in.
    """
    req_id = "MINIMAL-011"


class RemoveMagicDns(Requirement):
    """REQUIREMENT-ID: MINIMAL-012

    Remove Magic DNS and all OS DNS mutation: dns.rs, dns_config.rs,
    dns_resolver.rs, dns_packet.rs, daemon/dns_manager.rs, the port-53
    intercept in forward.rs, the magic-dns/dns-upstreams config keys, deps
    zbus/inotify, the panic-hook resolv.conf restore, and tests/e2e/dns.
    Peers are reached by mesh IP from `torpedo status`; naming is
    /etc/hosts' job (or a script over `status --json`). Hostnames remain in
    the roster (wire compat, status display). The daemon's host footprint
    shrinks to: TUN device, routes, config dir, log dir, unix socket.

    **Follow-up, 2026-07-17:** at the time this shipped, `membership::
    magic_dns_v4(subnet)` and the `is_reserved_ipv4` check that kept it out
    of the member IP pool were deliberately retained for D1 wire compat (a
    full-torpedo node on a shared network routes that address to its own
    resolver). RENAME-M02 subsequently severed D1 -- see that requirement's
    own addendum -- making the reservation's justification moot the same way
    it made the other D1-compat branches moot. Removed as part of the same
    cleanup pass: `magic_dns_v4`, `is_reserved_ipv4`, the skip-logic in
    `assign_ip`, and the reserved-IP check in `validate_member`, plus their
    three dedicated tests. `assign_ip` now only avoids IPs already held by a
    different member; `validate_member` only checks the CGNAT range and the
    network/gateway reservations.

    **Follow-up, 2026-07-17:** a CLI doc-comment-vs-handler audit found three
    remaining `--help` references to the removed feature: `Create.hostname`
    and `Join.hostname` both illustrated the hostname example as `"alice" ->
    alice.gaming.ray` (a Magic DNS `.ray`-domain label that has not existed
    since this requirement shipped); `Down`'s doc comment still said "take the
    data plane (TUN + Magic DNS) offline." Fixed all three (`main.rs`). Also
    found the same dead pattern in code, not just help text:
    `resolve_peer_name` (`runtime.rs`) still split its argument on `.` to
    accept a bare-or-qualified `alice.net.ray` hostname; since valid hostnames
    can never contain a `.` (`is_valid_hostname` is letters/digits/hyphens
    only), the split was permanently a no-op. Removed alongside the doc-
    comment fix documented in ADMIN-ADD-EASY-ID's own addendum.
    """
    req_id = "MINIMAL-012"


class ApprovalOnlyAdmission(Requirement):
    """REQUIREMENT-ID: MINIMAL-013  [PARTIALLY SUPERSEDED]

    NOTE 2026-07-14: The invite-removal part of MINIMAL-013 was applied
    (commit history shows the invite-free period) and then REVERSED when
    invite keys were brought back as the primary enrollment method. The
    INVITE-* requirements below document the restored invite system. The
    parts of MINIMAL-013 that still hold:
      - `tetron create` always makes a Restricted network
        (`--open`/`--closed` removed from CLI).
      - `GroupMode::Open` is still understood for D1 compat (auto-admit on
        full-tetron open networks), but tetron never creates one.
      - Joiner-side invite-code redemption (decoding an invite minted by a
        full-tetron coordinator) still works unchanged.
      - Reusable-key validation in membership.rs is kept as D1 compat.
      - `InviteShare`/`InviteUsed` from full co-coordinators are decoded
        and ignored on receipt (D1 compat).

    What was REMOVED and stayed removed:
      - `--open`/`--closed` flags on `tetron create`.
      - Reusable-key minting (validation-only survives).

    What was APPLIED and then REVERSED (invites are now fully present):
      - The single-use invite store (InviteStore, TOML files).
      - `tetron invite` create/list/revoke CLI.
      - InviteCreate/InviteList/InviteRevoke IPC ops.
      - `invite_create`/`invite_list`/`invite_revoke` daemon handlers.
      - The per-network `invite_lock` mutex was restored in the accept/join
        machinery.
      - The `initial_invite_key` auto-mint on create.
      - `redeem_invite_and_admit` as the primary admission gate.

    See INVITE-001 through INVITE-008 for the current design.
    """
    req_id = "MINIMAL-013"


class FixedHostnameNoEphemeral(Requirement):
    """REQUIREMENT-ID: MINIMAL-014

    Remove hostname rename propagation and the ephemeral auto-kick TTL.
    Deleted: the `torpedo hostname`/`torpedo ephemeral` CLI, the
    `SetHostname`/`SetEphemeral`/`GetEphemeral`(+`EphemeralStatus`) IPC ops,
    `MeshManager::set_hostname`/`announce_rename_to_peers`/`set_ephemeral`/
    `get_ephemeral`, the whole `src/daemon/mesh/rename.rs` (`pending_hostname`
    drain, `rename_satisfied`, `has_pending_hostname`),
    `spawn_stale_member_pruner`/`should_prune`, the `pending_hostname` and
    `ephemeral_ttl_secs` `NetworkConfig` fields, the status `ephemeral_ttl_secs`
    field + its status-line render, and the reconverge worker's 30s
    rename-backstop tick (now purely trigger-driven).

    Hostname is fixed at join: it is set once from the joiner's
    `JoinRequest`/`MeshHello`, the coordinator still resolves collisions
    authoritatively at admission (`admit_peer` -> `resolve_collision`), and a
    member adopts that authoritative name from the signed roster on reconverge
    via the trimmed `reconcile_local_hostname` (now adopt-blob-name only). The
    coordinator control reader no longer acts on a `MeshHello` hostname but
    still captures a full-torpedo peer's `device_cert` off it (D1).
    `outgoing_hostname` (announce the fixed name on reconnect) survives, moved
    from the deleted rename.rs into join.rs. `reconverge_and_apply` keeps its
    now-unused `alpn`/`my_ip` params (prefixed `_`) for call-site stability with
    torpedo. Manual `kick` remains the remediation tool for stale members.
    """
    req_id = "MINIMAL-014"


class PlainCliPresentation(Requirement):
    """REQUIREMENT-ID: MINIMAL-015

    Plain-text CLI output: remove style.rs, layout.rs, progress.rs and deps
    indicatif/crossterm/unicode-width/humansize/mime_guess. `--json` stays
    on every read command (the composable Unix interface). No colors,
    spinners, glyphs, or interactive pickers.
    """
    req_id = "MINIMAL-015"


class WorkspaceTrim(Requirement):
    """REQUIREMENT-ID: MINIMAL-016

    Trim the workspace to the one product: remove the ray-mobile member and
    android/ (the Android build reuses subsystems MINIMAL removes), reduce
    benches/ to the surviving forward path, prune cargo features to the
    default set, and sweep justfile/cliff.toml targets that reference
    removed surfaces.

    **Follow-up, 2026-07-17:** the ray-mobile *member crate* was removed, but
    15 doc comments across 6 files (`daemon/mod.rs`, `daemon/mesh/runtime.rs`,
    `daemon/mesh/bootstrap.rs`, `daemon/mesh/create_join.rs`,
    `daemon/mesh/diagnostics.rs`, `config.rs`) still named `ray-mobile` as a
    current consumer of the embedding API (`MeshManager::activate`/
    `attach_tun`/`detach_tun`/`shutdown_and_close`/`create_network`/
    `join_network`/`status`, the `DaemonState` legacy alias, and the
    `TETRON_CONFIG_DIR` Android override). The embedding API itself is not
    dead -- the `#[cfg(not(target_os = "android"))]` gates it exists for are
    still live, compiled code -- only the specific named example is gone.
    Reworded all 15 to describe the embedding API generically ("Part of the
    embedding API", "an embedder", "a mobile embedder") instead of citing a
    member crate that no longer exists in this workspace. Also fixed two
    unrelated staleness bits found in the same pass: `MeshManager::activate`'s
    doc comment still said it "configure[s] system DNS" / "configure[s] Magic
    DNS" (removed by `MINIMAL-012`; `activate`'s body has had zero DNS-related
    code since), and `bootstrap.rs`'s module doc said `handle_ipc_client`
    answers "`ray` CLI requests" (the binary is `tetron`; this particular
    phrasing didn't match `CON-010`'s `cli_reference_identity` regex because
    the character after "ray " was uppercase, so the automated gate never
    caught it).
    """
    req_id = "MINIMAL-016"


# --------------------------------------------------------------------------
# Dead-code sweep (TREE-SHAKE-*)
#
# Driven by the tiered audit in DO-NOT-COMMIT/AUDIT_dead-code-tree-shake_
# 2026-08-05.md and its same-day per-item code verification pass. The tree is
# warning-clean -- every piece of dead code here is hidden behind
# `#[allow(dead_code)]` or lives in the lib crate's `pub` surface -- so this
# is a semantic reachability sweep, not a "fix what rustc warns about" pass.
#
# Dependency ordering: TREE-SHAKE-001 through -005 are mutually independent.
# None consumes state, types, or symbols introduced by another; each touches a
# disjoint set of files (Cargo.toml / transport+create_join / the pending_pongs
# plumbing / membership.rs / comment-and-ignore-file text). They may land in
# any order, or in parallel, one commit each.
#
# Explicitly NOT in scope, and why:
#   - `images/torpedo2.png`. The audit filed it Tier 0 "safe to remove" as a
#     pre-fork brand asset; verification disproved that -- `README.md:3`
#     renders it as the README banner. It is live. At most a rename candidate
#     for a future branding pass, never a removal candidate.
#   - `NetworkState.mode` / `config::NetworkConfig::group_mode` (audit Tier 1
#     #1). Runtime-dead, but `group_mode` round-trips through
#     `networks/<name>.toml` on every node, so removing it is a config-format
#     migration (serde tolerance for the existing key, plus a testsuite
#     upgrade check), not a delete. Deferred to its own requirement.
#   - Everything in audit Tier 1 #2, Tier 2, and Tier 3: test fixtures kept by
#     design, deliberately-retained surfaces, and the KEEP-ON-PURPOSE list.
#
# These do not reintroduce anything a `MINIMAL-*` requirement removed; they
# finish removals those requirements left partially done (MINIMAL-010's
# firewall/QR surfaces, MINIMAL-016's stale-reference sweep precedent).
# --------------------------------------------------------------------------

class RemoveUnusedDependencies(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-001

    Drop the three direct `Cargo.toml` dependencies with zero references
    anywhere in `src/`, `tetron-proto/src/`, `benches/`, or `build.rs`:
    `serde_yml`, `qr2term` (the legacy terminal-QR invite surface, gone with
    the invite UX rework), and `async-trait`. Regenerate `Cargo.lock` in the
    same commit so `cargo build --release --locked` stays green.

    Every other direct dependency has real references and stays, including
    the ones an unused-dependency tool would misreport: `iroh-tor-transport`
    (optional, behind the `tor` feature, TOR-M01), `iroh-blobs` (GroupBlob
    transport, MINIMAL-004), `ratelimit` (HARDEN-004), `clap_complete` (the
    `completions` subcommand), and the vendored `noq-udp` path dependency.

    Independent of TREE-SHAKE-002..005.
    """
    req_id = "TREE-SHAKE-001"


class RemoveUncalledMeshHelpers(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-002

    Remove two uncallable helpers, each carrying `#[allow(dead_code)]` and
    each verified to have zero call sites repo-wide (the only grep hit is its
    own definition):

    - `transport::accept_connection_with_alpn` -- a leftover from an older
      accept path; the live accept path does not use it.
    - `daemon::mesh::create_join::try_dht_fallback_join` -- the file's own
      adjacent comment already calls it "this dead-code path" (MULTISEG-002
      era).

    Neither is part of the embedding API surface an external consumer could
    reach: `try_dht_fallback_join` is `pub(crate)`, and
    `accept_connection_with_alpn` is `pub` only because the whole module is.

    Independent of TREE-SHAKE-001, -003, -004, -005.
    """
    req_id = "TREE-SHAKE-002"


class RemovePendingPongsPlumbing(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-003

    Remove the `pending_pongs` map and all of its plumbing. The type is
    `Arc<DashMap<u64, oneshot::Sender<()>>>`, threaded through field
    declarations, clones, struct literals, and function parameters across
    `daemon/mod.rs`, `daemon/mesh/accept.rs`, `daemon/mesh/coordinator.rs`,
    `daemon/mesh/join.rs`, and `daemon/mesh/create_join.rs`. Verification
    found every one of those sites to be plumbing: there is **no `.insert()`
    anywhere in the tree**, so the two readers
    (`coordinator.rs` and `join.rs`, both `pending_pongs.remove(&nonce)`) can
    never hit and the map only ever holds nothing. Previously recorded as
    Finding #4 of the memory-leak audit.

    **Keep the `ControlMsg::Ping`/`Pong` wire variants.** They are alive and
    unrelated to this map: the passive Pong responder in `daemon/mod.rs`
    answers Ping probes sent by other nodes, and both `coordinator.rs` and
    `join.rs` handle inbound Ping. What is dead is the local
    wait-for-my-own-Pong bookkeeping that nothing ever registers into, not
    the liveness protocol itself. Removing the variants would break the wire
    format for peers that still probe us.

    Independent of TREE-SHAKE-001, -002, -004, -005.
    """
    req_id = "TREE-SHAKE-003"


class RemoveMembershipPolicyDeadWeight(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-004

    Remove the unused access-policy abstraction from `membership.rs`: the
    `MembershipPolicy` trait, its two implementors `OpenPolicy` and
    `RestrictedPolicy`, and the `policy_for_mode` dispatch function.
    Verification found no non-test, non-definition caller anywhere in the
    repo; `daemon/mod.rs`'s own comment on the adjacent `mode` field already
    names this the "same dead-weight class".

    The abstraction is unreachable by construction, not merely unused:
    admission is invite-only regardless of any policy (`LIVE-001`) and tetron
    never creates an `Open` network (`MINIMAL-013`), so no code path can
    consult a policy object even in principle.

    The only references are two unit tests that exercise the trait's own
    `allows_join` return value and nothing else. They encode no behavior that
    survives the removal, so they are deleted with it rather than rewritten.
    This is deliberately NOT the test-fixture-by-design case covered by
    `membership.rs`'s `validate_reusable`/`validate_invite` wrappers or
    `mesh/select.rs`'s `DialOutcome`/`pick_first_welcome`, whose tests encode
    a live spec over live logic -- those stay.

    Independent of TREE-SHAKE-001, -002, -003, -005.
    """
    req_id = "TREE-SHAKE-004"


class SweepStaleArtifactReferences(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-005

    Comment-and-config-text sweep for two classes of reference to files that
    no longer exist, in the same vein as MINIMAL-016's doc-comment pass:

    - `spec/design_spec.py`, which was split into
      `spec/{core,branding,addressing,membership,cli,security,constraints}.py`
      on 2026-07-28. Eight stale references remain, all inert comment or
      documentation text with no functional import: `daemon/mod.rs` (two),
      `daemon/mesh/runtime.rs`, `forward.rs`, `packet.rs`, `membership.rs`,
      `spec/main_spec.py`, `README.md`, and `CHANGELOG.md`. Repoint each at
      the module that actually holds the requirement it cites, rather than
      deleting the citation.
    - `.gitignore` entries for trees removed by MINIMAL-016 and the rename
      requirements: `/ray-proto/target` and the whole `android/` block
      (`android/.gradle/`, `android/build/`, `android/app/build/`,
      `**/jniLibs/**`, `android/local.properties`, `android/keystore.properties`,
      `*.jks`, `*.keystore`, `**/.cxx/`, `android/.idea/`, `*.iml`). Neither
      `ray-proto/` nor `android/` exists in the tree. The Android client now
      lives in the separate `tetron-mobile` repository with its own
      `.gitignore`, so these entries cannot become relevant again here.

    No behavior changes; nothing is compiled from any of it. Independent of
    TREE-SHAKE-001..004.
    """
    req_id = "TREE-SHAKE-005"


# --------------------------------------------------------------------------
# Modularization sweep (MODULARIZE-*)
#
# Driven by DO-NOT-COMMIT/PROPOSAL_codebase-modularization-sweep_2026-08-05.md
# (supersedes the earlier standalone PROPOSAL_modularize-membership_2026-08-05.md).
# Behavior-free: no call site, wire format, or test assertion changes, only
# where symbols are declared. Placed alongside TREE-SHAKE-* rather than in
# spec/addressing.py or spec/membership.py because, like the dead-code sweep,
# this is cross-cutting internal-structure maintenance, not a user-facing
# behavioral domain.
#
# Dependency ordering: MODULARIZE-002 assumes MODULARIZE-001's module layout
# already exists (it moves tests to match), so 001 must land first.
# --------------------------------------------------------------------------

class ExtractAddressingIdentityInvite(Requirement):
    """REQUIREMENT-ID: MODULARIZE-001

    Extract `src/membership.rs`'s pure overlay-addressing cluster into a new
    sibling module `src/addressing.rs`, mirroring the existing
    `spec/addressing.py` domain, and relocate two adjacent items whose
    previous home in `membership.rs` didn't match their nature:

    - **Addressing** (pure, stateless): `Subnet`, `default_subnet`,
      `resolve_subnet`, `subnet_change_warning`, `subnet_host_mask`,
      `subnet_netmask`, `ip_in_subnet`, `validate_subnet_matches_roster`,
      `subnets_overlap`, `next_available_subnet`, `subnet_gateway`,
      `parse_cidr`, the `cidr_opt` serde module, `derive_ip`,
      `derive_ip_with_index`, `assign_ip`, `derive_ipv6`,
      `IPV6_NETWORK_PREFIX_LEN`, `ipv6_network_prefix`, `ipv6_in_network` ->
      new `src/addressing.rs`. `validate_subnet_matches_roster` and
      `assign_ip` keep a dependency back on `crate::membership::{Member,
      MemberList}` for their signatures -- expected, not a defect; the
      re-export shim is what makes this safe regardless of which direction
      a given function's types point.

      NOT moved, despite being addressing-adjacent: `ensure_in_cgnat_range`.
      It is physically defined in the blob-validation section (used by
      `validate_member`/`validate_approved`), not the addressing section --
      an early draft of this requirement (the standalone proposal doc)
      miscounted it as part of the addressing cluster by line-range alone
      without checking the actual `fn` location; verified against HEAD
      before implementing and corrected here.
    - **Identity**: the `IdentityProvider` trait and `IrohIdentityProvider`
      struct -> the existing `src/identity.rs`, which already exists for
      exactly this kind of item.
    - **Invite record types**: the `InviteEntry` and `ReusableKey` struct
      definitions plus their `from_secret` constructors -> the existing
      `src/invite.rs`, which already holds the invite-code encoding logic
      that mints values of these types, so type and logic are no longer
      split across two files. The map-level `revoke_reusable`/
      `validate_reusable_key`/`revoke_invite`/`validate_invite` functions,
      and `GroupBlob`'s own `validate_reusable`/`validate_invite` wrapper
      methods, stay in `membership.rs` -- they operate on the whole
      `GroupBlob.reusable_keys`/`invites` maps, not just one type, and
      moving them would entangle this requirement with blob serialization
      instead of module placement.

    `membership.rs` keeps a `pub use` re-export of every relocated item (the
    pattern already proven in this file via `pub use tetron_proto::
    GroupMode`), so every existing `crate::membership::…` call site (146
    references across 18 files, verified against HEAD) compiles unchanged.

    No wire/serialization format change: `serde`'s derived output is driven
    by field names and `#[serde(...)]` attributes on the struct, not by the
    Rust module the struct is declared in, so `GroupBlob`'s canonical
    msgpack encoding and `NetworkConfig`'s TOML round-trip are unaffected --
    every derive attribute moved with its struct unchanged.

    Independent of MODULARIZE-002 in principle, but MODULARIZE-002 assumes
    this requirement's module layout already exists, so it must land first.
    """
    req_id = "MODULARIZE-001"


class SplitMembershipTestModule(Requirement):
    """REQUIREMENT-ID: MODULARIZE-002

    Split `membership.rs`'s single flat `mod tests` block (1,808 of the
    file's 2,925 pre-`MODULARIZE-001` lines) to match the module layout
    `MODULARIZE-001` establishes: tests for relocated addressing/identity/
    invite-type items move to their new modules (`src/addressing.rs`,
    `src/identity.rs`, `src/invite.rs`), colocated with the code they test,
    matching this repo's TDD convention (`docs/tetron-workflow.md` step 5).
    Tests for what remains in `membership.rs` (roster, `GroupBlob`, nuke
    consensus, tombstone, blob validation) stay in `membership.rs`'s own
    `#[cfg(test)]` module.

    Behavior-free: no test is added, removed, or changed in what it
    asserts, only which file it lives in and, where a moved test referenced
    a symbol now re-exported from `membership.rs`, updated to reference the
    symbol's new home directly.

    Depends on MODULARIZE-001 (assumes its module layout already exists).
    """
    req_id = "MODULARIZE-002"


# --------------------------------------------------------------------------
# Core-mesh pure-logic extraction (PURE-LOGIC-*)
#
# `create_join.rs`/`runtime.rs`/`join.rs` (the mesh create/join/lifecycle
# state machine) have zero unit tests -- flagged in
# DO-NOT-COMMIT/PROPOSAL_codebase-modularization-sweep_2026-08-05.md and
# TODO_DETAILS.md#core-mesh-zero-unit-tests. This is the no-new-deps
# alternative to reaching for iroh's test-utils feature
# (TODO_DETAILS.md#core-mesh-pure-logic-split): extract the genuinely pure
# decision logic embedded in those three files into
# `src/daemon/mesh/select.rs`, which already exists as this exact "pure
# decision helpers... no I/O, unit-tested directly" module
# (`coordinator_dial_order`, `find_subnet_collision`, `classify_candidate_addr`,
# `choose_path_index`, `classify_via_detail`, `persisted_roster` already live
# there). This requirement extends that established pattern rather than
# inventing a new one or a new module.
#
# Behavior-free: no call site's observable behavior changes, only where the
# decision logic is declared and that it is now independently testable.
# --------------------------------------------------------------------------

class ExtractCoreMeshPureLogic(Requirement):
    """REQUIREMENT-ID: PURE-LOGIC-001

    Extract four genuinely pure, currently-untested decision points from
    `create_join.rs`/`runtime.rs`/`join.rs` into `src/daemon/mesh/select.rs`,
    each as a small pure function plus unit tests in `daemon/mod.rs` (matching
    the existing per-function test-module convention there, e.g.
    `coordinator_dial_order_tests`):

    - **`solo_coordinator_nuke_outcome`** (from `runtime.rs::nuke_network`'s
      solo-coordinator branch): given `(cancel, second_present,
      has_other_members, force)`, decides `NothingToCancelOrSecond` /
      `WouldStrandMembers` / `Proceed`. The highest-value extraction --
      real branching logic on the network-destroying path, previously
      untested as a unit even though the lower-level primitives it's
      adjacent to (`nuke_consensus_reached`, `active_nuke_proposers`,
      `resolve_nuke_proposer`, `coordinator_count`) already are.
    - **`welcome_ip_collision`** (from `join.rs::perform_join_handshake`'s
      `Welcome` handling): given the just-received roster, `my_ip`, and
      `my_identity`, returns the colliding member's identity if some other
      identity already claims `my_ip` -- an IP-hijack check that previously
      ran inline inside an async handshake function.
    - **`next_backoff`** (from `join.rs::spawn_reconnect_loop`): the
      exponential-backoff-with-cap arithmetic (`(current * 2).min(max)`),
      previously inline and untestable in isolation from the reconnect
      loop's own async/tokio machinery.
    - **`reconnect_decision`** (from `join.rs::spawn_reconnect_loop`):
      given `(removed, prunes_member, was_pruned_locally)`, decides
      `Reconnect` / `IgnoreStaleDisconnect` / `PeerLeftDeliberately` /
      `PeerRemovedFromRoster` -- the three-way skip-or-reconnect branch a
      disconnect event goes through, previously embedded in the same loop
      as the actual redial I/O.

    Explicitly NOT attempted: `runtime.rs::leave_network`'s stranding
    computation (partitioning members into connected/unreachable requires
    an async `grant_admin_key` call interleaved with the decision, so it
    doesn't cleanly separate without deeper restructuring than this
    behavior-free pass should risk) and the propose-vs-execute consensus
    branch in `nuke_network` (already effectively backed by the tested
    `nuke_consensus_reached`/`active_nuke_proposers` primitives via the
    same comparison, so there is no untested logic left to extract there).

    No wire/serialization change: none of the four functions touch a
    serialized type. Every existing call site's behavior is unchanged --
    verified by `cargo test` still passing and a live `tetron-testsuite`
    regression pass (`AGENTS.md`'s mandatory core-change check) covering
    the actual create/join/nuke/reconnect paths these functions were
    extracted from.
    """
    req_id = "PURE-LOGIC-001"


class TorPerNetworkPolicy(Requirement):
    """REQUIREMENT-ID: TOR-M01  (post-MINIMAL, deferred)

    Flexible per-network Tor routing, as a per-network transport policy in
    networks/<name>.toml with three tiers of increasing isolation and cost:

    - `any` (default): clearnet UDP with relay fallback; current behavior.
    - `tor` (what `--tor` maps to today): the shared endpoint gains the Tor
      custom transport and dials for this network prefer onion addresses.
      Traffic-level Tor only: the shared endpoint still publishes clearnet
      addresses under the same endpoint id for its other networks, so a peer
      in the tor network can resolve our id to a real IP. This tier is
      censorship resistance / reachability, NOT anonymity, and the docs must
      say so.
    - `tor-isolated` (the new work): networks with this policy live on a
      SECOND iroh endpoint owned by the same daemon, with its own secret key
      (hence its own mesh identity and derived IPs), RelayMode disabled, no
      UDP address publishing, and onion-only discovery via the tor
      transport's address lookup. No clearnet address is ever published for
      that identity; this is the only leak-free per-network Tor. All
      tor-isolated networks share the one tor endpoint/identity (linkage
      among them is accepted and documented). MeshManager routes per-network
      ALPNs to the owning endpoint; the TUN stays shared.

    Deferred until after Phase 6: tier 3 touches bootstrap, MeshManager,
    create/join, and status, and must not ride along with the removal
    phases. Tiers 1-2 already exist upstream and are kept by MINIMAL-008.
    Policy is node-local routing, never a blob/protocol change (D1 was
    severed by RENAME-M02, but routing policy is inherently local).
    """
    req_id = "TOR-M01"


# --------------------------------------------------------------------------
# Invite-key admission (INVITE-*)
#
# MINIMAL-013 originally removed invite minting (approval-only admission).
# That removal was applied (committed) and then REVERSED: invite keys are
# restored as the primary enrollment method. The room id is discovery-only;
# an invite key is required to join (with the pending-queue fallback still
# present but secondary). See INVITE-007 for the current admission priority
# and the planned removal of the live-approval fallback.
#
# Reversal history: the INVITE-* requirements were applied on top of the
# invite-free state, restoring the InviteStore, invite CLI/IPC/daemon
# handlers, initial_invite_key on create, and redeem_invite_and_admit.
# The MINIMAL-013 requirement class above is marked PARTIALLY SUPERSEDED.
# --------------------------------------------------------------------------

class InviteKeyIntent(UserStory):
    """USER-STORY: INVITE-INTENT

    Replace live-approval admission with single-use invite keys as the
    primary way onto a network. A coordinator mints an invite key (a
    printable string), shares it out-of-band with whoever should join, and
    the bearer is auto-admitted on presentation -- no approval queue, no
    coordinator attendance required beyond minting.

    Priority: high.
    User journey: create a network -> mint an invite key -> share it with a
    collaborator -> they run `tetron join <key>` and connect immediately.
    Acceptance: `tetron invite <net> create` prints a usable key; joining
    with it succeeds without `tetron accept`; the invite is single-use
    (re-joining with the same key is denied). `tetron join <room-id>` alone
    fails with a message telling the user to obtain an invite key.
    """
    brief_title = "Single-use invite key admission"
    priority = "high"


# --------------------------------------------------------------------------
# Laptop fleet: making tetron work without an always-on member
#
# The three laptop fleet changes (CACHE-001, BLOB-001, COORD-001) let a
# network of laptop users who come and go operate reliably without an
# always-on member. The two-tier model (coordinator / member) is sufficient;
# no new roles are added.
#
# Implementation order:
#   1. CACHE-001 (peer address cache) -- standalone, quick win
#   2. BLOB-001 (invite in blob) -- core change, enables cross-machine invites
#   3. COORD-001 (multi-coordinator docs) -- already works, just document
# --------------------------------------------------------------------------

class LaptopFleetIntent(UserStory):
    """USER-STORY: LAPTOP-FLEET-INTENT

    Make tetron work for a network of laptop users who come and go with no
    always-on member. A member should be able to rejoin after an all-offline
    gap, join a network using an invite minted from a machine that is now
    asleep, and kick a departed member when the network creator is offline.

    Priority: high.
    User journey: Alice creates a network, mints an invite, grants Bob the
    network key via admin add. Everyone goes home for the night. Next morning
    Bob comes online first, can admit Carol (who has an invite from Alice)
    because the invite is in the blob, can reconnect without DHT because
    peers are cached, and can kick a stale member.
    Acceptance: `tetron join <invite>` works when the minting coordinator is
    offline but another coordinator is online; `tetron status` shows peers
    immediately after an all-offline restart; `tetron kick` works when any
    coordinator is online.
    """
    brief_title = "Laptop fleet operation"
    priority = "high"
