from libspec import Requirement, Constraint, UserStory

# --------------------------------------------------------------------------
# STRANDED-COORDINATOR-WARN: warn before a sole-coordinator leave strands members
# --------------------------------------------------------------------------

class LeaveWarnsWhenSoleCoordinatorHasOtherMembers(Requirement):
    """REQUIREMENT-ID: STRANDED-COORDINATOR-WARN

    Found live 2026-07-18 auditing the CLI/IPC command surface for
    multi-segment TUN: `leave_network` only tears down the *caller's* own
    participation — correct, and the only sane behavior for a command
    that by definition can't act on other nodes. But if the caller was
    the network's only coordinator, every other member is left in a
    network with no one able to admit joiners, mint invites, or kick —
    and had no signal this happened beyond eventually noticing the
    (former) coordinator shows "offline" forever. Live-confirmed this
    exact state 2026-07-18 (a second node still showed a test network
    with the departed sole coordinator as `offline` after it left).

    Not fixable in the sense of a guaranteed farewell broadcast — leave
    is a local, unilateral action, and a coordinator can't force delivery
    of a message to peers who may be offline anyway. What matters more
    than a farewell, though, is that this state was *permanent*: once
    the sole coordinator is gone, no remaining member can ever recover
    coordination capability on their own — `admin add`, `kick`, `invite
    create`, and `nuke` all require holding the network's secret key,
    and there is no path to obtain it after the fact. A warn-and-`--force`
    design (this requirement's first cut, superseded below same day)
    undersold that: it read like "some inconvenience," not "irreversible
    loss of governance for everyone else."

    **Design, revised same day (USER's call, 2026-07-18): don't just warn
    about the strand, actively prevent it where possible.** Before
    leaving, a sole coordinator with other members now auto-promotes
    every member reachable *right now* to co-coordinator, the same
    `AdminGrant` mechanism `tetron admin add` uses (already the
    project's own recommended practice — README/HOWTO tell users "every
    fully trusted member should be a co-coordinator to avoid a single
    point of failure"; this makes that happen automatically at the exact
    moment it matters most instead of requiring the leaver to have
    already done it). This is strictly better than an earlier
    `--transfer-to <peer>` idea (pick one successor) — it doesn't
    require the leaver to decide who the "right" successor is, and
    spreads trust across everyone present rather than creating a new
    single point of failure.

    **The one irreducible limit:** the network's secret key only ever
    travels over a live authenticated connection (`AdminGrant`) — never
    the public signed blob, since that would defeat the point of it
    being secret. A member who is offline at the exact moment `tetron
    leave` runs cannot be promoted, full stop; there is no way to
    pre-stage a grant for them. So the command still refuses by default
    (destructive-adjacent action, same `has_other_members && !force`
    shape `NUKE-CONSENSUS` already established) — but only for the
    residual case: members that auto-promotion could not reach. Anyone
    who *was* reachable is promoted regardless of whether the command
    ultimately proceeds or is blocked on someone else.

    **Fix:** `admin_add`'s `AdminGrant`-sending logic was factored out of
    `daemon/mesh/admin.rs` into `MeshManager::grant_admin_key(network,
    identity) -> Result<(), String>` — identity-only, no hostname
    resolution — so `leave_network` can call it directly for each other
    member without going through `admin_add`'s full IPC-message
    round-trip. `leave_network(&self, network: &str, force: bool)`, when
    `!force`, computes the sole-coordinator check as before
    (`coordinator_count(&roster) <= 1` while the caller itself
    `is_coordinator`), then — only if that's true and other members
    exist — partitions those other members into "currently connected"
    (via `handle.peers.peers_for_network_with_conn`) and not, calls
    `grant_admin_key` for each connected one, and only returns an error
    (naming the short ids of whoever remains unreachable, and how many
    were already promoted) if any member couldn't be saved from
    stranding. If every other member ends up promoted, the leave
    proceeds and the success message reports how many were promoted.
    `tetron leave --force` still bypasses the entire check (no
    auto-promotion attempted either) — an explicit, informed choice to
    abandon the network as-is, matching `nuke --force`'s existing
    semantics of "I know, don't check." Internal callers that already
    made the leave decision elsewhere still always pass `force: true`
    and so skip auto-promotion too: `nuke_network`'s own self-leave
    (tombstone already published — promoting anyone right before
    destroying the network is pointless) and
    `handle_removed_from_network` (reacting to an already-applied
    roster change — kicked or pruned — where granting the key out
    doesn't make sense either).

    **Covered by a unit test** (`leave_blocks_on_sole_coordinator_with_
    unreachable_members`, `daemon/mod.rs`'s `headless_tests`): a
    bare-bones sole-coordinator network with two other members, neither
    connected, confirms the leave is blocked with both short ids named
    in the message and the network handle left intact, then confirms
    `--force` bypasses it. **The "successfully auto-promoted a reachable
    member" happy path is not covered by an automated test** — it needs
    a real, live QUIC connection between two endpoints (`grant_admin_key`
    calls `conn.open_bi()` on an actual `iroh::endpoint::Connection`),
    which this codebase has no lightweight in-process test harness for;
    every other real-connection scenario in this project is verified via
    live multi-machine testing instead, not unit tests.

    Found: 2026-07-18, same audit pass as `STATUS-001` and
    `ADMIN-ADD-NETWORK-SCOPE`. Fixed: 2026-07-18 (warn+force cut);
    redesigned same day to auto-promote before blocking.

    **Not yet live-tested on real multi-machine hardware as of this
    writing** — verified via `reconcile.py` (build/clippy/test green)
    and the unit test above only. **Resolved same day — see the
    live-testing addendum below.**

    **Addendum, 2026-07-18 — `--force` is a deliberate, irreversible
    choice; document it as one.** USER's follow-up questions (is
    kick-everyone-then-leave the only way to force-close a network? is a
    zombie network ever desirable? is there still a way to make one?)
    surfaced that `--force` is in fact the *only* remaining path to a
    zombie network (an unreachable member blocks by default; `--force`
    is the sole override), and that this state is irrecoverable — no
    command or recovery flow can ever regenerate a lost network key, so
    once the last coordinator is gone the roster is frozen forever.
    `docs/HOWTO.md` gained a new "Create a zombie network
    (intentionally)" section: what a zombie actually is, the one
    deliberate way to make one (`--force`) plus the one *accidental* way
    (`sudo tetron uninstall` without `tetron leave`-ing first — uninstall
    never attempts a handoff), an explicit "not reversible" callout, and
    three legitimate reasons to want one (deliberately freezing
    membership as a security ceiling, grace-period wind-down without
    forcing an immediate decision on remaining members, throwaway/test
    networks) — plus a pointer to `nuke` for when the actual goal is
    destroying the network rather than merely orphaning it. The `--force`
    flag's own `--help` text (`Command::Leave` in `main.rs`) and the
    daemon's blocking-error message (`leave_network`, when some members
    remain unreachable) both gained an explicit "NOT REVERSIBLE" /
    "not reversible" callout too, so the warning is visible at the
    point of decision, not just in a doc a user may never open.

    **Addendum, 2026-07-18 — live-tested on 3 bare-metal machines
    (590i-aorus-ultra as sole coordinator, xps-17-9720 and x10sra as
    members), both scenarios the original caveat above named.**

    *All members reachable:* aorus created a fresh network, xps and
    x10sra joined, aorus ran `tetron leave` with no `--force` — reply
    was exactly "promoted 2 other member(s) to co-coordinator, then
    left network '...'". Verified both promotions were real, not just a
    local flag flip: `tetron admin <net> list` on each showed itself as
    a key-holder, and each independently minted a working invite
    (`tetron invite <net> create`) after aorus was gone — proof of a
    genuinely usable key, since minting requires a real, valid
    coordinator secret. Bonus check: with two coordinators now, one of
    them (xps) leaving proceeded immediately with no promotion message
    at all, confirming `coordinator_count <= 1` correctly gates the
    whole mechanism.

    *One member offline:* same setup, then `sudo systemctl stop tetron`
    on x10sra to take it offline. `tetron leave` with no `--force` on
    aorus refused, naming x10sra's exact short id and confirming xps
    was already promoted despite the overall block — matching the
    designed message precisely. Verified xps's promotion was still real
    (same admin-list + invite-mint proof) even though the command as a
    whole failed. `tetron leave --force` then proceeded, deliberately
    stranding x10sra. Restarting x10sra's daemon reproduced the exact
    zombie symptom this requirement exists to prevent by default: it
    still showed the network with aorus permanently offline, while its
    connection to the promoted xps stayed live and direct. Confirmed
    x10sra itself — never a coordinator — could still `tetron leave`
    freely with no block, since the check only ever applies to the
    caller's own coordinator status.

    No bugs found in either scenario; behavior matched the design
    exactly on the first live run. `reconcile.py` remained the gate for
    build/clippy/test throughout, matching the discipline established
    for every other destructive-adjacent feature in this project
    (`NUKE-CONSENSUS`, the `CONVERGE-*` fixes).
    """
    req_id = "STRANDED-COORDINATOR-WARN"


# --------------------------------------------------------------------------
# HARDEN-002/004/005: control-plane rate-limit hardening, bundled
# --------------------------------------------------------------------------

class RateLimitHardening(Requirement):
    """REQUIREMENT-ID: HARDEN-002

    A 2026-07-23 review of `HARDENING-SPEC.md` adopted three related
    rate-limit changes to `src/ratelimit.rs`'s `ControlGate` (guarding
    inbound control-plane messages -- `MemberSync`/`BlobUpdated` triggers,
    `MeshHello`, invite gossip -- per connection). Bundled into one
    requirement since they land together and share one review.

    **HARDEN-002 (tighten the per-connection defaults):** the original
    constants (`CAPACITY=20`, `REFILL_PER_SEC=2`, `STRIKE_LIMIT=100`) let an
    already-admitted peer burst 20 control messages instantly, each
    potentially triggering a pkarr resolve and, on a hash change, a blob
    fetch -- real work on the receiving node for cheap-to-send input.
    Tightened to `CAPACITY=5`, `REFILL_PER_SEC=1`, `STRIKE_LIMIT=20`. A
    legitimate peer never needs to burst more than a handful of control
    messages at once (one `MemberSync`/`BlobUpdated` per actual roster
    change), so this has no effect on normal operation.

    **HARDEN-004 (add a global token bucket alongside the per-connection
    one):** the per-connection gate alone bounds one connection, not the
    daemon's aggregate control-plane workload across every connection at
    once. `ratelimit::GlobalRateLimiter` is one additional token bucket
    shared daemon-wide (`GLOBAL_CAPACITY=10`, `GLOBAL_REFILL_PER_SEC=3`,
    `GLOBAL_STRIKE_LIMIT=50`), consulted in addition to (never instead of)
    each connection's own gate -- a message is only actually processed if
    *both* gates say Allow (`ControlGate::check_with_global`, which
    short-circuits: a per-connection Drop/Close never even consults the
    global gate, since the message is already being dropped). Because
    admission is invite-gated (`LIVE-001`), "N connections" means N
    separately-admitted identities, not N sockets from one unauthenticated
    attacker -- so the real severity this closes is narrower than the
    original hardening doc framed it (a multi-identity insider or a
    coordinator with many legitimately joined peers all misbehaving at
    once, not an anonymous flood) -- still worth it as defense-in-depth.
    `GlobalRateLimiter` builds on the `ratelimit` crate's own lock-free,
    atomics-based `Ratelimiter` (`try_wait` takes `&self`, not `&mut self`),
    so the shared bucket needs no `Mutex`: its own strike counter is a bare
    `AtomicU32`. A `Close` verdict from the global gate closes only the one
    connection whose message happened to tip the shared bucket over --
    there is no single "the abusive connection" to target under a
    multi-connection swarm, so this is a pragmatic choice, not a claim that
    the closed connection was individually at fault.

    **HARDEN-005 (make both sets of constants configurable):** a direct
    instance of the standing "configurable knobs over hardcoded values"
    preference, not just operator convenience -- this bundle ranks *above*
    where the original hardening doc placed it ("deferred, low priority")
    for exactly that reason. `config::RateLimitConfig` (a new
    `AppConfig`/`Settings` field, `ratelimit`) holds six `Option` fields --
    `capacity`/`refill_per_sec`/`strike_limit` for the per-connection gate,
    `global_capacity`/`global_refill_per_sec`/`global_strike_limit` for the
    shared one -- each `None` meaning "use the compiled default" above.
    Set via `tetron config set ratelimit.<key> <value>` (keys:
    `capacity`, `refill-per-sec`, `strike-limit`, `global-capacity`,
    `global-refill-per-sec`, `global-strike-limit`); an empty value resets
    that one key to its compiled default, matching `relay`/`discovery-dns`/
    `subnet`'s existing reset convention. Like every other `tetron config
    set`, applies on `sudo tetron restart` -- `ControlGate::new()` reads the
    override fresh each time it constructs a gate (once per connection, not
    a hot path), and the global gate is built once at daemon bootstrap
    (`bootstrap::build_daemon`) from the config snapshot already loaded
    there.

    **Rejected in the same review -- `HARDEN-001`** (IPC socket `0666` ->
    `0660` + `chown root:tetron`): contradicts this project's own explicit
    "Privilege & access" design (`AGENTS.md`) -- the socket is deliberately
    world-connectable, with authorization entirely per-request via
    `SO_PEERCRED`, not socket permissions. Both the WebUI and Systray addons
    architect around exactly that. Not adopted.
    """
    req_id = "HARDEN-002"


