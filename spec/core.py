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


class LogLevelLiveReload(Requirement):
    """REQUIREMENT-ID: LOG-004

    Raised by USER 2026-08-13, mid a real OOM investigation: wanted to bump
    a live, in-use machine's file-log level to `debug` for richer diagnostic
    detail, without a `sudo tetron restart` interrupting its actual in-use
    mesh connections. `LOG-003`'s `log-level` config key was read exactly
    once, at `init_tracing()` (`src/main.rs`) startup, to build the file
    layer's `EnvFilter` (`"info,tetron={level}"`) -- `tetron config set
    log-level <level>` was a pure client-side file write with no IPC call to
    the running daemon anywhere in that path, so a running daemon had no way
    to learn the config file changed short of being restarted.

    **Fix, using machinery already available (`tracing-subscriber`'s
    `reload` module needs only the already-enabled `registry` feature, no
    new dependency):**

    1. `init_tracing()` wraps the file layer's `EnvFilter` in a
       `tracing_subscriber::reload::Layer`, and hands the returned `Handle`
       to a new small library module (`log_reload`, `src/log_reload.rs`) via
       a `OnceLock` -- process-global state, the same shape `LogGuard` and
       the panic hook already use for tracing/process-lifetime concerns,
       since there is exactly one subscriber per process. `console_layer`
       (`LOG-003` part 3's decoupled, unconditionally-`"info"` filter) is
       untouched -- only the file layer's filter is reloadable.
    2. A new IPC request, `IpcMessage::SetLogLevel { level }`
       (`tetron-proto`): the CLI still writes `settings.toml` itself first
       (unchanged from `LOG-003`, so the value survives a future restart
       regardless), then additionally tries to notify the already-running
       daemon. The daemon's handler (`MeshManager::set_log_level`) calls
       `log_reload::reload_log_level(&level)`, which rebuilds the exact
       same `"info,tetron={level}"` filter `init_tracing()` computes at
       startup and swaps it in live via `handle.reload(..)`.
    3. `tetron config set/unset log-level` prints one of two messages
       depending on whether the live notification actually reached a
       running daemon: "applied immediately, no restart needed" on success,
       or the original "run `sudo tetron restart`" wording if the daemon
       isn't running (or the IPC call otherwise fails) -- the file write
       itself always succeeds either way, so a not-yet-started daemon still
       picks up the value normally at its next boot.

    Authorization is unchanged: `SetLogLevel` is not added to
    `check_authorized`'s open-read bucket (`Status`/`Sync`/list commands),
    so it needs root or the configured operator UID, same as every other
    mutating command -- consistent with `settings.toml` already being
    root-owned, which already required sudo to reach this code path at all.

    Out of scope: an explicit `RUST_LOG` set for this process's lifetime
    (the raw, ecosystem-standard override `LOG-003` part 3 preserves as the
    top of the resolution order) is superseded by a live reload if one
    occurs -- `reload_log_level` always applies the computed
    `"info,tetron={level}"` filter unconditionally, matching what
    `init_tracing()` itself falls back to whenever `RUST_LOG` is unset. A
    live reload is itself an explicit runtime request to change the level,
    so overriding a startup-time `RUST_LOG` this way is intended, not a gap.
    """

    req_id = "LOG-004"


class ReconnectAndPathIdleLogNoiseReduction(Requirement):
    """REQUIREMENT-ID: LOG-005

    Raised by USER 2026-08-14, reading live `journalctl` output from
    `xps-17-9720` during the OOM investigation: "Pretty much constant log
    spam. BAD." Two distinct offenders, both hitting the *console/journal*
    (unconditionally `info`-and-above per `LOG-003` part 3, so neither
    `log-level` nor `RUST_LOG` could quiet them):

    **1. `WARN ... failed closing path err=MultipathNotNegotiated`.**
    Traced into the vendored dependency, not guessed: `noq-proto`
    (upstream `https://github.com/n0-computer/noq`, resolved at 1.1.0 in
    `Cargo.lock`)'s connection actor fires this on every `PathTimer::
    PathIdle` tick for *any* connection where multipath was never
    negotiated -- i.e. the common case, not an anomaly. `close_path_inner`
    is a multipath-specific API; calling it from the idle-timer handler on
    a plain single-path connection always fails the same way, live-verified
    unrelated to actual connection health (fires equally on healthy
    Direct-connected peers and doomed reconnect attempts). Fixed the same
    way `noq-udp` was already patched (`vendor/noq-udp-1.1.0/PATCH.md`
    precedent): vendor `noq-proto` at the exact `Cargo.lock`-resolved
    version (`vendor/noq-proto-1.1.0/`, `[patch.crates-io]` in
    `Cargo.toml`), demote that one `warn!` to `debug!`
    (`vendor/noq-proto-1.1.0/PATCH.md` documents the patch). This is a
    log-level demotion only, not a root-cause fix -- the underlying
    `close_path_inner` misuse on non-multipath connections is upstream
    territory, out of scope here.

    **2. `INFO ... reconnecting in peer=... secs=30`.** A persistently
    unreachable peer re-logs this at `info` on every backoff iteration
    (steady state: every `BACKOFF_MAX` = 30s), functionally identical in
    shape to `PATH-DIAG-006`'s already-solved `Selected`-flap problem --
    sustained churn against one target with no new information after the
    first few attempts. Fixed with the same debounce shape, applied to
    `spawn_reconnect_loop`'s per-peer reconnect task
    (`src/daemon/mesh/join.rs`) instead of `log_path_events`:

    - New config keys `reconnect-log.threshold` (default 3) and
      `reconnect-log.window` (default 300s -- longer than `path-flap`'s
      60s default, since reconnect backoff already spaces attempts to 30s
      at steady state, so a 60s window would barely ever debounce
      anything; 300s cuts steady-state "still down" noise from once per
      30s to once per 5m while still periodically reconfirming the peer
      is still being retried), same `ReconnectLogConfig` shape as
      `PathFlapConfig`, plumbed through `schema.rs`/`overrides.rs`/
      `storage.rs` identically.
    - `reconnect_log_decision(now, window_start, count, threshold,
      window) -> (log_at_info, new_window_start, new_count)`: a pure
      function, unit-tested directly (`PURE-LOGIC-001` pattern),
      structurally identical to `path_flap_decision` -- a fresh window's
      first attempt always logs at `info` (a peer that just started
      failing must not be silently dropped), further attempts within the
      window log at `info` while `count <= threshold`, `debug` once
      exceeded.
    - State (`window_start`, `count`) is local to each per-peer reconnect
      task (already freshly spawned per disconnect event -- no shared/
      global state needed), config resolved once at task-start, matching
      `log_path_events`'s own "config resolved at the point a task starts"
      precedent.

    Neither fix changes reconnect *behavior* (backoff timing, retry
    logic, multipath negotiation) -- both are logging-only.
    """

    req_id = "LOG-005"


class GenericCallsiteLogRateLimiter(Requirement):
    """REQUIREMENT-ID: LOG-006

    Raised by USER 2026-08-14, immediately after `LOG-005` landed: reading
    live `journalctl -u tetron` on `aorus` turned up a *third*, distinct
    noisy line -- `iroh::net_report::report`'s `warn!("IPv4 address
    detected by QAD varies by destination")`
    (`iroh-1.0.3/src/net_report/report.rs:95`), firing unconditionally on
    every net_report probe cycle (~every 26-30s) once this host's NAT
    mapping is observed to vary by destination -- a persistent network
    characteristic, not a transient event, so it re-warns forever once
    true. Same shape as `PATH-DIAG-006` and `LOG-005`'s reconnect-log
    half: an unconditional per-event log on a fixed cadence for a static
    condition. Different in one way that matters: it originates in `iroh`
    itself (a plain crates.io dependency, not yet vendored, unlike
    `noq-proto`), so a `LOG-005`-style vendor-and-patch would mean a
    *fourth* vendored crate and a *third* incident-driven bespoke fix for
    what is structurally the same problem each time.

    Compared against `sudo journalctl -u tailscaled` the same day: not
    silent (2000 lines / ~2.5 days on this machine), but 95 of those 2000
    lines were `[RATELIMIT] format("...") (N dropped)` -- a generic,
    format-string-keyed limiter that catches any message repeating too
    fast and coalesces the repeats, so a newly-noisy line gets throttled
    automatically instead of needing its own incident + patch.

    **Scope (deliberately narrow -- a v1 mechanism, not a final
    architecture; wider scope is an explicit later decision, not
    foreclosed by this requirement):**

    1. **Keyed by `tracing::Metadata::callsite()`**, not by formatted
       message string (Tailscale's own approach). A `tracing` event's
       callsite `Identifier` is `'static` and unique per macro invocation
       site, already known at compile time -- cheaper than hashing a
       formatted string per event, and the right granularity: the QAD
       warning and `LOG-005`'s old `MultipathNotNegotiated` warning are
       each exactly one callsite regardless of what interpolated values
       they carry.
    2. **Pure decision function**, same signature shape as
       `path_flap_decision` (`PATH-DIAG-006`) and `reconnect_log_decision`
       (`LOG-005`): `(now, window_start, count, threshold, window) ->
       (allow, new_window_start, new_count)`. Unit-tested identically
       (`PURE-LOGIC-001` pattern).
    3. **A new library module, `src/log_ratelimit.rs`** (mirrors
       `src/log_reload.rs`'s precedent: a small module holding
       process-global state via `OnceLock`, one instance per process).
       State: `RwLock<HashMap<tracing::callsite::Identifier, (Instant,
       u32)>>` -- plain std, no new dependency; cardinality is bounded by
       the binary's fixed number of tracing call sites (compile-time
       constant), not runtime-growing, so no LRU/eviction is needed
       (unlike Tailscale's bounded cache, which has to evict because its
       keys are runtime-formatted strings).
    4. **Implemented as a `tracing_subscriber::layer::Filter`**, applied
       to `console_layer` only via `.with_filter()`
       (`init_tracing()`, `src/main.rs:426`) -- composes with `LOG-003`'s
       existing per-layer filter architecture without touching
       `global_filter` or `LOG-004`'s reload `Handle`. The file log's own
       `log-level` knob already governs its verbosity independently; this
       requirement does not extend rate-limiting there.
    5. **One global config pair**: `log-ratelimit.threshold` /
       `log-ratelimit.window` (defaults TBD at implementation, matching
       the `path-flap`/`reconnect-log` shape and `config
       set`/`get`/`unset` plumbing exactly), not per-callsite overrides.
    6. **Lazy suppressed-count flush**: the next allowed event at a given
       callsite is annotated with how many prior events at that callsite
       were suppressed since the last shown one -- no background
       ticker/task, so no new spawned work and no periodic wakeup cost.
    7. **Proof of value, no additional vendoring**: the `iroh` QAD warning
       is resolved by this mechanism directly -- `iroh` is NOT vendored
       for this requirement. The generic limiter is what makes that
       possible without a fourth vendored crate.

    **Explicitly out of scope for this requirement** (a later, separate
    decision if ever pursued):
    - Does **not** replace `path_flap_decision` or `reconnect_log_decision`
      -- both stay as domain-aware, per-peer overrides with semantics
      (e.g. "always show a fresh window's first attempt") a generic
      per-callsite limiter cannot express.
    - No per-callsite config overrides, only the one global pair.
    - No file-layer coverage.
    - No level-based gating -- applies uniformly to whatever reaches
      `console_layer` (i.e. `info` and above, per `LOG-003`'s
      unconditional console filter).
    """

    req_id = "LOG-006"


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