# --------------------------------------------------------------------------
# HARDEN-007: tighten the QUIC idle timeout for accepted connections
# --------------------------------------------------------------------------

class QuicIdleTimeoutTightened(Requirement):
    """REQUIREMENT-ID: HARDEN-007

    `src/transport.rs`'s `quic_transport_config()` builds the single
    `QuicTransportConfig` shared by the whole iroh `Endpoint` (both dial and
    accept sides) without ever setting `max_idle_timeout`, so it carries
    iroh/quinn's own 30s default. Per RFC 9000, the true per-connection idle
    timeout is `min(ours, peer's)` — tightening our own side to 10s bounds
    every connection this node is party to, including ones a misbehaving or
    silent peer never closes cleanly, without needing the peer's
    cooperation.

    Found 2026-08-07 during external PR #12's DHT-leak claim verification
    (`DO-NOT-COMMIT/ANALYSIS_external-PR12-dht-leak-claim_2026-08-07.md`,
    "Recommendation" item 2) — PR #12's own framing (a leak-fix for stalled
    scanner handshakes accumulating in `accept.rs`'s per-connection
    `tokio::spawn`) doesn't hold: Step 0 confirmed the 30s default already
    bounds every incoming handshake today, so nothing was ever unbounded.
    This is legitimate, modest hardening on its own merits — faster cleanup
    of scanner/probe connections that complete a handshake then go silent —
    independent of, and not fixing, that refuted claim. Not urgent.

    Fix: `quic_transport_config()` adds
    `.max_idle_timeout(Some(VarInt::from_u32(10_000).into()))` (10s, as
    milliseconds — `VarInt::from_u32` is infallible, so this needs no
    `Result`-ifying of the function's signature, unlike the
    `Duration::try_into()` form shown in iroh's own doc example). No other
    knob in `quic_transport_config()` changes.

    Verified by `cargo build`/`clippy`/testsuite (a live two-host mesh
    connection outliving an idle period, plus a stalled/incomplete
    handshake actually getting torn down within the new bound) — not a new
    unit test, this is a single builder-chain config value with no branching
    logic of its own to isolate.

    Independent of `DHT-ERRCAUSE-002` (same PR #12 analysis, different file,
    `src/dht.rs`, no shared state). May land in either order.
    """
    req_id = "HARDEN-007"


# --------------------------------------------------------------------------
# CONN-STABILITY-001: reverts HARDEN-007's global idle-timeout tightening --
# found to cause a severe, continuous regression on relay-tunneled
# connections between already-admitted, trusted mesh peers. Full
# investigation trail: `DO-NOT-COMMIT/ANALYSIS_idle-timeout-reconnect-churn_2026-08-11.md`,
# `DO-NOT-COMMIT/PLAN_connection-stability-idle-timeout_2026-08-11.md`,
# `DO-NOT-COMMIT/TODO_DETAILS.md` #6.
# --------------------------------------------------------------------------

class QuicIdleTimeoutRevertedToUpstreamDefault(Requirement):
    """REQUIREMENT-ID: CONN-STABILITY-001

    Reverts `HARDEN-007`: `quic_transport_config()`'s `max_idle_timeout`
    goes back to iroh/quinn's own 30s default (the explicit `.max_idle_timeout(...)`
    override removed entirely, not just changed to a different value --
    letting the builder's own default carry it, so a future upstream
    default change is inherited automatically rather than silently
    diverging from it again).

    **Why HARDEN-007's benefit was already thin.** Its own docstring
    concedes the claim it was originally framed to fix (PR #12's
    unbounded-handshake-accumulation concern) was refuted before it even
    landed -- *"Step 0 confirmed the 30s default already bounds every
    incoming handshake today, so nothing was ever unbounded."* What
    shipped was a fallback justification: faster cleanup (10s vs 30s) of
    scanner/probe connections that complete a handshake then go silent --
    explicitly *"Not urgent"* in its own text. And even that benefit only
    ever applied to the accept-side, pre-admission case: an already-admitted,
    trusted mesh peer isn't a scanner, so tightening its timeout carried
    the same code's benefit to zero of its own connections, while `.transport_config()`
    being one shared config for both dial and accept sides meant every
    connection paid the resulting cost regardless (`PLAN_connection-stability-idle-timeout_2026-08-11.md`'s
    Experiment B (2.2) had proposed narrowing the scope instead of a full
    revert -- superseded by this decision once the plain revert was
    empirically confirmed sufficient on its own, see below).

    **The cost, found live 2026-08-11/12 during OOM-reproduction testing,
    confirmed root-caused, not just observed.** A relay-forced idle
    connection between two admitted mesh peers cycled disconnect/reconnect
    continuously -- 425 events over an 83-minute production-condition run
    when first found; reproduced deterministically in a controlled 2-VM
    test (14 reconnects in a 180s idle window, ~10.2-10.4s apart,
    `error=timed out`). Traced to a genuine, specific mechanism, not
    vaguely attributed:

    1. The relay protocol (`iroh-relay-1.0.3`, vendored source) runs its
       own application-level ping/pong heartbeat, entirely independent of
       QUIC's own idle timer: `PING_INTERVAL = 15s` (+ random jitter,
       `protos/relay.rs:36`, `server/client.rs:339`), `PING_TIMEOUT = 5s`
       default (`ping_tracker.rs`). Both client and server sides correctly
       implement it (confirmed by reading `iroh::socket::transports::relay::actor`,
       not assumed) -- receiving a ping/pong frame counts as connection
       activity, which resets an idle timer.
    2. `HARDEN-007`'s 10s value is *shorter* than this heartbeat's own
       ~15s+jitter interval. A relay-tunneled connection therefore always
       dies before the relay's own keepalive mechanism ever gets a chance
       to fire even once -- a pure race between two independently-chosen
       constants, not a deeper protocol bug. This also explains why an
       earlier experiment (adding an explicit client-side
       `keep_alive_interval`, branch `experiment/conn-stability-keepalive`,
       kept for reference) made *zero* observed difference: the missing
       ingredient was never local keepalive at all, it was simply enough
       time for the relay's own already-correct heartbeat to engage.
    3. A genuinely idle DIRECT connection (no relay) does not show this
       churn (zero teardowns over 30 real minutes against real fleet
       peers) -- consistent with direct connections more often carrying
       *some* incidental traffic within a 10s window, unlike a relay hop
       whose only native keepalive-equivalent runs on a longer cadence.

    **Fix choice empirically confirmed, not assumed.** Reverting to 30s
    was tested directly against the identical relay-forced-idle setup
    that produced the churn: 0 reconnects over a 250s window (vs. 14 in
    180s at 10s) -- the connection lived well past the relay's own first
    heartbeat and was sustained by it indefinitely afterward. This settles
    the open question of whether reverting fully fixes the bug or merely
    slows it: it fixes it, at least for the relay protocol's own
    ~15s-cadence heartbeat providing sufficient cover once given the room
    to run.

    **Standing regression coverage (mandated by this investigation's own
    charter, not optional):** two new `tetron-testsuite` tests,
    `idle-connection-stability.sh` (direct path) and
    `idle-connection-stability-relay.sh` (relay-forced, reusing
    `lib/network_faults.sh`'s `force_relay_only`), each holding a joined
    connection genuinely idle for a duration comfortably longer than 30s
    and asserting zero reconnect events. Both added to `run-list.txt` as
    standing, routine gates (not manual/exploratory like the OOM-repro
    suite) -- confirmed to FAIL against the pre-fix 10s timeout before
    this fix landed, per this repo's own "a regression test that has
    never been observed to fail is not proven to test anything" standard.

    USER's own stated priority motivating this whole investigation,
    quoted directly: *"Connection and stability are the number one
    concern for a VPN. If anything we should improve over the upstream,
    not regress."* This reverts a regression back to upstream's own
    baseline; not itself a further improvement past it.
    """
    req_id = "CONN-STABILITY-001"


# --------------------------------------------------------------------------
# AUTHZ-001: AdminList/InviteList must be open to any local user, not
# operator-gated
# --------------------------------------------------------------------------

class AdminInviteListOpenToAnyUser(Requirement):
    """REQUIREMENT-ID: AUTHZ-001

    Found while scoping `SYNC-001`'s own authorization level: `AGENTS.md`'s
    "Privilege & access" section describes reads (`status`, `*... show`) as
    open to any local user, mutating commands as needing root or the
    configured `operator_uid`. In the actual code, `check_authorized`
    (`daemon/mod.rs`) only ever exempted the single literal
    `IpcMessage::Status` -- `AdminList` (`tetron admin <net> list`) and
    `InviteList` (`tetron invite <net> list`) fell through to the
    operator-gated branch and were denied for an unprivileged, non-operator
    local user, even though both are pure reads that mutate nothing.
    `AdminList`'s own doc comment in `tetron-proto/src/ipc.rs` already says
    "Open read" -- the gate never matched the documented intent.

    **Verified safe to open before fixing:** `admin_list`
    (`daemon/mesh/admin.rs`) has no further internal restriction at all,
    matching its "Open read" doc comment exactly. `invite_list`
    (`daemon/mesh/invite_handler.rs`) does carry its own independent
    per-network `coordinator_handle` check (its own doc comment says
    "coordinator-only") -- but that gate is about which *network* the caller
    holds the coordinator key for, entirely separate from `check_authorized`'s
    OS-user privilege gate. Opening the outer (OS-user) gate does not bypass
    the inner (per-network coordinator) one, so a non-coordinator local user
    still gets nothing back from `invite_list` for a network they don't hold
    the key to -- exactly the same result as before this fix, just reached
    via the correct gate instead of the wrong one.

    **Fix:** `check_authorized`'s exemption `matches!` gained `AdminList { .. }`
    and `InviteList { .. }` alongside `Status`/`Sync` (SYNC-001).
    """
    req_id = "AUTHZ-001"


# --------------------------------------------------------------------------
# ADDONS-SUITE-001: a name-free --help pointer only, no install script and
# no fetch-and-run subcommand in tetron itself
# --------------------------------------------------------------------------

class AddonSuiteInstallScript(Requirement):
    """REQUIREMENT-ID: ADDONS-SUITE-001

    **SUPERSEDED by ADDONS-SUITE-002.** The "decided" outcome below (no
    script, `--help` epilog only) held only briefly: `contrib/install-
    tetron-suite.sh` was rebuilt and committed after this requirement was
    written, gained `--check`/`--yes-core`/`--musl`/a `backup` component
    along the way without a matching spec update, and is now the
    documented, primary install path in `README.md`'s TL;DR. Kept verbatim
    below for the rejected-alternatives reasoning (still valid -- no
    `tetron` subcommand, no fetch-and-run coupling in core), which
    ADDONS-SUITE-002 does not repeat.

    Discussed at length 2026-07-24: should tetron itself gain a command (or
    ship a companion script) to discover/install its optional add-ons
    (`tetron-webui`, `tetron-systray`)? Landed on the least-invasive of
    several designs considered, after explicitly rejecting three more
    coupled alternatives -- including an initially-built standalone install
    script, superseded before being committed (see "Course-corrected" below).

    **What shipped:** nothing executable in `tetron` core. Just a minimal
    `tetron --help` epilog: `"Optional webui and other addons available,
    see the tetron project page for details."` -- deliberately names
    neither addon's repo, asset convention, or URL, so it can never go
    stale regardless of how the addons or their release process change.

    **Why not a `tetron addons` subcommand (rejected designs):** a real
    installed `tetron` binary (a downloaded release asset, not a repo
    checkout) has no filesystem access to a companion script at all, so any
    subcommand that "just runs" one needs to fetch it from somewhere over
    the network at invocation time. Three shapes were considered:

    1. **Embed a script at compile time** (`include_str!`, like
       `tetron-webui`'s static assets) -- rejected outright: editing it
       would then require rebuilding and re-releasing `tetron` itself,
       which defeats the entire point of keeping addons separate.
    2. **A `tetron addons` command that fetches-and-runs a script fresh
       from tetron's own repo each invocation** -- mechanically sound (the
       only fact baked into `tetron` would be "my own repo has a file at
       this path," not third-party knowledge), and was worked through in
       detail: show the fetched content (paged, not just a bare warning
       string) and require two separate confirmations before ever
       executing it, since a built-in command that fetches-and-runs a
       remote script is a real, ongoing code-execution trust surface
       (every future invocation trusts this GitHub repo's integrity, not
       just the one-time binary download) -- mitigated by forced review,
       but not eliminated.
    3. **A narrower `tetron addons` that installs only `tetron-webui`**,
       leaving `tetron-webui` itself responsible for installing further
       add-ons (`tetron-systray`, future ones) from inside its own UI --
       rejected for the same core reason as option 2, just a smaller dose
       of it: `tetron` core's own source would still permanently know
       `tetron-webui`'s specific repo and release-asset convention, coupling
       that runs backwards (the addon depends on tetron, never the other
       way around) and needs a `tetron` code change if that one addon is
       ever renamed or replaced.

    **Course-corrected 2026-07-24:** a standalone `contrib/install-tetron-
    suite.sh` script was built and live-tested as a middle ground (not a
    `tetron` subcommand, so no coupling in `tetron`'s own source -- but
    still a maintained artifact promising to fetch/verify/install all
    three components). USER decided against keeping even that: the
    `--help` epilog alone is the whole deliverable. The script was deleted
    before ever being committed. **This also means the earlier plan for
    `tetron-webui`'s own addon-install framework** (`src/addons.rs`, built
    and live-tested separately, on both Linux and macOS) **is unaffected
    and unrelated to this decision** -- that lives entirely in the
    `tetron-webui` repo, mirrors the same download/verify convention
    independently in Rust, and never depended on this script existing.

    **Decided:** `tetron` core gains nothing executable and no companion
    script. `tetron-webui` is the hub for installing further add-ons from
    inside its own dashboard -- already built (`tetron-webui`'s own
    `src/addons.rs`), not just planned. Getting to `tetron-webui` the
    first time is a manual, one-time bootstrap step (read its own README),
    a completely ordinary shape for software with an optional GUI layer on
    top of a CLI-first tool, not a UX gap.

    **`--help` epilog wording, deliberately name- and path-free:** an
    earlier draft named a script's exact path -- rejected as too
    committal, since it would need a matching edit the moment that path
    changed or stopped existing (exactly what happened here). The shipped
    wording makes no claim that could go stale: it relies only on the fact
    that anyone running `tetron --help` already has `tetron` installed
    from *somewhere* (a README, a release page) that already has the
    real, current links -- the epilog's only job is reminding them add-ons
    exist, not being the source of truth for where to find them.
    """
    req_id = "ADDONS-SUITE-001"


# --------------------------------------------------------------------------
# ADDONS-SUITE-002: interactive default-or-pick addon gate, TTY-aware, for
# contrib/install-tetron-suite.sh
# --------------------------------------------------------------------------

class InstallSuiteAddonSelectionGate(Requirement):
    """REQUIREMENT-ID: ADDONS-SUITE-002

    `contrib/install-tetron-suite.sh` (see ADDONS-SUITE-001 for why this
    script, rather than a `tetron` subcommand, is the install surface at
    all) unconditionally installed all four components (`core`, `webui`,
    `systray`, `backup`) with no way to opt out short of listing component
    names positionally, and had no concept of headless hosts or non-
    interactive invocation. Two concrete problems, both found working
    through this: (1) `tetron-systray` cannot function without a display
    at all, yet was installed unconditionally on headless machines; (2)
    the README's own documented one-liner (`curl -fsSL .../install-tetron-
    suite.sh | bash`) pipes the script over stdin, so `[ -t 0 ]` is false
    and `confirm_core()`'s existing safety gate silently skipped `core`
    (and `backup`) with only a stderr warning, unless `--yes-core` was
    also passed -- the documented quickstart did not actually install
    `core` as written.

    **Selection logic, in priority order:**

    1. **Any explicit flag wins, non-interactively.** `--core-only`,
       `--install-webui`, `--install-systray`, `--install-backup`,
       `--install-all` select components directly with no prompts.
       `core` is implicit in every `--install-*` flag (it is the
       daemon the addons talk to) -- only `--core-only` excludes the
       addons. `--core-only` combined with any `--install-*` is a usage
       error (contradictory intent). `--install-all` is `core` + every
       addon, `backup` included.
    2. **No flags, `--check`:** a read-only status report has nothing to
       confirm before running, so it skips the prompt entirely and uses
       the same display-aware default set tier 4 would compute (`core`
       alone if headless, `core, webui, systray` with a display) --
       `backup` still excluded, same as everywhere else.
    3. **No flags, no `--check`, no controlling terminal reachable**
       (piped install with nothing to prompt on at all -- see the
       `/dev/tty` note below for what "reachable" means here): behave as
       `--core-only`, printing one line telling the user how to get
       addons via flags instead of guessing.
    4. **No flags, no `--check`, a controlling terminal is reachable**
       (interactive default -- this is also what a normal `curl | bash`
       run hits, not tier 3, see below): detect a display via
       `$DISPLAY`/`$WAYLAND_DISPLAY`. Print the detection result and the
       resulting default component set (`core` alone if headless; `core,
       webui, systray` with a display -- `backup` is never in the
       default set at any tier, it is opt-in only via flag or the
       picker below), then a single gate: `Use defaults? [Y/n]`.
       Enter/`y` installs exactly the printed default, no further
       questions -- the common case costs one keystroke. `n` drops into
       a per-component `[y/n]` prompt for `webui`/`systray`/`backup`,
       each still pre-filled with the same display-aware default so the
       user only has to touch what they want to flip from the default
       (`backup` always defaults to `N` here too). `core` itself is not
       prompted in the picker -- it is the mandatory base the addons run
       against.

    Rationale for `backup` defaulting off unconditionally (confirmed with
    USER): unlike `webui`/`systray`, whose default hinges only on display
    presence, `backup` is a rare, admin-only case not tied to display at
    all -- opt-in via `--install-backup` or the picker, never a default.

    **Fresh-install exception to `confirm_core`'s TTY gate:** the
    non-interactive default (tier 3 above) would otherwise be
    self-defeating for `core` specifically, since `confirm_core()`
    already refuses to touch `core` without a TTY or `--yes-core` --
    exactly the piped case tier 3 targets. Resolved by making that gate
    conditional on whether this is an **upgrade** (something already
    installed at `dest`) rather than a fresh install: an upgrade
    genuinely disconnects live peers and still requires `--yes-core` or
    an interactive confirmation; a fresh install has no peers to disrupt
    and proceeds under the piped default without needing `--yes-core`.
    This makes the README's documented one-liner actually install `core`
    on a new machine, while preserving the original safety property for
    the case it was actually protecting against. Renamed to
    `confirm_sudo_install()` in the same change, since it now also gates
    `backup`'s sudo install with correct per-component messaging (it
    previously said "core tetron" even when confirming `backup`).

    **TTY detection reads `/dev/tty`, not stdin:** `[ -t 0 ]` is false
    under `curl | bash` even when a human is running it at a real
    terminal -- stdin there is the piped script itself, not the keyboard.
    `/dev/tty` reaches the controlling terminal directly regardless of
    stdin redirection (the same technique rustup's and Homebrew's
    installers use), so tier 4's prompts -- and `confirm_sudo_install`'s
    -- read from `/dev/tty` explicitly (`have_tty()`: `{ : < /dev/tty; }
    2>/dev/null`). This is what makes the README's own `curl | bash`
    one-liner show the real picker instead of always silently defaulting
    to core-only: only a genuinely absent controlling terminal (cron, CI,
    a container run without `-it`) falls through to tier 3.

    Deliberately unaffected by this requirement: `--musl`, `--yes-core`
    itself, checksum verification, and the `relay`/`testsuite` addons are
    out of scope entirely -- neither is fetched by this script (`tetron-
    relay`/`tetron-testsuite` have their own bringup, not part of the
    "suite").
    """
    req_id = "ADDONS-SUITE-002"


# --------------------------------------------------------------------------
# ADDONS-SUITE-003: tetron-hosts added as a fifth, opt-in-only component
# --------------------------------------------------------------------------

class InstallSuiteHostsComponent(Requirement):
    """REQUIREMENT-ID: ADDONS-SUITE-003

    Adds `tetron-hosts` (a new addon: syncs peer hostnames into
    `/etc/hosts` as `<hostname>.<network>`, see the `tetron-hosts` repo's
    own README for the full design) to `install-tetron-suite.sh` as a
    fifth component, alongside `core`/`webui`/`systray`/`backup`. Follows
    `ADDONS-SUITE-002`'s existing selection logic exactly (`--install-hosts`
    flag, `component_binary`/`component_repo` table entries, generic
    release-binary install path -- not `backup`'s special raw-script path)
    with one deliberate deviation from that requirement's tier-4 defaults:

    **`hosts` defaults to `N` in the interactive picker, unconditionally,
    same treatment as `backup` and for a related reason.** Unlike
    `webui`/`systray` (whose per-user services carry no elevated runtime
    privilege), `hosts` registers a **root-level system-wide scheduled
    service** (`component_service_needs_sudo` returns `1` for `hosts`,
    same tier as `core`, since writing `/etc/hosts` needs root regardless
    of who invokes it) -- not something to silently add to everyone's
    default install the moment this component exists. `--install-all`
    does include it (`core webui systray hosts backup`), matching
    `backup`'s own precedent of being opt-in-only in the picker but
    included when the caller explicitly asks for everything.

    **`tetron-hosts install`'s own further interactive wizard** (which
    networks to sync into `/etc/hosts`, whether to schedule automatic
    runs, at what interval) runs automatically as this script's normal
    last step for any component (`install_component`'s existing `"$dest"
    install` call, unchanged) -- no new plumbing needed here, since that
    wizard already reads from `/dev/tty` the same way this script's own
    prompts do, so it composes correctly under the same `curl | bash`
    conditions `ADDONS-SUITE-002` already handles.

    Unlike `relay`/`testsuite` (still explicitly out of scope per
    `ADDONS-SUITE-002`, each with its own separate bringup), `hosts` is a
    genuine "suite" component -- distributed as a normal versioned
    release binary this script tracks and upgrades like every other
    component, not a standalone-bringup addon.
    """

    req_id = "ADDONS-SUITE-003"