class RemoveCertFloorDeadCode(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-006

    Remove the orphaned `_tetron_certgen` cert-floor record cluster from
    `dht.rs`: the `CERT_FLOOR_RECORD_NAME` const, `encode_cert_floor_record`/
    `decode_cert_floor_record`, `publish_cert_floor`/`resolve_cert_floor`,
    and their own unit tests (which exercise only these functions and encode
    no behavior that survives the removal).

    Found 2026-08-07 while verifying an external PR's memory-leak claim
    (two of the three functions it proposed wrapping in a timeout turned out
    to have zero callers anywhere). Provenance via `git log`/`git show`:
    added in `3d5d1af` (`feat(pair): add \\`ray unpair\\` to revoke a paired
    device`, 2026-07-05) as the pkarr-published revocation-generation floor
    backing device unpairing. `MINIMAL-004` (`1d04c31`, "remove file sharing
    and device pairing") removed the whole pairing feature and its own
    commit message explicitly names "the `_torpedo_certgen` revocation
    floor" as one of the things removed -- but never touched `dht.rs` at all
    (`git show 1d04c31 -- src/dht.rs` is empty). Unreachable by
    construction, not merely unused: pairing is permanently gone
    (MINIMAL-004), so nothing can ever construct a value to publish or
    resolve through this record type.

    Same blind spot as TREE-SHAKE-001..005: `pub` items in the library
    crate's surface are invisible to rustc's `dead_code` lint, so this
    survived two prior dedicated sweeps and a tagged release (`v0.10.0`)
    undetected. The cross-repo verification method that caught it is now
    documented as a reusable procedure at `docs/tetron-workflow.md` section
    12, "Cross-repo dead-code sweep".

    Independent of TREE-SHAKE-001..005 and of TREE-SHAKE-007/-008 below
    (dht.rs's cert-floor cluster does not reference `control.rs`'s
    `DeviceCert`/`PairMsg` types or `identity.rs`'s storage functions, and
    nothing references it back).
    """
    req_id = "TREE-SHAKE-006"


class RemovePairingTicketCodecDeadCode(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-007

    Remove the orphaned pairing-ticket codec from `control.rs`: the
    `PairMsg` enum (`Request`/`Response` variants), the `PairNetwork` struct
    (used only as a `PairMsg::Response` field), `encode_pairing_ticket`/
    `decode_pairing_ticket`, and their own roundtrip unit test. Zero callers
    outside the test: `PAIR_ALPN` no longer exists anywhere in the tree and
    no `accept.rs` arm dispatches `PairMsg`, so nothing can ever send or
    receive one.

    Same provenance and same TREE-SHAKE-006 discovery session (2026-08-07):
    orphaned by `MINIMAL-004`'s pairing removal, missed by both prior
    `TREE-SHAKE` passes for the same `pub`-surface blind spot.

    Must land before TREE-SHAKE-008: `PairMsg::Response` holds a `cert:
    DeviceCert` field, so this requirement's removal must precede
    `DeviceCert`'s own removal in TREE-SHAKE-008, not the other way around
    -- removing `DeviceCert` first would leave `PairMsg` failing to
    compile. Independent of TREE-SHAKE-001..006.
    """
    req_id = "TREE-SHAKE-007"


class RemoveDeviceCertDeadCode(Requirement):
    """REQUIREMENT-ID: TREE-SHAKE-008

    Remove the `DeviceCert` type and everything downstream of it, once
    TREE-SHAKE-007 has cleared `PairMsg`'s reference to it:

    - `control.rs`: the `DeviceCert` struct + its `impl` block; the
      `CertRefresh`/`Unpaired` `ControlMsg` variants (zero references
      anywhere outside their own definition -- confirmed no match arm in
      the entire tree names either variant, so no catch-all/wildcard
      pattern needs updating for their removal); the `device_cert:
      Option<DeviceCert>` field on `ControlMsg::JoinRequest`,
      `::MeshHello`, and `::MemberApproved`.
    - `membership.rs`: the `device_cert: Option<DeviceCert>` field on
      `Member` and `ApprovedEntry`, and the `user_identity:
      Option<EndpointId>` field on both -- found during this requirement's
      own implementation, not the original round-3 sweep: every
      construction site across the entire codebase (~50, exhaustively
      grepped) sets `user_identity: None`; its only non-`None` source
      anywhere was `device_cert.as_ref().map(|c| c.user_identity)` in
      `accept.rs`, which is itself always `None` today (device_cert is
      always `None`). Once `device_cert` is gone, `user_identity` has zero
      remaining producers -- same provably-dead bar as everything else in
      this series, just discovered one level deeper.
    - `identity.rs`: `store_device_cert`/`load_device_cert`/
      `delete_device_cert` (+ the private `device_cert_path` helper) and
      their roundtrip test. `delete_device_cert`'s own doc comment names
      the removed feature directly ("`tetron unpair` best-effort wipe").
    - `daemon/mesh/accept.rs`: drop the `device_cert: Option<control::
      DeviceCert>` parameter from `redeem_invite_and_admit`, `admit_peer`,
      and `admit_approved_member`, and the `user_id_opt =
      device_cert.as_ref().map(|c| c.user_identity)` derivations (always
      `None` given the above).

    **The one spot requiring care, not a mechanical deletion:** the
    `MeshHello` handler in `accept.rs` destructures `device_cert` inside a
    real, currently-executing anti-spoofing check --

    ```rust
    let effective_user_id = if peer_identity == transport_id {
        peer_identity
    } else if let Some(ref cert) = device_cert {
        if !cert.verify() || cert.device_key != transport_id
            || cert.user_identity != peer_identity {
            tracing::warn!(...); return;
        }
        cert.user_identity
    } else {
        return;
    };
    let _ = effective_user_id;
    ```

    The middle branch (verify a presented device cert) is unreachable --
    `device_cert` can never be `Some` in a real message, same as
    everywhere else in this series. But the outer behavior -- reject a
    `MeshHello` whose claimed `identity` doesn't match its
    transport-authenticated identity -- is live, executing, meaningful
    anti-spoofing logic, not dead code, and must be preserved exactly.
    Simplifies to:

    ```rust
    let effective_user_id = if peer_identity == transport_id {
        peer_identity
    } else {
        return;
    };
    let _ = effective_user_id;
    ```

    identical observable behavior for every message any current or past
    tetron build has ever sent (nothing has ever presented a device cert,
    so the removed branch could never have been taken), reviewed and
    confirmed with USER before implementation given its entanglement with
    live security logic rather than pure dead-code deletion.

    **Wire-format safety, checked before implementation, not assumed:**
    every field removed here (`device_cert` on the three `ControlMsg`
    variants, `device_cert`/`user_identity` on `Member`/`ApprovedEntry`)
    already carries `#[serde(default, skip_serializing_if =
    "Option::is_none")]`, and encoding throughout (`encode_msg`,
    `canonical_group_bytes`, `group_blob_hash`) uses `rmp_serde::
    to_vec_named` -- name-keyed msgpack, not positional. Since every real
    code path already sets these fields to `None`, no build has ever put
    them on the wire; removing them changes zero bytes of what is
    currently sent. Safe in both rolling-upgrade directions: an old build
    receiving a message from a build with the field already removed
    decodes fine (`#[serde(default)]` fills the missing key), and a new
    build receiving a message from an old build that still sends the
    (always-empty-when-present) key ignores the unknown key by default
    (named-map decoding, no `deny_unknown_fields`). No ALPN version bump
    needed -- not a breaking wire change, only a formalization of what
    every build's actual bytes already are.

    Same discovery session as TREE-SHAKE-006/-007 (2026-08-07). Depends on
    TREE-SHAKE-007 landing first (see that requirement's docstring).
    """
    req_id = "TREE-SHAKE-008"


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


class SplitConfigModule(Requirement):
    """REQUIREMENT-ID: MODULARIZE-003

    Split `src/config.rs` (1,876 lines, zero section banners despite four
    distinct concerns) into a `src/config/` submodule tree, per the detailed
    symbol→module mapping at
    `DO-NOT-COMMIT/PROPOSAL_modularize-config_2026-08-09.md` (written because
    the earlier sweep proposal, `DO-NOT-COMMIT/PROPOSAL_codebase-
    modularization-sweep_2026-08-05.md` §3b/§7, identified this as a genuine
    split candidate but explicitly left the mapping itself as an open
    question).

    - **`config/schema.rs`**: on-disk types with zero I/O — `MemberEntry`,
      `ApprovedConfigEntry`, `NetworkConfig`, `ServerOverride` (+ its
      `impl`), `RateLimitConfig`, `DropMonitorConfig`, `AppConfig`, the
      `secret_key_hex`/`option_secret_key_hex` serde helper modules, and
      `upsert_network`/`remove_network` — the last two are a placement
      refinement versus the 2026-08-05 proposal, which bucketed them into
      storage by line-proximity even though neither touches disk (both are
      pure `AppConfig.networks` `Vec` mutation).
    - **`config/overrides.rs`**: relay/discovery resolution + `config set`/
      `config get` dispatch, kept as one file (not further split) since the
      dispatch match arms call the resolvers directly — `RELAY_PRESET_
      RAYFISH`/`DISCOVERY_PRESET_RAYFISH`, `validate_http_url`,
      `resolve_url_entry`, `relay_urls`, `discovery_urls`,
      `resolve_upstreams`, `parse_entries`, `config_set`, `set_drop_
      monitor_key`, `set_ratelimit_key`, `parse_ratelimit_value`,
      `parse_bool_value`, `parse_log_level_value`, `parse_duration`,
      `render_override`, `config_get`.
    - **`config/storage.rs`**: filesystem/persistence — `LEGACY_FILE`/
      `SETTINGS_FILE`/`NETWORKS_SUBDIR`, the private `Settings` DTO (a
      second placement refinement: it's `settings.toml`'s serialization
      shape specifically, consumed only by the storage functions, not a
      broadly-referenced public schema type despite sitting next to
      `AppConfig` in the original file), `tetron_gid`, `set_owner`,
      `ensure_dir`, `config_dir`, `validate_net_name`, `write_file`,
      `write_atomic`, `restrict_perms`, `migrate_location`,
      `migrate_legacy`, `load`/`load_in`, `save_settings`/`save_settings_in`,
      `save_network`/`save_network_in`, `load_network`/`load_network_in`,
      `delete_network`/`delete_network_in`, and the thin `load()`/
      `save_settings()` wrappers `node_subnet`, `selfcapture_mitigation_
      enabled`, `log_level`, `set_node_subnet`.
    - **`config.rs`** becomes a re-export shim (`mod schema; mod overrides;
      mod storage; pub use schema::*; pub use overrides::*; pub use
      storage::*;`), same pattern already proven twice in this codebase —
      `GroupMode` (`MODULARIZE-001`) and the pre-existing `pub use
      tetron_proto::TransportMode` already in this exact file. Two items
      need narrower re-exports to preserve their existing visibility:
      `pub(crate) use storage::CONFIG_ENV_LOCK;` (referenced at
      `crate::config::CONFIG_ENV_LOCK` by `src/logdir.rs` and
      `src/daemon/mod.rs`) and `pub(crate) use overrides::parse_duration;`
      (referenced at `crate::config::parse_duration` by
      `src/daemon/mesh/invite_handler.rs`). 99 references to `config::…`
      across 21 files outside `config.rs` itself depend on the shim keeping
      every path resolvable unchanged.

    **Three `reconcile.py` checks break unless fixed in this same commit** —
    found during scoping, not by the 2026-08-05 sweep proposal, since all
    three hardcode `Path("src/config.rs")` instead of whole-tree-scanning
    like most of the other checks do:

    1. `check_relay_preset` (`CON-001`) greps `src/config.rs` for the
       literal `'"rayfish" => Ok(preset.to_string())'`, which moves to
       `overrides.rs` — unpatched, reports `"value": "MISSING"` and
       `CON-001` fails.
    2. `check_product_identity` (`CON-M04`) greps `src/config.rs` for
       `"/etc/tetron"` (from `config_dir()`), which moves to `storage.rs` —
       unpatched, `config_dir_ok` becomes `False` and `CON-M04` fails.
    3. `check_crate_identity` (`CON-M03`) explicitly *skips* `src/config.rs`
       from its `rayfish`-leak scan, since that file deliberately contains
       the allowed relay-preset tokens. Once those tokens move to
       `overrides.rs`, the skip no longer covers them — this check would
       start **false-positively** flagging a leak that isn't one, the
       opposite failure mode from the other two.

    Fix: update the hardcoded path(s) in all three `check_*` functions to
    the new file(s) (`check_crate_identity`'s skip-list needs both
    `config/overrides.rs` and the `config.rs` shim added, not a
    single-path swap).

    **Drive-by fix, same function being moved anyway:** `config_dir()`'s
    doc comment claims macOS uses `~/.config/tetron` — that's the Linux
    XDG-style path, not what this function actually does. The code itself
    is correct (`dirs::config_dir()` resolves to `~/Library/Application
    Support` on macOS, joined with `tetron`; under a root LaunchDaemon `~`
    is `/var/root`, landing at `/var/root/Library/Application
    Support/tetron`, matching `AGENTS.md`'s documented path exactly) — only
    the comment is stale.

    No wire/serialization format change: verified every struct's
    `#[serde(...)]` attributes reference field names, never a module path.

    Depends on nothing (`MODULARIZE-001`/`002` are a different file,
    disjoint symbol set). `MODULARIZE-004` (the test-module split) assumes
    this requirement's module layout already exists — and, discovered
    during implementation (see `MODULARIZE-004`'s own docstring), cannot
    land as a later, separate commit the way `MODULARIZE-002` did for
    `membership.rs`: several storage functions are deliberately private
    test-seam variants, invisible from this shim even via glob re-export,
    so there is no compiling intermediate state with the old test module
    still here. Both requirements land in one commit.
    """
    req_id = "MODULARIZE-003"


class SplitConfigTestModule(Requirement):
    """REQUIREMENT-ID: MODULARIZE-004

    Split `config.rs`'s single flat `mod tests` block (630 of the file's
    1,876 pre-`MODULARIZE-003` lines, ~30 `#[test]` fns, no section
    banners) to match the module layout `MODULARIZE-003` establishes: each
    test moves to whichever of `config/schema.rs`, `config/overrides.rs`,
    `config/storage.rs` exercises the function it tests, colocated with
    that code, matching this repo's TDD convention (`docs/tetron-
    workflow.md` step 5) and the exact precedent `MODULARIZE-002` already
    set for `membership.rs`'s own test split.

    Behavior-free: no test is added, removed, or changed in what it
    asserts, only which file it lives in.

    **Discovered during implementation, correcting the original plan: this
    cannot be a second, later commit — it must land in the same commit as
    `MODULARIZE-003`.** Unlike `membership.rs`'s split (`MODULARIZE-001`),
    where every extracted item was already `pub`, several of `config.rs`'s
    storage functions are deliberately private test-seam variants
    (`load_in`, `save_settings_in`, `save_network_in`, `load_network_in`,
    `delete_network_in`, `migrate_legacy`, plus the `LEGACY_FILE` const) —
    private to `config::storage` by design, for dependency injection in
    tests. Rust's privacy model is "visible in the defining module and its
    descendants"; `config` (the shim) is an *ancestor* of `config::storage`,
    not a descendant, so these items are invisible from `config.rs` even
    via `pub use storage::*` — there is no working intermediate state where
    `MODULARIZE-003` lands with the old flat test module still in
    `config.rs` and `MODULARIZE-004` moves it later, the way
    `MODULARIZE-002` did for `membership.rs`. Two requirements, one commit,
    per `docs/tetron-workflow.md` step 9's bundling exception ("too
    entangled to review separately") — discovered here, not assumed at
    scoping time.

    Depends on MODULARIZE-003 (assumes its module layout already exists);
    lands in the same commit as it.
    """
    req_id = "MODULARIZE-004"


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


class VeilidCustomTransportMechanism(Requirement):
    """REQUIREMENT-ID: VEILID-001

    A new `veilid-transport` workspace crate implements iroh's
    `CustomTransport`/`CustomEndpoint`/`CustomSender` traits (the same
    `unstable-custom-transports` mechanism TOR-M01 already uses) by
    embedding `veilid-core` in-process and carrying each QUIC transmit as
    a Veilid `AppMessage`.

    Addressed by the peer's stable Veilid `NodeId` (`Target::NodeId`,
    Veilid's default safety-routing for sender privacy), not a
    private-route `RouteId` as first sketched pre-implementation: a
    `NodeId` doesn't rotate the way an imported private route does, which
    sidesteps route-churn bookkeeping entirely, and tetron's invite-gated
    model (peers already mutually know each other via the signed roster)
    doesn't need the private-route mechanism's stronger receiver-anonymity
    property anyway.

    Embedding `veilid-core` in-process, rather than talking to a
    separately-running daemon the way `iroh-tor-transport` talks to
    Tor's ControlPort, is not a style choice: no published
    `veilid-server`/client crate exists on crates.io to reuse for that
    (checked 2026-09-11). Embedding `veilid-core` directly is the only
    implementable path today.

    Out of scope for this requirement, each needing its own future
    requirement once this one has landed and been live-verified:

    - Automatic peer discovery. A registered `CustomTransport`'s local
      address is NOT automatically included in what iroh's own pkarr
      publisher sends (`socket.rs::publish_my_addr` builds its address
      list from direct/relay addrs only, by design -- the same isolation
      boundary that makes Tor need `TorCustomTransport::discovery()` as a
      separate mechanism applies here too). VEILID-002 addresses this by
      riding tetron's own signed-roster distribution instead of a new
      DHT-based discovery protocol -- see its own docstring.
    - Wiring into tetron's own `transport.rs`/`TransportMode`/CLI/status
      display, mirroring `src/transport.rs`'s existing `tor` feature
      block -- see VEILID-002.
    - A `tetron-testsuite` scenario exercising it end-to-end -- depends on
      dial-path address injection (see VEILID-003).

    Verified by the crate's own integration test, `#[ignore]`d by default
    since it needs to reach Veilid's public bootstrap network (not
    available in every CI/sandbox environment): build two plain iroh
    `Endpoint`s, each configured with only this custom transport and no
    relay, manually exchange their `CustomAddr`s (standing in for the
    discovery this requirement does not build), and assert a real QUIC
    connection opens and carries data between them end to end.

    **Bug found + fixed live via `tetron-testsuite`'s `veilid-smoke` run
    (2026-09-11), not by inspection:** `VeilidTransportBuilder::build()`
    originally awaited Veilid's own public-network attachment (up to ~2
    minutes observed) before returning. Since `bind_endpoint`
    (`transport.rs`) awaits this synchronously while building the one
    shared iroh `Endpoint` at daemon *process* startup -- before the IPC
    socket, TUN, or any *other*, unrelated network is up -- this stalled
    the *entire* daemon's startup on Veilid's bootstrap, not just Veilid
    functionality. `tetron restart` gave up waiting for the daemon to
    become IPC-reachable well before attachment finished, so `sudo tetron
    restart` itself failed on a VM in the `veilid-smoke` test. Fixed:
    `build()` now returns immediately after `api_startup` (a node's own
    identity is available then regardless of attachment state); actual
    network attachment continues in the background, logged once complete
    for diagnostics only. `poll_send`'s existing fire-and-forget send
    already degrades gracefully in the meantime -- failed sends are logged
    and dropped, the same way an unreachable IP/relay path already
    behaves for every other transport.
    """
    req_id = "VEILID-001"


class VeilidCoreWiring(Requirement):
    """REQUIREMENT-ID: VEILID-002 (depends on VEILID-001)

    Wires `veilid-transport` into tetron's own daemon/CLI, mirroring the
    existing `tor` feature block in `src/transport.rs` exactly:

    - `veilid` cargo feature (`dep:veilid-transport`, a workspace path
      dependency -- `veilid-transport` is not published anywhere).
    - `TransportMode::Veilid` (`tetron-proto`) and `ConnType::Veilid`
      (`tetron-proto::ipc`), alongside the existing `Tor` variants.
    - `--veilid` on `tetron create`/`tetron join` (`conflicts_with =
      "tor"` -- the two are mutually exclusive per network, matching one
      shared iroh `Endpoint` gaining at most the transports its joined
      networks actually asked for).
    - `bootstrap.rs` derives `use_veilid` the same way it already derives
      `use_tor` (`.any(|net| ...is_veilid())` over joined networks) and
      passes it into `create_endpoint_with_alpns`, which starts an
      embedded Veilid node (`VeilidTransportBuilder::build()`) and
      registers it via `.add_custom_transport()` when set.
    - `choose_path_index` (`daemon/mesh/select.rs`) ranks `Veilid` last,
      after `Tor` -- both are higher-latency/anonymizing paths, tried
      only once Direct and Relay are unavailable.
    - `Member` (`membership.rs`, part of the signed `GroupBlob`) gains an
      additive `veilid_node_id: Option<String>` field -- the mechanism
      for VEILID-001's deferred discovery gap: since every joined peer
      already receives every other member's roster entry (hostname, IP,
      etc.) via the existing signed-blob distribution, riding a peer's
      Veilid `NodeId` on that same channel needs no new publish/resolve
      protocol, unlike Tor's onion address (which correctly stays off
      the roster/pkarr path since it isn't network-position data the
      coordinator's admission logic needs to reason about).

    Explicitly NOT done here, deferred to VEILID-003 (its own
    single-responsibility requirement, since these three are only
    meaningful as one working unit and don't decompose further on their
    own):

    - Populating `veilid_node_id` anywhere. Every construction site in
      this pass sets it to `None` (`build_initial_roster`'s own-entry
      construction, `admit_peer`/`admit_approved_member`'s admitted-peer
      construction, and every persisted-config fallback-restore path) --
      the roster *schema* gained the field, but nothing populates it yet.
    - Propagating an admitted peer's own `veilid_node_id` through the
      join/admission wire handshake to the coordinator (today's
      `MeshHello`-equivalent carries `hostname` but not this).
    - Injecting a resolved `TransportAddr::Custom` (built from a target
      peer's roster `veilid_node_id`) into the `EndpointAddr` at the
      single dial chokepoint, `transport::connect_to_peer_with_alpn`.
    - A daemon-lifecycle-scoped Veilid transport handle reachable from
      both the roster-construction and dial-chokepoint call sites (the
      one embedded Veilid node -- like the one shared iroh `Endpoint` --
      needs to be built once and shared, not re-started per call).
    - A `tetron-testsuite` scenario (depends on the above actually
      carrying peer traffic to be worth writing).

    As landed, `--veilid` makes the daemon start a real embedded Veilid
    node and register it as an active custom transport on the shared
    endpoint (structurally verified: compiles under `--features veilid`,
    clippy/fmt clean, `choose_path_index` correctly ranks it), but no peer
    dial actually used it yet -- there was no dialable address for iroh to
    route through the custom transport with. VEILID-003 closes that gap.
    """
    req_id = "VEILID-002"


class VeilidDialPathWiring(Requirement):
    """REQUIREMENT-ID: VEILID-003 (depends on VEILID-002)

    Closes VEILID-002's gap: a `--veilid` network can now actually dial
    peers over the Veilid custom transport, not just start one.

    - `ControlMsg::JoinRequest`/`MeshHello`/`MemberApproved`
      (`src/control.rs`) each gain an additive `veilid_node_id:
      Option<String>` field alongside the existing `hostname`, so a
      joiner's own Veilid `NodeId` rides the same admission/reconnect
      handshake that already carries its hostname -- no separate wire
      round-trip. `JoinRequest`/reconnect `MeshHello` sends are gated on
      the *sending* network's own `transport` being `Veilid` (checked at
      each send site, not globally), so a node's Veilid identity never
      leaks to a coordinator of an unrelated network it also belongs to.
    - Coordinator-side admission (`accept.rs`'s `admit_peer`/
      `admit_approved_member`, reached via `redeem_invite_and_admit`)
      seats the joiner's `veilid_node_id` directly in the `Member` it
      constructs, and broadcasts it in `MemberApproved`. A reconnecting
      known member's fresh `MeshHello` also refreshes the roster's stored
      value (mirroring how a hostname change on reconnect already
      updates the roster) -- relevant here specifically because a
      restarted daemon's embedded Veilid node currently gets a fresh
      random identity each start (VEILID-001's known config-hardening
      gap; not closed by this requirement).
    - `MeshManager` gains a `veilid_node_id: Option<String>` field --
      this daemon's own value, threaded from `create_endpoint_with_alpns`
      (now returns `(Endpoint, Option<String>)`) at bootstrap. Read at
      every join/create/reconnect send site (always gated on that
      network's own `transport`, per the leak note above), never used
      for dialing -- the dial path resolves a *peer's* own
      `veilid_node_id` from their roster entry instead.
    - `transport::connect_to_peer_with_alpn` gains a `veilid_node_id:
      Option<&str>` parameter: when `Some` and parseable, it is resolved
      via `veilid_transport::node_id_to_custom_addr` into a
      `TransportAddr::Custom` appended to the dialed `EndpointAddr`
      (alongside whatever `peercache` already contributes) --
      `#[cfg(feature = "veilid")]`-gated, matching every other
      `veilid_transport` reference in `src/transport.rs`. Every call
      site was updated: two are `Member`-in-scope already (cheap), most
      needed a roster lookup by `EndpointId` first, and three (the
      pre-roster bootstrap blob-fetch dials in `create_join.rs`/
      `reconverge.rs`, run before any roster exists to resolve an
      address from) correctly pass `None`.

    Verification: `reconcile.py` green (fmt/build/clippy/test/cargo-audit)
    under both default features and `--features veilid`; new coverage in
    `control.rs` (`ControlMsg` round-trips including a populated
    `veilid_node_id`). No `tetron-testsuite` pass yet -- deserved its own
    live-network check the way `VEILID-001`'s own integration test does
    (blocked on the same Claude-Code-sandbox network restriction
    documented there), not a claim that this has been live-verified
    end to end.
    """
    req_id = "VEILID-003"


class VeilidCoordinatorSelfEntryHeal(Requirement):
    """REQUIREMENT-ID: VEILID-004 (depends on VEILID-003)

    Closes a real gap found sanity-checking VEILID-003 before live
    testing: the shared iroh `Endpoint` (and its embedded Veilid
    transport, if any) is built once at daemon *process* startup from
    whatever networks are *already* in config -- but `--veilid` only gets
    persisted to config *during* `tetron create --veilid`/`tetron join
    --veilid`, which runs against an *already-running* daemon. On a
    network's very first `--veilid` create, `MeshManager::veilid_node_id`
    is therefore still `None` at the moment `build_initial_roster` runs,
    so the coordinator's own `Member` entry gets `veilid_node_id: None`
    baked into the signed roster -- permanently, since nothing previously
    republished it. A rejoining/reconnecting MEMBER already self-heals
    the same class of problem via a fresh `MeshHello` on its own restart
    (VEILID-003's own design), but a coordinator has no equivalent
    "reconnect to myself" path.

    Fix, in `runtime.rs::restore_coordinator_network` (the boot-time
    per-network coordinator restore, the only place `self.veilid_node_id`
    -- now populated, since the endpoint is already up by the time
    networks restore -- and the network's own persisted `transport` are
    both available together): after the roster is restored and this
    node's own `Member` entry is in hand, if this network's `transport`
    is `Veilid`, `self.veilid_node_id` is `Some`, and it differs from
    the roster's stored value, correct it in place and bump the
    generation before `seal_and_publish` -- required because
    `seal_and_publish` alone only recomputes the snapshot hash, and
    `dht_read_before_write`'s equal-generation-but-different-hash case
    treats a same-generation republish as a tie and silently skips the
    write, which would have made this fix a no-op in practice.

    No separate member-side restore path exists to parallel-check: a
    member's restore goes through the ordinary reconnect handshake
    (`initial=false` in `join_mesh_shared`), which already carries a
    fresh `my_veilid_node_id` once its own transport is up, per
    VEILID-003.

    This still does not make a network's Veilid identity stable *within*
    one uninterrupted daemon session that never restarts after its first
    `--veilid` create -- only the *next* boot corrects it. A live fix
    with no restart required at all would need the daemon to notice a
    newly-available `self.veilid_node_id` while already running and
    republish then, which is not attempted here.
    """
    req_id = "VEILID-004"


class VeilidNonBlockingStartup(Requirement):
    """REQUIREMENT-ID: VEILID-005 (depends on VEILID-004)

    A severe bug found and fixed live via `tetron-testsuite`'s
    `veilid-smoke` scenario (2026-09-11), across several iterations of
    running it, reading `journalctl`, and re-running -- not by code
    inspection. Before this fix, **any daemon with a `--veilid` network
    configured could not reliably restart at all**: `VeilidTransportBuilder
    ::build()` (the `veilid-transport` crate) synchronously awaited first
    the node's own identity becoming resolvable, then full attachment to
    the public Veilid network, before returning -- and `bind_endpoint`
    (`transport.rs`) awaits `build()` synchronously while constructing the
    one shared iroh `Endpoint`, before the daemon's IPC socket, TUN, or any
    *other*, unrelated network is even up. Observed live: `tetron restart`
    gave up waiting for the daemon to become IPC-reachable well inside
    Veilid's own attach window (up to several minutes), the daemon process
    then errored out and exited, and systemd's restart policy respawned it
    into the *same* failure repeatedly -- a genuine crash loop, confirmed
    over 9+ restart cycles in one run, blocking not just Veilid
    connectivity but the whole daemon (every joined network, `--veilid` or
    not) for as long as the loop continued.

    Root cause, once isolated: `get_state()`'s `network.node_ids` (this
    node's own identity) reflects the identity `veilid-core`'s internal
    log already shows within milliseconds of startup only once the
    network layer has made attachment progress -- `attachment.state` was
    observed stuck at `Detached` for the node's entire process lifetime in
    a failing run. Identity is therefore coupled to the same
    slow, occasionally IPv6-black-holed (see `VEILID-001`'s own
    IPv4-only fix, found in the same investigation) attachment process as
    everything else, not available cheaply and synchronously the way an
    earlier version of this code assumed.

    Fix: both identity resolution and attachment readiness now resolve
    fully in the background (`veilid-transport`'s
    `spawn_identity_and_attach_watcher`), never blocking `build()`'s
    return. `VeilidCustomTransport::own_node_id()`/`own_addr()` return
    `Option` (`None` until resolved) instead of a value guaranteed to
    exist; `watch_local_addrs()` starts empty and is updated live once
    identity resolves -- the same "not yet known, arrives later" shape
    iroh's own IP/relay transports already have for their local
    addresses, not a special case invented here. Live-verified: the same
    `veilid-smoke` scenario that previously crash-looped for 9+ restarts
    now completes both restarts cleanly, with the daemon IPC-reachable
    within a couple of seconds each time, and runs the full ~9-minute
    scenario (join, real peer traffic, 60KB+ transferred) without a single
    failure of this kind.

    Consequence, left open rather than solved here: `transport.rs`'s
    `bind_endpoint` still captures `MeshManager::veilid_node_id` as a
    one-shot `Option<String>` snapshot via a short (10s), bounded,
    best-effort wait on `own_node_id()`, so daemon startup itself stays
    fast either way. But live testing showed identity resolution
    routinely taking well over 10s (multiple minutes observed) even with
    the IPv4 fix -- meaning that snapshot is `None` far more often than
    `Some` in practice, so the join/create roster-population paths
    VEILID-002/003/004 built rarely have a real value to work with within
    a given daemon session. The same `veilid-smoke` run confirmed this
    directly: both peers connected successfully (over `Direct`, correctly
    outranking Veilid per `choose_path_index` since both VMs shared a
    LAN), but `conn_type` never became `Veilid` -- consistent with the
    roster never having received a real `veilid_node_id` for either side
    within the test's own runtime. A full live fix needs
    `MeshManager::veilid_node_id` to become a live-updating value (e.g. an
    `ArcSwapOption<String>`, matching this codebase's own preference for
    that over a `Mutex` for shared, frequently-read state) that the
    background watcher updates once resolved, with every consumption site
    (`build_initial_roster`, `run_join_handshake`'s `JoinParams`,
    `spawn_coordinator_background_tasks`, `spawn_join_reconnect`,
    `dial_all_members`) reading it fresh rather than a boot-time snapshot
    -- not attempted here, tracked as the next concrete step before a
    `--veilid` network can be expected to actually select Veilid as its
    connection type within a single, no-restart-needed session.
    """
    req_id = "VEILID-005"


class VeilidLiveIdentityAndMissingAttach(Requirement):
    """REQUIREMENT-ID: VEILID-006 (depends on VEILID-005)

    Closes the gap VEILID-005 left open, plus a second, more severe bug
    found while closing it -- both live-verified via `tetron-testsuite`'s
    `veilid-smoke` scenario (2026-09-11).

    Part 1 -- live-updating identity (the gap VEILID-005 named): before
    this fix, `MeshManager::veilid_node_id` was a one-shot `Option<String>`
    snapshot captured via a short, bounded, best-effort wait during
    `bind_endpoint`, so it was `None` far more often than `Some` for the
    life of a daemon process. Fixed by making it an
    `Arc<arc_swap::ArcSwapOption<String>>`: a background task
    (`spawn_veilid_identity_watcher`, `daemon/mesh/runtime.rs`) polls
    until `veilid-transport`'s own identity resolves, then republishes it
    into this daemon's own roster entry on every coordinator-owned network
    configured for Veilid (`republish_own_veilid_identity`, gated the same
    way VEILID-004's self-heal is, and reusing `publish.rs`'s existing
    `update_snapshot_and_publish` to bump generation/snapshot/blob-store/
    DHT together). Every consumption site VEILID-005 named
    (`build_initial_roster`, `run_join_handshake`'s `JoinParams`,
    `spawn_coordinator_background_tasks`, `spawn_join_reconnect`,
    `dial_all_members`) reads the live cell via the new
    `MeshManager::veilid_node_id()` accessor method instead of a
    boot-time-frozen field.

    Part 2 -- missing `attach()` call (found live while verifying part 1,
    not by inspection): even with live-updating plumbing in place, a
    freshly-rebuilt `veilid-smoke` run still showed **zero** Veilid path
    activity after two full 240-second settle windows. Manual single-VM
    diagnosis (`journalctl` inspection well past the test's own timeout)
    found `veilid-transport`'s own readiness watcher logging
    `did not reach full readiness within 5 minutes elapsed=300s
    identity_known=false` -- and, more tellingly, **zero** veilid-core
    `attach`/`bootstrap`/`rtab` log lines of any kind after the initial
    few milliseconds of startup, even though the node's own internal
    `rtab: Node Ids: [...]` line (proof identity is known internally)
    appeared immediately. Root cause: `VeilidTransportBuilder::build()`
    called `api_startup()` but never called `VeilidAPI::attach()` --
    `api_startup` only constructs the API context; `attach`'s own doc
    comment states the network connect only begins once it is called
    ("Sets the attachment to maintain peers; the network connect proceeds
    in the background tick loop"). Without it, the node never began
    attaching at all, so `attachment.state` stayed `Detached` forever and
    `get_state()`'s `network.node_ids` -- which VEILID-005 correctly
    observed only reflects the internally-known identity once the network
    layer makes attachment progress -- never had any progress to reflect.
    This, not any inherent slowness in `veilid-core` itself, was the real
    reason VEILID-005's investigation saw identity/attachment take
    "multiple minutes or never."

    Fix: `build()` now calls `api.attach().await?` immediately after
    `api_startup()`. Live-verified on a single diagnostic VM after the
    fix: `attach: Attaching...` and the full veilid-core bootstrap log
    sequence now appear within 1 second of startup, `own identity
    resolved` at ~1.5s, and `attached to the public Veilid network`
    (`public_internet_ready`) at ~7s -- down from never-within-5-minutes.
    Re-running the full two-node `veilid-smoke` scenario after this fix
    exercises `conn_type: Veilid`/Veilid path-activity end to end (see
    `tests/veilid-smoke.sh` in `tetron-testsuite` for the live assertion:
    a Veilid path candidate with real `has_activity` in `paths[]`, not
    `conn_type` itself, since `choose_path_index` deliberately ranks
    Direct above Veilid and this test topology always has Direct
    available) -- but still does not pass end to end, live-verified after
    this fix: the roster/dial-path wiring this requirement covers is now
    confirmed correct (both peers dial each other with the right,
    up-to-date `veilid_node_id` candidate address), but no traffic
    actually crosses the resulting path. See `VEILID-007` for the
    separate, deeper gap this fix's own verification run exposed.
    """

    req_id = "VEILID-006"


class VeilidCustomPathIrohRaceGap(Requirement):
    """REQUIREMENT-ID: VEILID-007 (depends on VEILID-006)

    Still-open gap, found live while verifying VEILID-006, not yet fixed.
    With VEILID-001..006 all in place and correct (identity resolves in
    ~1-2s, attachment in ~7s, the roster carries a fresh `veilid_node_id`
    on both sides, and both peers' dial calls correctly include a
    `TransportAddr::Custom` candidate for the Veilid `NodeId`), the
    `veilid-smoke` scenario still shows **zero** Veilid path activity in
    `paths[]` after a full settle window, on both a manual single-restart
    VM pair and a clean automated `tetron-testsuite` run.

    Two sub-findings, both confirmed live via `journalctl`/rolling-file
    log inspection (`RUST_LOG`/`tetron config set log-level debug` only
    elevates the `tetron` crate's own target -- `vendor/iroh-1.0.3`'s
    tracing calls needed temporary `info!` promotion to be visible at all,
    since the file layer's `EnvFilter` is `"info,tetron=<level>"` and
    dependency crates always stay at `info` regardless of the configured
    level, per `LOG-003`):

    1. **Vendored iroh 1.0.3 never gave a custom transport (`--tor`,
       `--veilid`) the same "raced but still worth keeping as a backup
       path" treatment relay already gets.** `RemoteStateActor::
       handle_msg_add_connection` (`vendor/iroh-1.0.3/src/socket/
       remote_map/remote_state.rs`), on establishing a connection whose
       winning path is not relay, explicitly re-adds any known *relay*
       candidate as a backup path -- so relay always shows up in `paths[]`
       even when Direct wins the connection outright. Custom-transport
       candidates got no equivalent call: on a shared-LAN topology where
       Direct wins in ~1ms, a Veilid candidate address correctly included
       in the original `EndpointAddr` was silently dropped once Direct
       won the race, and could never appear as a path at all.

       Patched (`vendor/iroh-1.0.3/PATCH.md`, dedicated entry): widened
       that backfill to also include `transports::Addr::Custom` entries.
       Live-verified this much: `open_path_on_conn` now runs for the
       Veilid candidate and successfully registers a `PathId` on the
       connection (`"opening new path"` trace log, non-`None` `path_id`).

    2. **Opening a path does not by itself cause any traffic to be sent
       on it**, discovered once (1) was fixed and still nothing arrived:
       registering a `PathId` only reserves a slot in the connection; QUIC
       path validation (`PATH_CHALLENGE`/`PATH_RESPONSE`) only begins once
       something actually pings the path. Relay's backup path shows real
       `has_activity` anyway because the relay connection carries its own
       independent keepalive traffic outside QUIC path validation entirely
       -- a custom transport has no such side channel. Patched by pinging
       every relay/custom path just opened in that same backfill step,
       mirroring the existing `handle_msg_network_change` handler's own
       "ping every path so loss-detection starts ASAP" pattern. Live-
       verified this much too: `path.ping()` is called and returns `Ok`
       for the Veilid path (no `"failed to ping"` warning), on a `PathId`
       confirmed registered on a real, non-immediately-closed connection.

    Despite both of these landing correctly, `poll_send` on
    `VeilidCustomSender` (`veilid-transport/src/lib.rs`) was still never
    observed to fire even once, confirmed with dedicated `poll_send`/
    `AppMessage`-received tracing added on both the send and receive
    sides across two independent live runs (one manual single-cycle VM
    pair, one clean fully-automated `tetron-testsuite` run) -- so this is
    not restart-storm noise. Root cause suspected but **not confirmed**:
    live log inspection during this investigation showed the same peer
    accumulating **multiple concurrent `noq::Connection` objects**
    (distinct `conn_id`s, several `AddConnection` events within
    milliseconds of each other) for what tetron's own reconnect logic
    believes is a single logical dial -- consistent with iroh racing
    several connection attempts per `ep.connect()` call (one per
    candidate transport) and keeping only the fastest (Direct, ~1ms on a
    shared LAN) as the connection `tetron status` actually reports, while
    discarding the others. The custom-transport path in the investigation
    was repeatedly observed opened and pinged on connections *other than*
    the one that ultimately won and is shown in `tetron status --json`,
    which would fully explain silent `poll_send` starvation without any
    further bug in this crate's own code. `RemoteCidsExhausted` (the
    trigger PATH-DIAG-008's own dedup patch targets) also fired
    repeatedly on nearly every open attempt in these runs, consistent
    with several connection objects competing for the same finite
    remote-issued CID pool.

    Confirming or ruling this out needs deliberately instrumenting or
    tracing `noq`'s own connection-establishment/racing logic directly
    (not attempted here -- out of scope for what VEILID-006 set out to
    close), or reproducing with a topology where Direct is unavailable so
    there is no fast winner to race against. Tracked here rather than
    closed silently; `tests/veilid-smoke.sh` in `tetron-testsuite`
    continues to run (not in the default `run-list.txt`) and correctly
    fails until this is resolved -- it is not a flaky or misconfigured
    test, it is accurately reporting a real, still-open gap.

    UPDATE (found investigating this requirement further, see VEILID-008):
    live cross-network testing (a real laptop switched onto a mobile
    hotspot, removing the same-LAN "Direct wins in ~1ms" confound entirely)
    produced a stronger negative result than this requirement's own
    racing-connection theory predicts -- with no faster Direct candidate to
    race against at all, the Veilid custom sender was *still* never
    exercised (zero `poll_send`/`AppMessage`/`VeilidCustomSender` log
    lines). That data point, plus a code-level read of the roster-
    propagation path, surfaced VEILID-008: a member's `veilid_node_id`
    could never be corrected in the coordinator's published roster after
    that member's initial admission, regardless of how many times it
    reconnected with a live, freshly-attached Veilid identity. This does
    not yet prove the connection-racing theory above is wrong (VEILID-008
    was a real, independently-confirmable bug either way), but it is a
    more parsimonious explanation for every symptom recorded here, since a
    peer whose `veilid_node_id` the other side never received has no
    candidate to dial in the first place -- no racing required.

    UPDATE 2 (VEILID-008 live-tested, still fails, root cause narrowed
    further -- see VEILID-009): re-testing found VEILID-008 working exactly
    as designed, but exposed a second, distinct propagation gap underneath
    it -- whichever side's post-restart reconnect dial happens to win the
    race determines whether the member's identity has any path to the
    coordinator at all, independent of VEILID-008's fix. VEILID-009 closes
    that gap with a periodic member-side re-announce. Re-test again once
    VEILID-009 lands; the `noq`-level connection-racing tracing this
    requirement originally called for is still not needed to explain any
    symptom observed so far, and remains the last resort if propagation
    turns out not to be the whole story.

    UPDATE 3 (VEILID-009 live-tested, still fails, structural gap found and
    closed -- VEILID-010; then the actual root cause found by reading
    `veilid-core` source, not by more live testing -- VEILID-011): the
    periodic announce fired correctly but was never read at all, traced to
    `spawn_coordinator_dial_retry` never spawning a control-reader on its
    own successful connections (VEILID-010, a pre-existing structural gap,
    not Veilid-specific). But even with every roster-propagation gap now
    closed (VEILID-006/008/009/010), the actual send call underneath all of
    it -- `veilid-core`'s `RoutingContext::app_message(Target::NodeId, ..)`
    -- has been unconditionally rejected since before this investigation
    began: it requires the `footgun-nodeid-target` Cargo feature, never
    enabled in `veilid-transport/Cargo.toml`, so every send failed
    instantly, silently (fire-and-forget `poll_send`, failure logged at a
    level `tetron`'s own log filter never surfaces for a dependency crate).
    This fully explains every symptom recorded in this requirement, with
    nothing left for the connection-racing theory to account for. See
    `VeilidNodeIdTargetFootgunFeature` (VEILID-011) for the fix and its
    independent confirmation (a working sibling project, `tailveil`,
    pinned to the same `veilid-core` version, differing in exactly this
    feature flag). **Not yet closed**: VEILID-011 is fix-applied but not
    yet live-verified end-to-end -- re-run `tests/veilid-smoke.sh` before
    treating this requirement as resolved.
    """

    req_id = "VEILID-007"


class VeilidMemberIdentityReconnectPropagation(Requirement):
    """REQUIREMENT-ID: VEILID-008 (depends on VEILID-006)

    Closes a roster-propagation gap found investigating VEILID-007: a
    member's `veilid_node_id` was only ever written into the coordinator's
    published roster once, at that member's initial admission
    (`accept.rs::admit_peer`). Nothing updated it again for the rest of
    that membership's life, no matter how many times the member
    reconnected with a fresh, correct Veilid identity.

    The gap: every reconnect, a member already sends its current
    `veilid_node_id` to the coordinator in `MeshHello`
    (`join.rs::send_reconnect_hello`), same as VEILID-003's own join-time
    design intends. But the coordinator's accept-side fast path for an
    already-known member (`accept.rs::handle_known_member_reconnect`)
    hands the connection straight to `coordinator.rs::
    spawn_coordinator_control_reader`'s persistent loop -- which received
    every `MeshHello` on that loop but discarded it outright. The comment
    at that discard site predates VEILID-003: *"every other control
    message (including an inbound Pong, and MeshHello -- whose hostname is
    inert since MINIMAL-014 fixed hostname at join) is received but not
    acted on here."* True for `hostname`; stale for `veilid_node_id`, which
    rides the same message but was never carved out.

    Compounded by `VeilidNonBlockingStartup` (VEILID-005)'s own finding:
    the embedded Veilid node only actually starts on a daemon's *second*
    boot for a given config, so a member's `veilid_node_id` at the moment
    of first admission is essentially always `None` or stale -- meaning
    this gap, not restart timing, is what permanently locked every member
    (other than the coordinator itself, which self-heals via VEILID-004's
    boot-time fix and VEILID-006's periodic `republish_own_veilid_identity`
    watcher) out of ever getting a working Veilid dial candidate published
    for it.

    Fix: `spawn_coordinator_control_reader` now also matches
    `ControlMsg::MeshHello { veilid_node_id: Some(id), .. }`, applying it
    via a new `apply_reconnect_veilid_node_id` helper that mirrors
    `runtime.rs::republish_own_veilid_identity`'s own compare-then-publish
    shape (skip the write entirely, no generation bump, when the roster
    already matches; otherwise write, bump generation, and republish the
    signed blob via the same `update_snapshot_and_publish` both paths
    share) -- covered by `coordinator.rs`'s `veilid_reconnect_tests`
    (changed / unchanged / unknown-peer cases). The `remote_id` used to key
    the roster write is the QUIC-authenticated connection identity already
    captured at accept time, not the peer-claimed `identity` field inside
    the message body, so a peer can only ever update its own entry.

    Narrows, but does not by itself resolve, `VeilidCustomPathIrohRaceGap`
    (VEILID-007): that requirement's own investigation notes record a run
    where Veilid stayed silent even with no faster Direct candidate to
    race against, which this gap fully explains without requiring the
    connection-racing theory to be true.

    UPDATE (live re-run against this fix, `tetron-testsuite`'s
    `veilid-smoke`, 2026-09-12): still zero Veilid path activity -- but the
    debug logs captured this time (not captured in earlier runs) show this
    fix working exactly as designed, and pin the remaining gap precisely.
    The member's own dial-time `MeshHello` to the coordinator correctly
    carried the coordinator's Veilid candidate; the coordinator would have
    applied a fresh member `veilid_node_id` correctly had one arrived. But
    the connection that actually survived the post-restart reconnect race
    was the *coordinator's own* dial into the member, not the member's dial
    into the coordinator -- both sides' reconnect loops fire independently
    after a mutual disconnect, and either can win. A coordinator-initiated
    `MeshHello` only ever announces the coordinator's own identity; there is
    no message on that connection carrying the member's identity back, so
    this fix's own handler never got a `MeshHello` to act on at all. See
    `VeilidMemberIdentityPeriodicAnnounce` (VEILID-009) for the follow-up
    that closes this specific gap.
    """

    req_id = "VEILID-008"


class VeilidMemberIdentityPeriodicAnnounce(Requirement):
    """REQUIREMENT-ID: VEILID-009 (depends on VEILID-008)

    Closes the specific gap VEILID-008's live re-run exposed: a member's
    `veilid_node_id` only ever reaches the coordinator via a `MeshHello`
    the *member* sends when *its own* dial to the coordinator wins a
    reconnect race. When the *coordinator's* own dial wins instead --
    equally likely, since both sides' reconnect loops fire independently
    after a mutual disconnect (confirmed live: the surviving connection in
    `veilid-smoke`'s run was the coordinator-initiated one) -- nothing ever
    carries the member's identity to the coordinator at all, VEILID-008 or
    not, because a coordinator-initiated `MeshHello` only ever announces
    the coordinator's own identity.

    Fix: `runtime.rs::spawn_veilid_member_identity_watcher`, a member-side
    periodic background task (5s cadence, same as VEILID-006's
    coordinator-side `spawn_veilid_identity_watcher`) that re-sends
    `MeshHello` to the coordinator over whatever connection is *currently*
    live for that network, independent of which side dialed it. Reuses
    `join.rs::send_reconnect_hello` (now `pub(crate)`) rather than
    duplicating the wire-message construction. No coordinator-side change
    needed: `spawn_coordinator_control_reader`'s VEILID-008 handling
    already applies whatever `veilid_node_id` arrives in any `MeshHello`,
    regardless of when it arrives, and already no-ops (no generation bump,
    no republish) when the roster already matches -- so resending an
    unchanged identity every 5s costs one small control message per member
    per tick, not a wasted blob republish.

    Not unit-tested: like `admit_peer`/`spawn_coordinator_control_reader`
    and every other VEILID requirement that needs a live QUIC `Connection`
    to do anything (VEILID-001 through 006), this is verified live via
    `tetron-testsuite`'s `veilid-smoke`, not a synthetic fixture -- the
    coordinator-side roster-mutation logic VEILID-008 added is the only
    part of this whole chain that could be meaningfully unit-tested without
    a real connection, and already is (`coordinator.rs`'s
    `veilid_reconnect_tests`).

    UPDATE (live re-run against this fix, 2026-09-12): debug logs confirmed
    the periodic announce itself firing correctly, every 5s, exactly as
    designed -- but the coordinator never applied a single one. Root cause
    was structural, not in this fix: `spawn_coordinator_dial_retry` (the
    task that owns a connection whenever the *coordinator's* own dial wins
    a reconnect race -- confirmed the common case in this exact topology)
    never spawned a control-reader loop on the connection it dials at all,
    so nothing arriving on it -- this fix's announces included -- was ever
    read. See `VeilidCoordinatorDialControlReader` (VEILID-010).
    """

    req_id = "VEILID-009"


class VeilidCoordinatorDialControlReader(Requirement):
    """REQUIREMENT-ID: VEILID-010 (depends on VEILID-009)

    Closes a structural gap VEILID-009's live re-run exposed, pre-dating
    Veilid entirely: `coordinator.rs::spawn_coordinator_dial_retry`
    (CONVERGE-012 -- the task that redials a member after a coordinator
    observes it disconnect) registered its successful connection into the
    `PeerTable` and spawned a data-plane `forward::spawn_peer_reader`, but
    never spawned a control-message listener on it at all. Every other
    coordinator-side connection-establishing path
    (`accept.rs::handle_known_member_reconnect`,
    `accept.rs::spawn_admitted_member_tasks`) already spawns
    `spawn_coordinator_control_reader`; this one -- the one that runs
    whenever the *coordinator's* own dial wins a reconnect race instead of
    the member's -- silently did not. QUIC is full-duplex per connection:
    each side needs its own `accept_bi()` loop to see streams the other
    side opens, independent of who dialed. Without one here, nothing the
    member ever sent on a coordinator-dialed connection was read: not a
    `Ping`, not `VEILID-009`'s periodic identity announce, nothing --
    invisible because it never errored, the reads simply never happened.

    Fix: `spawn_coordinator_dial_retry` now also calls
    `spawn_coordinator_control_reader` on its own successful connection,
    threading the two additional handles it needs (`blob_store`,
    `dht_notify`) through from the existing `CoordinatorCleanup` bundle its
    caller already holds.

    `dial_all_members`'s own initial-connect success branch (used by both
    roles at first connect/restore) has the identical latent gap and is
    **not** fixed here -- deliberately out of scope: by the time any
    reconnect cycle matters (which is when a stale identity would actually
    need correcting), `join.rs::spawn_reconnect_loop` (member-initiated,
    already correct via `spawn_member_control_listener`) or this function
    (coordinator-initiated, now correct) has always taken over. Revisit
    only if a concrete gap traces back to the very first connection
    specifically.

    Not unit-tested, same reasoning as VEILID-009: needs a live QUIC
    `Connection` to exercise at all; verified via `tetron-testsuite`'s
    `veilid-smoke`.
    """

    req_id = "VEILID-010"


class VeilidNodeIdTargetFootgunFeature(Requirement):
    """REQUIREMENT-ID: VEILID-011 (depends on VEILID-001)

    The actual root cause behind every symptom `VeilidCustomPathIrohRaceGap`
    (VEILID-007) recorded, found by reading `veilid-core`'s own source after
    VEILID-008/009/010 (all real, independently-justified fixes) still left
    live re-tests showing zero Veilid traffic: `veilid-core`'s
    `RoutingContext::app_message` has two implementations gated by a Cargo
    feature, `footgun-nodeid-target`. Without it:

    ```rust
    pub async fn app_message(&self, target: Target, message: Vec<u8>) -> VeilidAPIResult<()> {
        match target {
            Target::RouteId(_) => self.internal_app_message(target, message.into()).await,
            Target::NodeId(_) => Err(VeilidAPIError::invalid_target(
                "Only PrivateRoute targets are allowed without the footgun feature",
            )),
        }
    }
    ```

    `veilid-transport/src/lib.rs`'s `VeilidCustomSender::poll_send` calls
    exactly this, with exactly `Target::NodeId` (`VEILID-001`'s own
    deliberate addressing choice -- see that requirement's docstring for
    why a private/safety route was rejected). `veilid-transport/Cargo.toml`
    never enabled `footgun-nodeid-target`. Every single `app_message` call
    this transport has ever made -- across every VEILID-006 through
    VEILID-010 live test, every topology, same-LAN and cross-network --
    has failed **instantly**, before any network I/O, with `InvalidTarget`.

    Two things hid this completely:

    1. `poll_send` is fire-and-forget by design (matches UDP's own
       unreliable-send semantics): it `tokio::spawn`s the `app_message`
       future and unconditionally returns `Poll::Ready(Ok(()))` to the QUIC
       layer immediately. iroh never saw the failure -- from its side a
       `PATH_CHALLENGE` ping over this path just never got a
       `PATH_RESPONSE`, indistinguishable from an ordinary black hole.
    2. The failure *is* logged (`tracing::debug!("...app_message send
       failed: {e}")`) -- but at `debug!`, on `veilid_transport`'s own
       crate target, which `main.rs::init_tracing`'s file-layer filter
       (`"info,tetron=<level>"`, `LOG-003`) never promotes past `info`
       regardless of `tetron config set log-level debug` -- that override
       only ever elevates the `tetron` crate's own target. The error fired
       on every single call, silently, through every diagnostic session in
       this entire investigation.

    Confirmed independently before touching code: `tailveil`
    (`/home/erik/code/tailveil`, a separate Tailscale-alike built on
    Veilid, unrelated to tetron) uses the identical `veilid-core = "0.5.7"`
    and the identical `Target::NodeId` + `rc.app_message(target, payload)`
    call shape, and its `Cargo.toml` enables `footgun-nodeid-target`
    (alongside `footgun-config`, an unrelated internal-tuning gate tetron
    does not need -- see `veilid_config.rs`). Working sibling code pinned
    to the same dependency version, differing in exactly this one feature
    flag, is about as strong a confirmation as this could get without a
    live re-test (pending -- this requirement is fix-applied, not yet
    verified end-to-end; do not treat VEILID-007 as closed until
    `tests/veilid-smoke.sh` actually passes against it).

    Fix: `veilid-transport/Cargo.toml`'s `veilid-core` dependency now
    enables `footgun-nodeid-target`. Deliberately did **not** also adopt
    `tailveil`'s `SafetySelection::Unsafe` choice -- this feature alone is
    sufficient for `Target::NodeId` under the default `SafetySelection::Safe`
    (sender-privacy via safety route), which is what `VEILID-001` already
    chose and documented; switching to `Unsafe` is an independent
    latency/privacy tradeoff, not required to fix this bug, and not made
    here.
    """

    req_id = "VEILID-011"


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
# CONFIG-CACHE-001: stop re-reading and re-parsing the config tree on every
# status request
# --------------------------------------------------------------------------

class ConfigLoadIsCached(Requirement):
    """REQUIREMENT-ID: CONFIG-CACHE-001

    `config::load()` re-reads and re-parses the entire on-disk config tree
    from scratch on every call, and it is called from request-handling
    paths. Found 2026-08-16 while tracing why `xps-17-9720` accumulates
    memory and `aorus` does not.

    **The measured cost.** `MeshManager::status()` calls `config::load()`
    twice per request: once directly (to collect `direct` network names)
    and once more inside `network_status()` per network (for
    `nuke_proposal_ttl`). Each `load()` runs `migrate_legacy`, reads and
    TOML-parses `settings.toml`, `read_dir`s `networks/`, then reads and
    TOML-parses every `networks/*.toml`. With one network that is roughly
    **four file reads and four TOML parses per status request**.

    `tetron-systray` polls `IpcMessage::Status` every 8 seconds
    unconditionally (`POLL_INTERVAL`, `tetron-systray/src/main.rs`) --
    450 requests/hour, so **~1,800 file reads and ~1,800 TOML parses per
    hour**, forever, on any machine with a tray icon. It does not matter
    whether anyone is looking at the tray; the poll is unconditional.
    `tetron-webui`'s browser client polls `/api/status` every 10s but only
    while a tab is actually open, so it is a real but conditional second
    source.

    Nothing about this work is necessary: the config changes rarely, and
    the daemon re-derives an identical `AppConfig` every time.

    **Design: validate against the filesystem, do not expire on a timer.**
    A process-global cache holds the last parsed `AppConfig` alongside a
    *fingerprint* of the files it came from -- `settings.toml`'s
    (mtime, len), the `networks/` directory's mtime, and each
    `networks/*.toml`'s (name, mtime, len). A call re-`stat`s those and
    returns the cached value when the fingerprint matches, reloading fully
    when it does not. On a hit that is 2+N stats instead of 2+N reads plus
    as many TOML parses.

    A TTL was considered and rejected: the dominant caller polls on a
    fixed 8-second cadence, so any TTL short enough to bound staleness
    usefully would be missed by every single poll, and any TTL long enough
    to be hit would introduce a staleness window for no additional
    benefit. Filesystem validation has **no staleness window at all**,
    which matters because `tetron config set`, `join`, and `leave` all
    write the tree from a *different process* than the running daemon.
    That is also why explicit in-process invalidation is not sufficient on
    its own.

    The directory mtime is what catches a network being added or removed
    (an atomic rename into `networks/` updates it), so the hot path never
    needs a `read_dir` -- the cached entry already names the files to
    stat, and a changed directory mtime forces the full reload that
    re-enumerates them.

    **Scope.** The cache lives inside `config::load()` so every caller
    benefits (`status`, `node_subnet`, the nuke-TTL lookup, and any future
    one) rather than only the paths noticed today. `migrate_legacy` runs on
    the miss path, preserving its existing once-at-startup effect; it is
    idempotent, so skipping it on a hit changes nothing. Callers that
    mutate config are unaffected -- their writes change the files, which
    changes the fingerprint, which invalidates the cache on the next read.

    This is a performance fix, correct on its own merits and worth making
    regardless of what the concurrent memory-leak investigation concludes.
    It deliberately does **not** change what `status()` returns, and does
    not address the separate and larger question of whether `Status`
    should compute full per-path `ConnectionInfo` (iroh path enumeration
    plus `stats()` per path, per peer) to answer a caller that only needs
    one connected/not-connected bit -- see the systray-cost discussion for
    that, which requires a wire-format decision this requirement does not
    make.
    """

    req_id = "CONFIG-CACHE-001"


# --------------------------------------------------------------------------
# STATUS-CACHE-001: the daemon owns the status refresh rate, not its clients
# --------------------------------------------------------------------------

class StatusSnapshotIsDaemonPaced(Requirement):
    """REQUIREMENT-ID: STATUS-CACHE-001

    Answering `IpcMessage::Status` walks iroh's path machinery: for every
    connected peer, `gather_conn_info` calls `conn.paths()` and then
    `p.stats()` on each path, building a full `ConnectionInfo` with every
    candidate's address, RTT, MTU, black-hole count and PLPMTUD probe
    counters. That work is done on demand, once per request.

    **The cost is set by the clients, which is the actual defect.**
    `tetron-systray` polls every 8 seconds unconditionally -- 450
    requests/hour whether or not the tray is ever opened -- and
    `tetron-webui`'s browser client polls every 10 seconds for as long as
    a tab is open, which USER reports is most of the time. Together that
    is ~810 full traversals/hour, and it grows with every additional tab
    or client. Meanwhile systray consumes exactly one bit of it per peer
    (`connection.is_some()`), having discarded the counters, the version
    and the endpoint id.

    A daemon must not let uncoordinated UI clients dictate how much work
    it does.

    **Design, settled with USER 2026-08-16.** The daemon caches the
    expensive part and chooses its own refresh rate:

    1. **Only the per-peer `ConnectionInfo` is cached** -- the
       `conn.paths()`/`p.stats()` traversal. The scalar counters
       (`packets_rx/tx`, `bytes_rx/tx`, `drops`, `fragmented_*`) are plain
       atomic reads and stay **live on every request**, so traffic and
       drop numbers a dashboard actually watches are never stale, even
       between refreshes.
    2. **Lazy floor, not an eager timer.** The snapshot is rebuilt on
       read, and only when older than the refresh interval. A headless
       machine with no UI attached pays nothing at all; a machine with
       five tabs open pays exactly what one with a single tab pays.
       Chosen over a fixed timer specifically because most peers in a real
       fleet are headless, and an eager timer would tax the machines with
       no UI to serve. A client can still *trigger* a rebuild, but can
       never make it happen more often than the daemon allows.
    3. **Invalidated immediately on mutation** -- join, leave, kick,
       standby, resume, admin changes -- so the UI is never stale
       following something the user just did. Without this the refresh
       interval would be visible as a broken-looking UI after every
       action.
    4. **The existing `IpcMessage::Sync` also invalidates it.** Its
       meaning is already "stop waiting for intervals, get current state
       now", which is exactly the right semantics, and `tetron-webui`
       already has a button wired to it. A scoped
       `Sync { network: Some(..) }` invalidates the whole snapshot:
       over-invalidating a cache is harmless and not worth the complexity
       of tracking per-network entries.
    5. **Refresh interval is a config knob with a sensible default**
       (`status-cache.interval`, ~10-15s), matching how `path-flap`,
       `reconnect-log`, `reconnect-cold` and `reconnect-frozen` are all
       handled.

    **No wire-format change, and no addon changes.** `Status` keeps its
    exact shape and simply stops forcing a rebuild; `Sync` already
    exists. `tetron-systray` needs no change whatsoever, and
    `tetron-webui` needs none either -- its existing per-network `sync`
    button gains fresh-status behavior for free. This was not the first
    design considered: a new `StatusRefresh` variant was proposed to give
    a UI a way to demand freshness, and was dropped once `Sync` turned
    out to mean the same thing already. Avoiding the wire change also
    avoids dragging addon version bumps along with a core minor.

    **Relationship to `CONFIG-CACHE-001`.** They stack and are
    independent. That one removes the filesystem work from the same
    request path (~1,800 reads and parses/hour); this one removes the
    iroh traversal. Neither subsumes the other.

    Found while investigating why `xps-17-9720` accumulates memory while
    `aorus` does not -- systray runs on xps and cannot run on headless
    aorus, making this the one verified behavioral difference between the
    two machines. This requirement is **not** justified by that
    investigation and does not depend on its outcome: bounding daemon
    work by daemon policy rather than by client behavior is correct
    regardless of whether the traversal turns out to leak.

    **Gap found 2026-08-22, embedder scope (`tetron-mobile`).** Point 3's
    "invalidated immediately on mutation" promise is only actually wired
    at one call site: `MeshManager::handle_request`, the desktop
    Unix-socket IPC dispatch loop, which checks
    `invalidates_status_snapshot(&req)` and calls
    `invalidate_status_snapshot()` before matching on the message
    (`daemon/mod.rs`). An embedder built on `build_headless()` (no IPC
    socket -- `tetron-mobile`'s `Node`, and any future non-desktop
    consumer) calls `MeshManager` methods (`join_network`,
    `leave_network`, `activate`, `deactivate`, ...) directly and never
    passes through that dispatch loop, so it never invalidates the
    snapshot at all. Live-verified in `tetron-mobile`, 2026-08-22 (LG
    V40, real hardware): joining a second network left the embedder's own
    status read reporting only the first for the ~12s default interval,
    then correctly reporting both once the floor elapsed -- exactly the
    stale-after-an-action failure point 3 exists to prevent, just outside
    the one place that currently prevents it.

    **Fix, this requirement (not a new one -- same cache, same
    invalidation contract, wider callers):** `invalidate_status_snapshot`
    goes from `pub(crate)` to `pub`, so an embedder can call it itself
    after its own mutating `MeshManager` calls -- the same explicit
    "invalidate after mutation" shape `handle_request` already uses, at a
    new call-site category rather than a new mechanism. No behavior
    change for the desktop/IPC path (`handle_request`'s own call is
    unchanged); no wire-format change. Deliberately not moved *into*
    each `MeshManager` mutator instead (the alternative considered): that
    would invalidate for every caller uniformly and remove the
    per-embedder remember-to-call-it burden, but every mutator already
    has to be enumerated exhaustively either way, and `handle_request`'s
    denylist-shaped `invalidates_status_snapshot` (invalidate on
    everything except `Status`/`AdminList`/`InviteList`) already fails
    *safe* for any IPC message added later -- moving the call means
    re-deriving that same fail-safe shape per mutator, more places to get
    wrong, not fewer. Exposing the existing method keeps one
    implementation and lets each embedder own its own call sites, same
    as `tetron-mobile`'s crate already owns deciding when `up`/`down`/
    `join`/`leave` happen at all.

    **Gap found 2026-08-28, background-mutation scope.** The
    invalidate-on-mutation contract (point 3) was wired only to
    `handle_request`'s denylist and to the embedder call (above) -- both
    *local command* paths. But the cached `NetworkStatus` list also holds
    `member_count` and the per-peer connection list, and those are
    mutated by background mesh tasks that never touch `handle_request`:
    - the coordinator's `spawn_peer_cleanup` pruning a member after its
      deliberate `tetron leave` (`prunes_member()`), or stamping
      `last_seen` on any disconnect;
    - `reconverge_and_apply` replacing the roster from a signed record on
      every non-coordinator node after a `MemberSync` hint;
    - `spawn_reconnect_loop` dropping a disconnected peer from the
      connection table.
    Between such an event and the next `status-cache.interval` boundary,
    `tetron status` kept reporting a departed member -- with its
    connection still shown as live -- for up to the full interval.
    Caught by `tetron-testsuite`'s `core-smoke` (member leaves, coordinator
    still shows `member_count=1` eight seconds later); it regressed
    silently the moment the cache landed.

    **Fix, this requirement:** `MeshManager::status_snapshot` becomes an
    `Arc<StatusCache>` (`StatusCache = RwLock<Option<StatusSnapshot>>`), a
    clone of which is threaded through `MeshCtx` -- the bundle those
    background tasks already carry (alongside `stats`/`blob_store`/
    `pruned_peers`). A free `clear_status_cache(&StatusCache)` is called
    at each of the three mutation sites above. `invalidate_status_snapshot`
    now delegates to it. No wire-format change, no new config, no change
    to the IPC path's own behavior; the cache simply stops being able to
    outlive a roster/peer change that happened off the command path.
    """

    req_id = "STATUS-CACHE-001"


class EmbedderNetworkChangeForward(Requirement):
    """REQUIREMENT-ID: EMBED-NETCHANGE-001

    `MeshManager` exposes an async `network_changed()` method as part of the
    embedding API: the host OS observed a network change (Wi-Fi/cellular
    switch, access-point roam, airplane-mode flip) and the embedder forwards
    that signal in. The method calls `Endpoint::network_change()` on the
    iroh endpoint, which rebinds the QUIC UDP socket and re-probes paths
    (re-STUN, relay reconnect, address re-publish).

    **Why the embedder has to forward it.** On desktop, iroh's `netwatch`
    watches route changes itself through a netlink subscription, so the
    endpoint learns of a change with no help from the daemon. On Android an
    app can not subscribe to netlink route updates -- `netwatch`'s Android
    route monitor is a stub -- so the endpoint never learns the network
    moved. A Wi-Fi/cellular handoff then leaves the endpoint bound to dead
    sockets: no relay, no address publish, no mDNS announce, the device
    invisible to the mesh until something rebuilds the endpoint (a manual
    VPN toggle). Measured upstream on a real device: 116 consecutive failed
    address publishes over roughly 3.5 hours of standby after one
    transition.

    **Shape.** `pub async fn network_changed(&self)` on `MeshManager`
    (`src/daemon/mod.rs`), one line: `self.endpoint.network_change().await`.
    `Endpoint::network_change()` is already a no-op on a closed endpoint
    (it logs at debug and returns), so the method is safe and idempotent --
    cheap to call on every OS callback, and harmless when the network did
    not actually change or iroh already saw it. No new state, no config
    knob, no wire-format change. The desktop IPC path does not call it and
    does not need to.

    **Consumer.** `tetron-mobile`'s `Node` wraps it over UniFFI as
    `Node::network_changed()`, and its Android layer registers a
    `ConnectivityManager` default-network callback for the node's whole
    lifetime (standby included, where nothing else would notice a change)
    that forwards `onAvailable`/`onLost` into the FFI call. That side lives
    in the `tetron-mobile` repo (its own MOBILE-* requirement) and is
    cross-repo follow-up, not part of this requirement.

    Ports upstream rayfish commit 3887fda, adapted to this fork's
    `MeshManager` (direct `endpoint: Endpoint` field, no `transport`
    wrapper). Independent of STATUS-CACHE-001; no ordering constraint
    either way.
    """

    req_id = "EMBED-NETCHANGE-001"


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