# --------------------------------------------------------------------------
# ADDONS-SUITE-004: upgrading is the default -- core is never silently
# left behind, and an installed component is never left stale
# --------------------------------------------------------------------------

class InstallSuiteUpgradesByDefault(Requirement):
    """REQUIREMENT-ID: ADDONS-SUITE-004

    Reported live by USER 2026-08-16, running the README's own documented
    one-liner (`curl -fsSL .../install-tetron-suite.sh | bash`): it
    upgraded the addons and left `core` behind, so a freshly upgraded
    `webui`/`systray` was left talking `tetron-proto` to a stale daemon --
    exactly the version skew the matched-release discipline exists to
    prevent, reintroduced at install time. The run reported success.

    Three separate defects combined to produce it, all fixed here:

    1. **`core`'s upgrade prompt defaulted to "no".** `confirm_sudo_install`
       asked `[y/N]` for an upgrade of an already-installed `core`, so a
       bare Enter declined it. The prompt immediately before it
       (`ADDONS-SUITE-002` tier 4's `Use defaults? [Y/n]`) trains Enter,
       and every other prompt in the script is `[Y/n]` -- so the one
       component that matters most was the one keystroke-declined by
       muscle memory. It then `continue`d silently, and the script still
       exited `0` behind a row of green addon lines, which is why this
       went unnoticed long enough to be reported from a real fleet rather
       than caught here.
    2. **No controlling terminal skipped `core` outright** unless
       `--yes-core` was passed (cron, CI, a container without `-it`).
    3. **A component installed on this host but outside the computed
       default set was never even considered.** `ADDONS-SUITE-002`'s
       display-aware default set (`core` alone when headless) governed
       upgrades as well as fresh installs, so a headless machine with
       `webui` installed would never upgrade it -- silently, forever.

    **This supersedes `ADDONS-SUITE-002`'s `--yes-core` upgrade gate.**
    That gate was written to protect a real property: upgrading `core`
    restarts the daemon and briefly drops every peer on the host. The
    protection was aimed at the wrong target. Someone running an installer
    named "install-tetron-suite" is *asking* to be upgraded; the surprise
    worth preventing is a host left on mismatched versions, not a
    momentary reconnect. So the default inverts:

    - Upgrading `core` proceeds by default. The prompt still states the
      cost plainly ("briefly disconnects every peer on this host") but is
      `[Y/n]`, and an explicit `n` still declines.
    - With no controlling terminal, `core` upgrades rather than skipping,
      logging one warning saying so.
    - **`--no-core`** is the new opt-out, and is the supported way to
      express "addons only". Declining must be a deliberate statement,
      not an inferred keystroke.
    - **`--yes-core` is accepted and ignored**, not removed and not an
      error: it appears in existing cron entries, CI jobs, and previously
      documented command lines, and there is nothing left for it to
      permit. Rejecting it via the catch-all `unrecognized argument`
      would break exactly the automated callers that took the old advice.
    - `--core-only` together with `--no-core` is a usage error; they
      select nothing between them.

    **Already installed implies always upgraded.** The component set is
    the union of what the selection logic chose and everything already
    present on the host, `--check` included (reporting the version of an
    installed-but-unselected component is what a status check is for).
    `ADDONS-SUITE-002`'s selection tiers keep their full meaning for what
    gets **newly installed**; they no longer decide what gets left stale.
    Two explicit overrides survive the sweep: `--core-only`, and any addon
    explicitly declined at the tier-4 picker -- answering "no" to a
    component has to mean no, even for one already present. Conversely,
    the picker now pre-answers `Y` for any component already installed
    (including `hosts`/`backup`, otherwise unconditionally `N` per
    `ADDONS-SUITE-003`), since that prompt is offering an upgrade rather
    than a new install.

    **Skips are reported, never silent.** Anything selected but not
    installed is listed in a closing summary, and a skipped `core` gets
    its own warning naming the consequence (addons speak `tetron-proto`
    to the daemon). The original defect was silent success; the exit code
    alone is not enough to convey it.

    Verified by `contrib/tests/install-suite-confirm.test.sh`, which
    extracts `confirm_sudo_install` from the shipped script at run time
    (so the test cannot drift from the code) and drives its prompt reads
    from a file rather than `/dev/tty`. Its twelve cases pass here and
    reproduce defects 1 and 2 as failures against the pre-fix script,
    leaving the fresh-install and addon paths unchanged.
    """

    req_id = "ADDONS-SUITE-004"


# --------------------------------------------------------------------------
# KICK-COORDINATOR-001: any coordinator can kick any other coordinator
# --------------------------------------------------------------------------

class KickAllowsCoordinatorTarget(Requirement):
    """REQUIREMENT-ID: KICK-COORDINATOR-001

    `tetron kick <net> <endpoint-id>` used to refuse unconditionally when
    the target held the network key (`kick_member`'s `if is_coord { refuse
    }` block, `runtime.rs`) -- not just against the sole coordinator, against
    *any* coordinator, always, with no override. Found 2026-07-25 to be a
    real operational gap, not a theoretical one: `coordinator_count()`
    (`membership.rs`) counts roster-flagged coordinators with no liveness
    concept at all, so a coordinator whose machine permanently dies or is
    reinstalled stays `is_coordinator: true` in the roster forever, with no
    way to remove that stale entry via any interface.

    Two concrete consequences of the old refusal, both from the same stale
    count: (1) `leave_network`'s stranding-safety check
    (`is_sole_coordinator = ... && coordinator_count(&roster) <= 1`) is
    silently defeated for a genuinely-solo surviving coordinator once a
    zombie coordinator inflates the count to 2 -- they can `leave` with no
    warning and no `--force`, stranding the network anyway, exactly the
    outcome that check exists to prevent. (2) `nuke_network`'s consensus
    gate reads the same stale count and can permanently deadlock: "2
    coordinators" on paper, one of whom can never propose or second again.
    Separately, in a network where most members are coordinators
    (COORD-001's laptop-fleet model), `kick` was close to useless -- almost
    nobody was a legal target.

    DECISION (superseding an initial "add a demote step, gate it behind
    nuke-shaped consensus" direction that was considered and rejected):
    **no demote primitive, no consensus.** Any coordinator may kick any
    other coordinator directly and unilaterally, the same way any
    coordinator can already unilaterally kick any ordinary member today.
    Reasoning, worked through with USER:

    - Kicking a coordinator was never going to be real key revocation
      either way. `AdminGrant` hands every coordinator a copy of the
      *same* shared `network_secret_key` (not a distinct key/certificate
      per coordinator), and `invite_create`'s only gate
      (`coordinator_handle()` -> `handle.role.is_coordinator()`) is
      derived purely from a node's own **local** possession of that key,
      never from the roster. So neither a consensus-gated demote nor a
      unilateral one can stop a still-genuinely-live former coordinator
      from self-minting an invite and rejoining with their own copy of
      the key -- a `--force`-shaped safety rail here would have protected
      against a risk (irreversible harm) that plain kick, unlike `nuke`,
      does not actually carry.
    - Because kicking a coordinator can't do anything a still-live target
      can't immediately undo themselves (self-invite) or that any other
      coordinator can't immediately redo the other way (`admin add`
      re-grants), it is fully reversible by construction -- the opposite
      risk shape from `nuke` (irreversible destruction), which is why
      `nuke` earns consensus (NUKE-CONSENSUS) and this does not.
    - It is not a new capability class: `kick_member`'s top-level gate
      already lets any single coordinator unilaterally kick any ordinary
      member with no consensus. Coordinators were an arbitrary carve-out
      from a tool that was already unilateral for everyone else; removing
      the carve-out is closing an inconsistency, not opening a new one.
    - A separate demote-without-removal primitive (clear `is_coordinator`,
      keep the target as an ordinary member) was considered and explicitly
      rejected as a prerequisite: it only serves downgrading a *live,
      cooperative* coordinator's trust while keeping them around, which is
      a different feature from "remove a zombie admin" and isn't needed to
      solve it.

    Implementation: delete the `if is_coord { refuse }` block from
    `kick_member` entirely. No other change to the function is needed --
    `remove_member_roster_only` already does `s.members.remove(&member_id)`,
    which drops the target's whole `Member` entry including
    `is_coordinator`, so this single deletion also fixes both
    `coordinator_count()`-driven bugs above as a side effect (the zombie's
    entry is gone, not left behind with a merely-cleared flag). The
    pre-existing self-kick refusal (`"cannot kick yourself"`) is unrelated
    and unchanged. The success message now names whether the target was a
    coordinator and, if so, states plainly what this does and does not do
    (removes roster/enforcement access; does not revoke the key) --
    replacing the old refusal's own overclaim ("kicking can't remove its
    access. Revoke the key instead" pointed at a remedy that doesn't
    actually exist).

    EXPLICITLY OUT OF SCOPE: evicting a still-*live*, actually-malicious
    coordinator who won't cooperate. That needs real key rotation --
    generating a new `network_secret_key` and redistributing it to every
    still-trusted coordinator, which also relocates the network's own DHT
    discovery identity (the room id is the key's own derived pubkey) --
    a much bigger, separate, not-currently-scoped project. This
    requirement solves "remove a zombie admin," the stated priority, and
    is deliberately honest about not solving the other case.

    Doc updates: CLI help text for `tetron kick` (drop any coordinator
    caveat), AGENTS.md's kick bullet (the "Refused against
    coordinators/self" line becomes "Refused against self only"),
    CHANGELOG.md.
    """
    req_id = "KICK-COORDINATOR-001"


# --------------------------------------------------------------------------
# Self-capture routing mitigation (SELFCAPTURE-ROUTE-*)
# --------------------------------------------------------------------------

class SelfCaptureRoutingMitigation(Requirement):
    """REQUIREMENT-ID: SELFCAPTURE-ROUTE-001

    Fixes the original overlay self-capture bug (renamed `TUN-CAPTURE-001`,
    formerly "Bug 1"/`OVERLAY-SELFCAPTURE-001` during investigation): every
    tetron node's TUN device installs an OS subnet route (e.g. `10.88.0.0/24
    -> tun0`) that iroh does not know is virtual, so it can offer a peer's own
    overlay IP as a direct-dial candidate; any peer sharing that same subnet
    route locally swallows the resulting packet into its own kernel/tetron
    forwarder instead of ever reaching the real remote host -- causing
    relay-fallback flapping and higher latency than necessary on every peer
    pair, guaranteed by construction, not just occasionally.

    **Mechanism (decided and live-verified on both platforms 2026-07-24,
    never coded until now):** route iroh's own outbound control/data traffic
    around every overlay subnet route, identified by its one fixed source
    port (daemon-wide, not per-network -- one shared `Endpoint` serves every
    joined network, and iroh has no per-network identity to key on at the
    point it sends). A candidate address bled onto the wrong network never
    gets a chance to be a problem if iroh's own packets never take the
    overlay-shadowed route to begin with.

    - **Linux:** `ip rule add ipproto udp sport <port> table <T>` (a plain
      FIB rule match, no `nftables`/fwmark needed -- confirmed live on this
      dev machine that `ipproto`/`sport` selectors work directly), where
      table `<T>` (a fixed, arbitrary, tetron-owned id, distinct from the
      main/default/local tables and from whatever else a given system's own
      policy routing already occupies) holds nothing but a mirror of the
      real default route. Since table `<T>` never contains any overlay
      subnet route, this works uniformly for however many networks are
      joined without needing a per-network update.
    - **macOS:** a `pf` sub-anchor (`route-to`) loaded under the stock
      `nat-anchor "com.apple/*"` every macOS install already ships --
      matching rayfish's own already-proven `pfctl` integration in its exit-
      node feature. Loading a ruleset into a named anchor replaces its prior
      contents, so this is naturally idempotent without a separate
      check-first step the way the Linux `ip rule` path needs.

    **Applied daemon-wide, once, at `bootstrap::run_daemon` startup** (right
    alongside the existing `SUBNET-012` preflight) -- not at `tetron
    install`, since `ip rule`/`pf` state is runtime kernel state that does
    not survive a reboot; a fix living in `install` would silently stop
    working after the very first reboot with nothing pointing back at
    install as the cause. **Idempotent:** safe on every daemon start/restart
    -- the Linux path checks the existing rule's port (if any) before
    deciding whether to leave it, replace it (a reconfigured `listen-port`
    since last run), or add it fresh, and always `route replace`s (not
    `add`s) the shadow default route, tolerating a changed real gateway
    across restarts. **Fail-open:** a missing tool or failed command logs a
    warning and the daemon starts normally regardless -- this is a
    best-effort mitigation, not a correctness requirement, so it must never
    block startup.

    **Configurable, on by default** (`tetron config set selfcapture-
    mitigation off`, matching the existing `CONFIG-AUDIT-002` key style,
    documented in `tetron config --help`) -- an advanced user running their
    own conflicting policy-routing setup needs an escape hatch. Disabling it
    actively tears down any rule/anchor already applied from an earlier run
    (found live-testing: a naive "just skip re-applying" implementation left
    a stale rule in place with no way to see it reflected in config at all)
    -- symmetric with enabling it, not just a one-way switch.

    **Torn down only at `tetron uninstall`, not on ordinary `tetron
    stop`/restart** -- mirrors existing precedent: TUN devices themselves are
    not torn down on ordinary stop/start either, only on actual network
    `leave`/`nuke` (`teardown_network_runtime`), and since this mitigation
    isn't tied to any specific network there is no equivalent event to hook
    an earlier teardown to. An unclean daemon exit (crash, the fail-fast
    panic-abort) just leaves the existing rule/anchor in place, which the
    next start's idempotent apply recognizes as already-correct rather than
    duplicating -- so imperfect teardown on a crash is harmless, not a
    correctness gap.
    """
    req_id = "SELFCAPTURE-ROUTE-001"


# --------------------------------------------------------------------------
# PATH-BLEED-001 status-layer fix (PATHBLEED-STATUS-*)
# --------------------------------------------------------------------------

class PathBleedSubnetScopeFilter(Requirement):
    """REQUIREMENT-ID: PATHBLEED-STATUS-001

    Fixes `PATH-BLEED-001` (the cross-network path-sharing status bug found
    2026-07-26, see `DO-NOT-COMMIT/FINDINGS_PathBleed_DataLossAnalysis.md`
    and `DO-NOT-COMMIT/RESULTS_PathBleed_DataLossTest.md`): iroh's
    `RemoteStateActor` tracks path state per **peer identity**, not per
    tetron network, so a path selected on one of a node's networks gets
    broadcast onto that same peer's connection on every other network they
    share -- `tetron status` can display a peer's *other*-network overlay
    address as this network's own `Direct` remote address. Both the source
    dive and a live VM test (tagged-UDP traffic during an active bleed, zero
    misdelivery across 900 packets) already confirmed this causes no
    misdelivery and no bleed-attributable data loss -- it is a status/
    observability bug only, so the fix lives entirely in `tetron status`'s
    own path-selection-for-display logic, not iroh's data plane.

    `src/daemon/mesh/select.rs`'s `choose_path_index` decides what
    `gather_conn_info` (`src/daemon/mesh/diagnostics.rs`) reports: today it
    unconditionally trusts iroh's own `is_selected()` flag, which is exactly
    the value PATH-BLEED-001 can poison. `choose_path_index`'s signature
    gains a third field per candidate -- `in_subnet: bool` -- computed by
    `gather_conn_info` from each path's address against *this specific
    network's own* subnet: `membership::ip_in_subnet` for a `TransportAddr::Ip`
    v4 address (already existed, used by `SUBNET-012`/`SUBNET-COLLISION-002`),
    a new `membership::ipv6_in_network` sibling for a v6 address (checking the
    address's own /56 against `membership::ipv6_network_prefix(network_key)`,
    IPV6-001's per-network scoping), and unconditionally `true` for
    `Relay`/`Custom` paths -- a relay URL is not network-scoped in the first
    place (tetron's `relay`/`discovery-dns` config is daemon-wide), so there
    is nothing bleed-shaped to check there.

    A disqualified (`in_subnet == false`) candidate is excluded from **both**
    the `is_selected()`-preference check and the Direct>Relay>Tor fallback
    scan -- not just stripped of its selected flag, since the existing
    fallback loop matches by classification alone and would otherwise
    re-surface the same wrong address a moment later. If literally nothing
    trustworthy remains, `choose_path_index` returns `None` (renders as `?`)
    rather than confidently reporting a definitely-wrong address.

    `SUBNET-COLLISION-001` (landed first, on purpose) makes this filter's
    core assumption -- "a peer's address outside this network's own subnet
    can't legitimately belong to this network" -- structurally guaranteed
    for any node that joins after it shipped, not just a good heuristic;
    this filter remains the runtime safety net for pre-existing installs
    that already have overlapping-subnet networks from before that guard
    existed.

    **This check was too narrow, not meaningless -- corrected by
    `PATHBLEED-STATUS-003`, found 2026-08-02 while diagnosing a live
    incident.** `network_subnet` here is *this specific network's own*
    virtual overlay subnet. A genuine Direct candidate's address (a real
    LAN/public IP) will never fall inside it -- so checking only against
    the currently-queried network's subnet rejects genuine candidates
    almost universally. But the check itself is not meaningless: a bled
    candidate's address (iroh offering a peer's own overlay IP on a
    *different* one of its networks as a "direct" candidate,
    `SELFCAPTURE-ROUTE-001`) genuinely does fall inside *some* overlay
    subnet -- just not necessarily this one. `PATHBLEED-STATUS-003`'s
    corrected fix checks against every network this daemon manages, not
    only the one being queried, and does **not** conclude
    `PATHBLEED-STATUS-002` is sufficient alone -- see that requirement's
    own corrected docstring.
    """
    req_id = "PATHBLEED-STATUS-001"


class PathBleedActivityCorroboration(Requirement):
    """REQUIREMENT-ID: PATHBLEED-STATUS-002

    Hardening layer on top of `PATHBLEED-STATUS-001`, same status-only scope
    (no data-plane change). iroh's public `Path::stats()` exposes real
    per-path counters (`udp_tx`/`udp_rx` bytes and datagrams) -- a
    freshly-opened, never-actually-used candidate reads as zero real traffic
    on its own stats even while `is_selected()` claims it and even while it's
    in-subnet. This targets the residual case `PATHBLEED-STATUS-001`'s
    subnet check alone can't catch: two of a node's own networks that happen
    to share an identical subnet from before `SUBNET-COLLISION-001` existed,
    where a bled candidate looks legitimately in-subnet by coincidence but
    has never actually carried *this* connection's traffic.

    `choose_path_index` becomes three tiers, each restricted to `in_subnet`
    candidates only: (1) selected *and* (active or the sole trustworthy
    candidate) -- trusted outright; (2) no tier-1 winner: any candidate with
    real activity, by class (Direct > Relay > Tor), regardless of
    `is_selected()`; (3) nothing has proven itself yet: plain classification
    among all trustworthy candidates, so a genuinely new, still-validating
    path is still reported rather than hidden as `?` just for being new.

    **Found live while writing this requirement's own tests, not just
    designed up front:** an earlier version only gated the *selected*-
    preference check on activity and left the classification fallback
    unchanged -- which meant a selected-but-inactive, in-subnet Direct
    candidate that failed the activity gate still won anyway via the
    fallback's own Direct-first classification preference, silently
    defeating the entire hardening layer for exactly the case it was built
    for. The three-tier restructure (activity as its own tier, ahead of
    plain classification) is what actually achieves "an unselected but
    active alternative outranks a selected-but-inactive one."

    **Corrected 2026-08-02 (`PATHBLEED-STATUS-003`): "never-actually-used
    reads as zero" was only ever true of `udp_rx`, not `udp_tx`.** Traced
    `noq-proto-1.1.0` directly: `udp_tx` increments for any outgoing
    datagram on a path, including the QUIC `PATH_CHALLENGE` probe sent to
    *validate* a brand-new candidate -- so a doomed, never-validating bled
    candidate reads activity almost immediately, on its first probe, not
    only once real traffic is confirmed. `has_activity` (`diagnostics.rs`)
    now checks `udp_rx.bytes > 0` -- incremented only on actual receipt --
    which is what this requirement's own reasoning always assumed it was
    checking.
    """
    req_id = "PATHBLEED-STATUS-002"


class PathBleedDropUselessSubnetCheck(Requirement):
    """REQUIREMENT-ID: PATHBLEED-STATUS-003

    Found 2026-08-02 diagnosing a live incident (a fresh, clean two-VM
    reproduction -- no Docker/Tailscale/libvirt, same virtual LAN -- showed
    `via_detail: DirectBled` for a genuine Direct candidate on both sides,
    continuously, from 10s through 220s after join; confirmed the same
    version boundary against real fleet history: `macbookpro` on v0.8.0
    (predates this check, `git merge-base --is-ancestor` confirms) showed
    real `Direct`, while this box on v0.8.2 (confirmed *already includes*
    `PATHBLEED-STATUS-001`/`-002`) never once did, all session).

    **Corrected 2026-08-02, same day, before release -- an independent
    review (a separate, larger/newer model given a full handoff package,
    per the standing "check in at checkpoints" practice this session
    itself added to `DO-NOT-COMMIT/TODO.md`) found this requirement's
    first cut, committed as `a00eb88`, was itself wrong in exactly the
    class of way `PATHBLEED-STATUS-001` originally was: a fix for a real
    bug that broke a different real thing. That commit is superseded by
    this corrected version -- kept in git history, not the current
    design.**

    **What the first cut got wrong.** It collapsed `in_subnet` to an
    unconditional `true` for every candidate, reasoning that "a peer's
    real transport address is never scoped to a logical tetron network."
    That premise is false for the *exact* candidate `PATH-BLEED-001`'s own
    live VM reproduction recorded: `DO-NOT-COMMIT/
    RESULTS_PathBleed_DataLossTest.md:29-33` shows the bled `Direct`
    candidate's `remote_addr` was `10.88.1.147` -- the peer's own
    **overlay** address *on a different one of its networks*, not a real
    LAN/public address at all. iroh's own local-interface enumeration can
    pick up a node's TUN device and offer a peer's overlay IP as a
    "direct" candidate (`SELFCAPTURE-ROUTE-001` mitigates iroh's own
    *outbound* use of this, but does not remove the candidate from
    `conn.paths()`'s list) -- so a bled candidate's address is
    overlay-shaped, and checking it against *some* overlay subnet is
    exactly the meaningful signal the first cut wrongly concluded didn't
    exist. Collapsing `in_subnet` to `true` reopened `PATH-BLEED-001`'s
    original symptom for this address shape while fixing it for the
    other (genuine real-address) shape -- net effect, a regression traded
    for a regression.

    **Also wrong: leaning on `PATHBLEED-STATUS-002`'s `has_activity` alone
    is not sufficient.** Traced `noq-proto-1.1.0`'s actual source: `udp_tx`
    (`connection/mod.rs`'s `build_transmit`, ~line 1245) increments for
    *any* outgoing datagram on a path, including the QUIC `PATH_CHALLENGE`
    probe sent to validate a brand-new, unproven candidate (`connection/
    mod.rs` ~line 6152-6172, flows through the same `build_transmit`).
    So a doomed, never-validating bled candidate reads `has_activity: true`
    almost immediately -- on its first validation attempt, not only once
    real traffic is confirmed. `udp_rx` (incremented only on actual
    receipt, `connection/mod.rs` ~line 2231) is the signal that is
    actually safe to lean on; `PATHBLEED-STATUS-002`'s own docstring's
    "never-actually-used bled candidate reads as zero" claim was only
    ever true of `udp_rx`, not `udp_tx`.

    **The corrected fix, two parts:**

    1. **`in_subnet` checks against every overlay subnet/network-prefix
       this daemon manages, not just the currently-queried network's
       own.** `classify_candidate_addr` (`select.rs`) gains
       `managed_subnets: &[Subnet]` (v4) and `managed_network_keys:
       &[EndpointId]` (v6, for `ipv6_in_network`) parameters -- computed
       once in `MeshManager::status()` (every joined network's own
       `subnet`/`network_key`) and threaded through `network_status` into
       `gather_conn_info`. A candidate's address is trustworthy
       (`in_subnet: true`) when it falls inside **none** of them (a
       genuine real address never will); it is treated as a bled/
       self-captured overlay address (`in_subnet: false`, excluded) when
       it falls inside **any** of them (including, harmlessly, the
       currently-queried network's own -- a genuine candidate would never
       coincidentally match that either). `ip_in_subnet`/
       `ipv6_in_network` (`membership.rs`) are unchanged, just called
       against the full managed set instead of one network.
    2. **`has_activity` checks `udp_rx.bytes > 0`, not `udp_tx.bytes > 0`**
       (`diagnostics.rs::gather_conn_info`) -- receipt-confirmed traffic,
       not merely attempted transmission.

    Re-deriving `choose_path_index`'s four tiers with this corrected
    `in_subnet`: unchanged in shape from `PATHBLEED-STATUS-001`'s
    original tiers, since `in_subnet` is `true`/`false` again (just
    computed correctly this time) -- no tier restructuring, same as
    before.

    **`PATH-DIAG-004`'s `DirectBled` is reachable again**, correctly this
    time -- its own doc-comment "currently unreachable" note (added when
    the first cut shipped) is removed; it was never accurate for more
    than a few hours.

    **Also caught by the same review, noted but not fixed by this
    requirement** (separate, smaller follow-ups, see `DO-NOT-COMMIT/
    TODO.md`): `reconcile.py`'s `cargo test`/`cargo clippy` invocations
    don't pass `--workspace`, so `tetron-proto`'s own tests and lints are
    never checked by the per-commit gate; a stale "Draft for review, not
    yet implemented" note was left in this file's own `PATH-DIAG-*`
    section header after that batch actually shipped; `classify_via_detail`
    can report `DirectUnvalidated` even when the Direct candidate genuinely
    has activity, if `Relay` itself currently holds tier-1 priority (a
    labeling-precision issue, not a trust/security one).

    **Options considered and rejected, still true after the correction:**
    the actual root-cause architectural fix (a separate iroh `Endpoint`
    per network, so there is no shared peer-identity bookkeeping to bleed
    across networks at all) remains technically possible but a much
    bigger change than this bug warrants -- noted as a future option, not
    attempted here.

    **Second independent review (2026-08-03, checkpoint 2), follow-ups
    landed same day.** The corrected fix's core logic was confirmed sound
    and safe to land; the review's concrete findings were addressed
    directly rather than left as more TODO items:

    - `tetron-proto/src/ipc.rs`'s `PathCandidateInfo::in_subnet`/
      `has_activity` doc comments still described the *first* cut's
      semantics ("this specific network's own subnet"; "`udp_tx.bytes >
      0`") on a struct that ships over the wire to `tetron-webui`/
      `tetron-systray` -- corrected to match this requirement's actual
      scope (every managed subnet) and signal (`udp_rx`).
    - `MeshManager::status()`'s `managed_subnets` computation
      (`diagnostics.rs`) used `.read().ok().map(...)`, silently dropping
      a network from the exclusion set entirely if its state lock were
      ever poisoned -- fails open on exactly the trust boundary this
      requirement exists to enforce. Changed to recover the guard's data
      via `PoisonError::into_inner()` instead (same idiom already used
      elsewhere in this codebase, e.g. `logdir.rs`/`identity.rs`'s
      `ENV_LOCK`), so a poisoned lock still contributes its subnet to the
      exclusion set rather than silently widening what counts as
      trustworthy.
    - `choose_path_index`'s own doc comment (`select.rs`) still cited
      "two of a node's own networks happening to share an identical
      subnet, from before `SUBNET-COLLISION-001`" as the residual case
      `PATHBLEED-STATUS-002`'s activity gate exists to catch -- true of
      the *original* `PATHBLEED-STATUS-001` design (which only checked
      the currently-queried network's own subnet, so a coincidental
      collision was the only way a same-daemon bleed got caught at all),
      but no longer the operative reasoning once `-003` checks every
      managed subnet directly. Corrected to name the reasoning below
      instead.

    **Two residual gaps, documented as permanent known limitations, not
    fixed** (both narrower than the original bug, in the same class of
    incompleteness that motivated writing this requirement's own history
    down rather than treating any single cut as final):

    1. **Bled candidate from a network the peer shares but this daemon
       does not.** `managed_subnets` is built from `self.networks` --
       *this* daemon's own joined networks. If the peer offers a
       candidate that is their own overlay address on some *other*
       network of theirs that this daemon isn't a member of, it never
       appears in `managed_subnets` at all, so the subnet check cannot
       exclude it -- `PATHBLEED-STATUS-002`'s `has_activity` gate is the
       only remaining defense (an unvalidated candidate still reads no
       activity), same as before this requirement's correction, just
       narrowed from "always insufficient" to "insufficient only for
       this specific unshared-network shape."
    2. **Inverse false positive.** If a user's real LAN subnet
       numerically overlaps one of their own chosen overlay subnets --
       never true by default (tetron's `10.88.0.0/24` default vs. a
       typical home `192.168.x.0/24` LAN) but possible with a
       user-configured overlay range -- a genuine Direct candidate on
       that LAN would be wrongly excluded as if it were a bled overlay
       address. Not observed in practice; noted here so a future report
       of "direct never wins even though the LAN address is clearly
       right" is recognized rather than re-diagnosed from scratch.

    `docs/ARCHITECTURE.md` and `docs/CONNECTIVITY.md` updated alongside
    (both described pre-`-003` design, and `CONNECTIVITY.md`'s "Planned
    Observability" table listed `PATH-DIAG-001/002/004`'s already-shipped
    fields as not-yet-built).
    """
    req_id = "PATHBLEED-STATUS-003"


# --------------------------------------------------------------------------
# PATH-DIAG-*: relay-vs-direct path observability (Level 1 instrumentation)
# --------------------------------------------------------------------------
#
# Implemented and shipped 2026-08-02 (PATH-DIAG-001/002/004; PATH-DIAG-003
# was scoped and then dropped before implementation, see TODO.md).
# Motivated by a live incident 2026-08-02 (Android tablet + several
# LAN machines reporting relay while carrying real traffic; one peer's own
# status showed `Direct` for connections this daemon reported `Unknown`/
# `Relay` for). Full background: `DO-NOT-COMMIT/RESEARCH_RelayVsDirect_iroh.md`.
# Human-readable design: `DO-NOT-COMMIT/PLAN_RelayVsDirect_Level1Instrumentation.md`.
#
# All four requirements below extend `choose_path_index`/`gather_conn_info`
# (`src/daemon/mesh/select.rs`, `src/daemon/mesh/diagnostics.rs`), the exact
# functions `PATHBLEED-STATUS-001`/`-002` above already modify -- this is a
# further layer on the same status-observability logic, not a new subsystem.
#
# Checked against `MINIMAL-006` (removed `torpedo ping`/`torpedo netcheck`
# plus the original, larger `daemon/mesh/diagnostics.rs`) and `MINIMAL-009`
# (removed the Prometheus exporter and `torpedo report` bundle, but
# explicitly kept "per-peer counters that status display... needs... as
# plain fields"): none of PATH-DIAG-001..004 reintroduces active probing or
# an export surface. All four are passive -- surfacing state iroh and
# tetron already compute/receive, the same category MTU-DIAG-001 already
# established as in-scope post-MINIMAL. `tetron ping --paths`/`tetron
# probe`/`tetron-connectivity-watch` (Level 2/3 of the same brainstorm)
# would need their own explicit reckoning with `MINIMAL-006` when/if they're
# specced -- flagged here so it isn't missed later, not addressed by this
# batch.

class PathTransitionLogging(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-001

    Subscribes to iroh's `Connection::path_events()` (vendored
    `iroh-1.0.3`, `src/endpoint/connection.rs:1161-1178` -- a live stream of
    path-opened / path-closed-with-final-stats / selected-path-changed /
    `Lagged` events, ending when the connection closes) once per peer
    connection, logging each event at `debug`/`info`. Not subscribed to
    anywhere in tetron today.

    Placement: alongside the existing per-peer reader/reconnect task
    (`info_span!("peer"/"reconnect", …)` per `AGENTS.md`'s tracing
    conventions), so these log lines are already correlated by that span
    without inventing a new one. Whether this is a distinct spawned task per
    connection or folded into the existing per-peer reader loop directly is
    an implementation decision, not fixed by this requirement -- the
    observable requirement is only that transitions get logged, not the
    task topology that logs them.

    Pure `tracing` output -- no IPC/wire change, no new `ConnectionInfo`
    field. `PATH-DIAG-002` and `PATH-DIAG-004` do not depend on this one.

    **`PATH-DIAG-003` (connection age, timestamped from this subscription)
    was dropped 2026-08-02 before implementation** -- see
    `DO-NOT-COMMIT/TODO.md`'s "Connection-age tracking, deferred" entry.
    Its payoff turned out to be narrow (only meaningfully distinct from
    `PATH-DIAG-004`'s `DirectUnvalidated` case) and this task's own log
    lines already carry timestamps an external consumer can use to derive
    the same thing closely enough, without tetron core threading new
    per-connection state through `MeshCtx`/`ForwardCtx`.

    Implemented as a small `log_path_events` task spawned once from within
    `spawn_peer_reader` itself (`src/forward.rs`) -- not at any of its seven
    external call sites -- sharing the reader's own connection (cloned) and
    span, so no call site needed to change.

    **Found while writing this requirement's own tests, not just designed
    up front:** iroh's `PathEvent` is `#[non_exhaustive]` at both the enum
    and every struct-variant level, so tetron's own test code cannot
    construct a `PathEvent` to hand a synthetic unit test -- only iroh
    internals can. Getting a real one needs an actual live connection.
    Decision (USER, 2026-08-02): skip a synthetic unit test for this one
    requirement rather than pull in iroh's `test-utils` cargo feature
    (in-memory `TestNetwork`/`TestTransport`, `src/test_utils/
    test_transport.rs`) purely to obtain one -- a real new dependency
    feature combination and test pattern for this codebase, disproportionate
    to a ~10-line logging item. Verified instead via `cargo build`/`clippy`
    (the non-exhaustive `match` forces every variant to be handled) and a
    `tetron-testsuite` live check. Revisit test-utils if a future
    `PATH-DIAG-*` (or unrelated) change needs to unit-test logic consuming
    real iroh path/connection events -- flagged as a TODO at
    `log_path_events`'s own doc comment in `src/forward.rs`, not re-decided
    here.
    """
    req_id = "PATH-DIAG-001"


class PathCandidateListExposure(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-002

    `gather_conn_info` (`src/daemon/mesh/diagnostics.rs:172-251`) already
    computes, per connection, a `classes: Vec<(ConnType, bool, bool, bool)>`
    -- one tuple per candidate path: `(conn_type, is_selected, in_subnet,
    has_activity)`, the exact data `PATHBLEED-STATUS-001`/`-002` already
    rely on -- plus each path's `.rtt()`, all discarded after
    `choose_path_index` picks one winner. This requirement stops discarding
    it: `ipc::ConnectionInfo` (`tetron-proto/src/ipc.rs:365-382`) gains a new
    field carrying the full candidate list, populated from data already
    computed in the same function -- no new instrumentation logic, purely
    plumbing already-live data out to `--json`.

    New wire type (naming subject to review):
    ```rust
    pub struct PathCandidateInfo {
        pub conn_type: ConnType,
        pub remote_addr: String,
        pub is_selected: bool,
        pub in_subnet: bool,
        pub has_activity: bool,
        pub rtt_ms: Option<f64>,
    }
    // on ConnectionInfo:
    #[serde(default)]
    pub paths: Vec<PathCandidateInfo>,
    ```
    `#[serde(default)]` matches the existing precedent
    (`ConnectionInfo::max_datagram_size`, MTU-DIAG-001) so an older daemon's
    response still decodes.

    `--json` only for now, matching `MTU-DIAG-001`'s and `STATUS-002`'s
    existing precedent that per-connection detail beyond the aligned
    summary table is `--json`-only -- the plain-text `via` column is
    unchanged by this requirement. `PATH-DIAG-004` depends on this landing
    first (it classifies against this same candidate list).

    **Cross-repo impact:** `tetron-proto`'s `ConnectionInfo` is the shared
    wire type `tetron-webui` and `tetron-systray` both depend on. Additive
    and backwards-compatible, but per standing practice
    (`feedback_always_check_addons_on_wire_changes` in auto-memory) both
    addon repos need an explicit `Cargo.toml` bump once this lands -- not
    an assumed auto-follow. Whether either addon's UI renders the new field
    is out of scope for this requirement (see each addon's own
    `DO-NOT-COMMIT/TODO.md`).
    """
    req_id = "PATH-DIAG-002"


class ViaDetailReasonField(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-004

    Today, several genuinely different situations all render identically as
    plain `relay` (or `?`) via `choose_path_index`. This requirement adds an
    explicit machine-readable reason distinguishing them, classified from
    the same `classes` data `PATH-DIAG-002` already exposes -- depends on
    that requirement landing first, since it needs the full candidate list
    in scope rather than re-deriving it separately.

    **Corrected 2026-08-02 while implementing this requirement, not just
    designed up front -- traced `choose_path_index`'s actual four tiers
    precisely rather than trusting an earlier summary of it, matching
    `PATHBLEED-STATUS-002`'s own precedent of documenting a real
    implementation-time discovery in this docstring.** The original design
    here listed a third case, "a direct candidate exists and even carries
    traffic, but iroh hasn't marked it `is_selected` yet, still reports
    relay." That case cannot actually happen: tier 2
    (`ct == want && in_subnet && has_activity`) does not check `is_selected`
    at all, so an in-subnet, active Direct candidate always wins at tier 2
    regardless of iroh's own selection state. What the original design
    missed instead: tier 3 (`ct == want && in_subnet`, no activity check)
    means an in-subnet Direct candidate with **zero** activity still wins
    unless tier 2 already matched something else in-subnet-and-active
    first -- and separately, a Direct-*shaped* candidate can be excluded
    from *every* tier by being **out-of-subnet**
    (`PATHBLEED-STATUS-001`'s cross-network-bleed exclusion), a genuinely
    different reason than "unvalidated."

    Corrected three-way split:
    - `NoDirectCandidate` -- no `Direct` entry in the candidate list at
      all (in-subnet or otherwise).
    - `DirectUnvalidated` -- an in-subnet Direct candidate exists but
      lacks real traffic, and something else with activity wins instead
      (the genuine `PATHBLEED-STATUS-002` case: tier 2 matched a
      different, active, in-subnet candidate before tier 3 ever got a
      chance to fall back to the inactive Direct one).
    - `DirectBled` -- a Direct-shaped candidate exists but was excluded
      as out-of-subnet (`PATHBLEED-STATUS-001`'s own exclusion) --
      replaces the original, inaccurate `DirectNotYetSelected`.

    Only populated when the reported `conn_type` is *not* `Direct` --
    `None` when it is, since there is nothing to explain. If both an
    in-subnet-but-inactive Direct candidate and an out-of-subnet
    Direct-shaped one are present simultaneously, `DirectUnvalidated`
    takes priority over `DirectBled` as the more directly actionable
    reason -- an edge case, not expected to matter in practice.

    New wire type (naming subject to review):
    ```rust
    pub enum ViaDetail {
        NoDirectCandidate,
        DirectUnvalidated,     // the PATHBLEED-STATUS-002 case
        DirectBled,            // the PATHBLEED-STATUS-001 case
    }
    // on ConnectionInfo:
    #[serde(default)]
    pub via_detail: Option<ViaDetail>,
    ```

    Directly motivated by this session's live incident: manually
    re-deriving this exact distinction by hand from raw `--json` candidate
    data (before `PATH-DIAG-002` even existed to make that data available
    normally) is what surfaced the asymmetric-classification finding in the
    first place -- this requirement makes that reasoning a first-class,
    queryable field instead of something only reconstructable by an agent
    or engineer reading source and cross-referencing two machines' status
    output by hand.

    `--json` only for now, same precedent as `PATH-DIAG-002`; whether
    plain-text ever shows it inline (e.g. `relay (unvalidated)`) is an open
    question for review, not decided here.
    """
    req_id = "PATH-DIAG-004"


# --------------------------------------------------------------------------
# PATH-DIAG-005..007: path-event log-noise mitigation. Distinct motivation
# from PATH-DIAG-001/002/004 above (those expose candidate/classification
# data via `--json`; these contain log *volume*, a live-log problem, not a
# status-display one) -- found live 2026-08-12 investigating a real,
# still-open flapping connection (`rpi5-test`, TODO_DETAILS.md
# #path-candidate-flapping). Dependency order: PATH-DIAG-005 must land
# first (it consolidates two independent subscribers into the one place
# -006/-007 both build on); -006 and -007 have no dependency on each other
# and can land in either order.
# --------------------------------------------------------------------------

class DeduplicatePathEventLogging(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-005

    Found live 2026-08-12: `spawn_path_logger` (`src/lib.rs`) and
    `log_path_events` (`src/forward.rs`, `PATH-DIAG-001`) both
    independently subscribe to the same `Connection::path_events()` stream
    for the same connection -- every path transition is logged twice, in
    two different formats. Traced via call-site grep: `spawn_path_logger`
    is called explicitly from `accept.rs` (twice), `join.rs`, and
    `create_join.rs`; every one of those same connections is *also*
    registered via `register_mesh_peer`/`spawn_peer_reader`
    (`src/forward.rs`), which spawns `log_path_events` on the identical
    `conn.clone()` per `PATH-DIAG-001`'s own placement decision ("a small
    `log_path_events` task spawned once from within `spawn_peer_reader`
    itself... not at any of its seven external call sites"). The two
    loggers also disagree on level: `spawn_path_logger` logs `Opened`/
    `Closed`/`Selected` all at `info!`; `log_path_events` already logs
    `Opened`/`Closed` at `debug!` and only `Selected` at `info!` --
    `log_path_events` is the correctly-designed one, `spawn_path_logger`
    is the redundant, over-verbose one.

    Matters beyond tidiness: every multi-homed real user (laptop wifi +
    ethernet, phone wifi + cellular) generates some baseline path churn,
    and `spawn_path_logger`'s blanket `info!` level puts every one of
    those `Opened`/`Closed` transitions into the default production log
    stream (`info`, per `LOG-003`) for no reason -- the exact class of
    high-frequency, low-value-at-default-verbosity event `LOG-003` already
    keeps out via `trace!` for the five per-packet forwarding events.

    **Fix:** remove `spawn_path_logger` and its four call sites entirely.
    Its one behavior not already covered by `log_path_events` -- an
    initial one-time dump of paths already open at subscribe time (with
    `rtt`/`is_selected`) -- is folded into `log_path_events` itself, at
    `debug!` (consistent with `Opened`/`Closed`'s existing level, since
    it's the same class of low-value-at-scale diagnostic, not a lifecycle
    milestone). One subscriber per connection afterward, not two.
    """
    req_id = "PATH-DIAG-005"


class DebounceRepeatedPathSelection(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-006

    Depends on `PATH-DIAG-005` landing first (one consolidated event
    consumer to attach this logic to, rather than duplicating it in two
    places).

    Found live 2026-08-12: a real, still-open connection (`rpi5-test`,
    `TODO_DETAILS.md` #path-candidate-flapping) genuinely oscillates
    `Selected` between two of the *peer's* own local interfaces (its
    LAN address confirmed as `192.168.1.43`, not this machine's -- the
    original bug writeup's "presumably two local interfaces on this
    machine" guess was wrong) roughly every 20-50s, with no teardown in
    between. This is legitimate iroh/noq multipath tie-breaking behavior
    on two genuinely close-quality paths -- tetron has no control over
    the underlying selection algorithm (`spawn_path_logger`/
    `log_path_events` are both pure passive observers of iroh's own
    `PathEvent` stream, confirmed via grep: neither `is_selected()` nor
    `PathEvent::Selected` is read anywhere in `src/forward.rs` for an
    actual forwarding decision) -- so the only available mitigation is
    log volume, not behavior change. Expected to be common in the wild,
    not specific to this one test peer: any real user with two
    similar-quality paths (not just this repo's own rpi5-test hardware)
    will produce the same pattern.

    **Fix:** a configurable per-peer rate limit on `Selected` logging,
    matching the existing `tetron config set` pattern (`log-level`,
    `nuke-proposal-ttl`) rather than a hardcoded threshold. Two new
    settings:
    - `path-flap-threshold` (count, default TBD at implementation --
      small, e.g. single digits)
    - `path-flap-window` (duration, default TBD at implementation --
      tens of seconds, matching the observed real cadence)

    Within `log_path_events`'s own per-connection task (already a fresh
    task per connection lifetime -- no new shared/global state needed,
    the counter is a local variable in that task's own loop): track
    `(window_start, count_in_window)`. On each `Selected` event, if
    `now - window_start` exceeds the window, reset (`window_start = now`,
    `count = 1`) and log at `info!` (a fresh window's first transition is
    always shown -- a genuine, rare interface change for an ordinary user
    must not be silently dropped). Otherwise increment; log at `info!`
    while `count <= threshold`, `debug!` once it exceeds -- so a settling
    connection's first few real flips are visible, and only sustained
    churn within one peer's own window gets quieted.

    The counting/level decision is a pure function of
    `(now, window_start, count, threshold, window)` -> `(log_at_info,
    new_window_start, new_count)`, extracted and unit-tested directly
    (`PURE-LOGIC-001` pattern) -- unlike `PATH-DIAG-001`'s own
    already-documented constraint (iroh's `#[non_exhaustive]` `PathEvent`
    cannot be constructed by tetron's test code), this decision logic
    itself never touches a `PathEvent`, only plain timestamps/counts, so
    it is fully testable without a live connection.
    """
    req_id = "PATH-DIAG-006"


class SuppressSelfCandidatePathEvents(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-007

    Depends on `PATH-DIAG-005` landing first (same reason as
    `PATH-DIAG-006`: one consolidated event consumer).

    Found live 2026-08-12, root-caused by reading the vendored dependency
    source rather than assumed: the same `rpi5-test` connection's path
    events also included a *third* address, `10.77.0.113` -- confirmed via
    `tetron status` to be `rpi5-test`'s own tetron overlay IP, not a real
    external candidate. Distinct in kind from `PATH-DIAG-006`'s case:
    these never reach `Selected` at all -- `Opened` immediately followed
    by `Closed` within under a millisecond, since the address can never
    be validated as reachable. `PATH-DIAG-006`'s rate limiter would not
    catch this (it only throttles `Selected`), so this needs its own
    check.

    **Root cause, traced into the vendored dependency, not guessed:**
    `netwatch-0.19.1`'s `LocalAddresses::new()` (the code iroh's endpoint
    uses to discover this node's own advertisable direct addresses)
    enumerates every "up", non-loopback interface via
    `netdev::interface::get_interfaces()` with no filtering by interface
    name or type at all (`netwatch-0.19.1/src/ip.rs:37-48`) -- a `tun`
    device looks identical to a real NIC to this code. Every tetron node
    therefore unknowingly advertises its own tetron overlay IP as a raw
    NAT-traversal candidate to every peer. Checked `iroh::Endpoint`'s
    public builder for an exclusion hook: `addr_filter()`/`AddrFilter`
    exists, but its doc comment scopes it to `AddressLookupServices` (the
    DNS/pkarr discovery-record publish pipeline) -- a different code path
    from `local_direct_addrs`/`DirectAddr`
    (`socket/remote_map/remote_state.rs`), the one that actually feeds
    live multipath candidate negotiation. No public API found to exclude
    an interface from that path. This is upstream territory (affects any
    TUN-based application built on iroh, not tetron-specific) -- worth a
    future issue against `n0-computer/iroh` or `n0-computer/netwatch`,
    not attempted here.

    **Fix, scoped to what tetron can control without upstream changes:**
    tetron already knows every network's own overlay subnet
    (`NetworkState.subnet`, threaded to `log_path_events` as a new
    parameter via `spawn_peer_reader`). Any `PathEvent`'s `remote_addr`
    whose IP falls inside that subnet (`addressing::ip_in_subnet`) is
    provably a self-referential candidate -- log it at `debug!`
    unconditionally (not subject to `PATH-DIAG-006`'s window/threshold,
    since it is never legitimate at any frequency, unlike a real
    `Selected` flip). Suppresses the *log noise* only; does not and
    cannot (absent the upstream API gap above) prevent iroh from
    attempting/opening the candidate itself -- that half is out of scope
    for this requirement, flagged as a candidate follow-up if the
    upstream gap ever closes.
    """
    req_id = "PATH-DIAG-007"


class VendoredIrohPendingOpenPathsDedupe(Requirement):
    """REQUIREMENT-ID: PATH-DIAG-008

    Root cause of the OOM/memory-burst investigation
    (`DO-NOT-COMMIT/oom-leak-investigation/`), found 2026-08-16 on aorus by
    a size-filtered `realloc` uprobe (six hits above 32 MB in 17 seconds,
    every one the same stack), traced into the vendored dependency, not
    guessed: `iroh-1.0.3/src/socket/remote_map/remote_state.rs`.

    **Mechanism.** `RemoteStateActor::open_path_on_conn` (line ~1046), on
    `PathError::RemoteCidsExhausted` / `PathError::MaxPathIdReached`,
    unconditionally pushes the failing address onto `State::
    pending_open_paths` (a plain `VecDeque<FourTuple>`, no dedup, no
    bound) -- once per connection that fails on that address. A 333ms
    timer then drains the whole queue and retries every popped address
    against *every* live connection to that remote peer
    (`open_path_on_all_conns`, line ~737), regardless of which connection
    originally queued it. So one address failing on C connections queues
    C identical copies; next tick, each of those C copies fans back out
    to C connections again. The queue multiplies by C every 333ms for as
    long as the failure condition holds -- geometric, not linear. C=1 is
    a fixed point; C=8 (this investigation's 8-member test coordinators)
    produced the observed 40 -> 80 -> 160 -> 320 MB doublings within
    single-digit seconds. Full mechanism and evidence:
    `DO-NOT-COMMIT/oom-leak-investigation/aorus-tracking/taskcensus/
    ROOTCAUSE_IrohPendingOpenPaths_2026-08-16.md`.

    **Fix, chosen over two alternatives considered** (push-once-per-address
    via a restructured call site; bounding the queue with a fixed cap --
    both recorded with reasons in `DO-NOT-COMMIT/oom-leak-investigation/
    PLAN_VendoredIrohDedupePatch_ChoicesSequenceReasons_2026-08-17.md`):
    a dedup guard immediately before the `push_back`, skipping the push
    if `open_addr` is already queued. Chosen because it is the smallest
    diff of the three (a single conditional at one call site, versus a
    signature change at the restructure option, versus a cap that
    saturates almost immediately under geometric growth and would still
    let the burst happen, just clipped, while silently starving whichever
    candidates get dropped past the cap) and because it loses no
    information: `open_path_on_all_conns` already retries every distinct
    candidate against every live connection unconditionally every tick,
    so the duplicate queue entries the dedup removes never carried
    distinct per-connection state to begin with.

    **Vendoring, same precedent as `LOG-005`/`PORTABILITY-001`:** iroh is
    not currently vendored (`Cargo.toml` pulls it straight from
    crates.io). Vendored at the exact `Cargo.lock`-resolved version
    (`vendor/iroh-1.0.3/`, `[patch.crates-io]` entry in `Cargo.toml`,
    `vendor/iroh-1.0.3/PATCH.md` documents the diff and upstream-report
    status). Not reported upstream to n0 yet at requirement-write time --
    tracked as follow-up, since the maintenance cost of carrying this
    patch across iroh's release cadence is strictly higher than the two
    existing vendored patches' cadence.

    **Explicitly out of scope for this requirement, sequenced
    separately:** the "reduce local candidate addresses" mitigation
    (binding the transport to a single address instead of `0.0.0.0`,
    `transport.rs:64`) was considered and dropped -- it costs
    multi-homing/NAT-traversal robustness for a probabilistic reduction in
    trigger frequency that this patch's structural fix makes unnecessary
    (reasons in the same `PLAN_VendoredIrohDedupePatch_...` doc). Tier A
    containment (`MemoryMax=`/`Restart=always`, `IDEAS_OOM_
    EmergencyMitigations_IfRootCauseNotFound_2026-08-16.md`) is not part
    of this requirement and must not land before this patch is verified
    on the deterministic repro harness with no memory cap active -- a cap
    present during verification would make "the patch held" and "the
    patch failed but the cap caught it" produce the same observable
    restart, destroying the test's own signal.

    **No tetron-owned code changes.** This requirement's only surface is
    the vendored dependency source and the `Cargo.toml` patch entry --
    there is no tetron-side function to unit-test (the same shape as
    `LOG-005`'s `noq-proto` demotion, as distinct from that same
    requirement's tetron-owned `reconnect_log_decision` half, which *did*
    get a `PURE-LOGIC-001` unit test). Verification is the live repro
    protocol recorded in the same plan doc: uncapped daemon, same
    deterministic 8-member/45s-churn harness and `scheduling open_path`
    trace-level counter and 5s burst watchdog that caught the original
    burst, run past the 25-62 minute onset window observed pre-patch
    across three coordinators, looking for a demonstrated plateau (queue
    length and RSS flat while the underlying CID-exhaustion trace event
    keeps firing) -- not merely "ran longer without dying."
    """

    req_id = "PATH-DIAG-008"


# --------------------------------------------------------------------------
# IPC-DECODE-ERR-001: reply (bounded) on an undecodable IPC request instead
# of silently dropping the connection
# --------------------------------------------------------------------------

class IpcDecodeErrorBoundedReply(Requirement):
    """REQUIREMENT-ID: IPC-DECODE-ERR-001

    Found 2026-08-23 during the periodic upstream-rayfish review
    (`DO-NOT-COMMIT/REVIEW_upstream-rayfish_2026-08-23.md`), then
    independently verified against tetron's own current source before
    scoping this requirement -- not ported on the strength of upstream's
    commit message alone.

    **The gap:** `handle_ipc_client` (`src/daemon/mesh/bootstrap.rs`)
    reads one request with `ipc::recv(&mut framed).await?` and propagates
    any error -- including a genuine decode failure (an `IpcMessage`
    variant this build does not know, or a malformed/corrupted frame) --
    straight out via `?`, before any reply is written. The accept loop's
    caller only logs it at `debug!` and drops the connection. Every
    in-tree IPC client (`tetron` CLI, `tetron-webui`, `tetron-systray`)
    goes through this same path, so a version-skew mismatch across the
    fleet currently degrades to an undiagnosable "connection closed"
    instead of naming what the daemon actually rejected.

    **The fix:** `handle_ipc_client` matches the `recv` error instead of
    `?`-ing it through. On failure it logs at `debug!` (so the event stays
    visible to `tetron report`/log review -- see the DoS note below for
    why this matters) and best-effort replies with `IpcMessage::Error {
    message }` carrying the decode-failure reason, then returns `Ok(())`
    without propagating further -- the send is allowed to fail silently,
    since the other common cause of a decode failure is a client that has
    already gone away.

    **The reply text is bounded to a fixed length (truncated on a UTF-8
    char boundary, with a truncation marker appended) -- this is not
    optional polish, it is the substance of this requirement.** tetron's
    IPC socket is `0666` by design (`set_socket_permissions`, `HARDEN-002`'s
    rejected-`HARDEN-001` note: authority is granted per-request via
    `SO_PEERCRED`, never by socket permissions, so any local user can
    connect), and `tetron-proto::ipc::MAX_FRAME_LEN` is 1 MiB. A decode
    error's `Display` (via `rmp_serde`) can echo back client-supplied
    content. Without a bound, any local user could send a ~1 MB malformed
    frame, never read the reply, and park a daemon task plus a file
    descriptor on a write that can never complete -- repeated, that
    exhausts the daemon's fd table. Upstream rayfish shipped exactly this
    unbounded reply first (`1001e80`) and had to patch the same day
    (`12272c2`) once review caught it; this requirement adopts the bounded
    shape from its first commit rather than repeating that two-step
    history.

    **Regression risk:** low. `handle_ipc_client` has no existing tests to
    break, and the change is additive -- a new failure-path match arm --
    leaving the existing decode-succeeds path (`daemon.handle_request` /
    `ipc::send(&mut framed, resp)`) untouched.
    """

    req_id = "IPC-DECODE-ERR-001"
