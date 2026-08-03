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
    """
    req_id = "PATHBLEED-STATUS-002"


# --------------------------------------------------------------------------
# PATH-DIAG-*: relay-vs-direct path observability (Level 1 instrumentation)
# --------------------------------------------------------------------------
#
# Draft for review -- not yet implemented, `reconcile.py` not yet run against
# these. Motivated by a live incident 2026-08-02 (Android tablet + several
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
    field. `PATH-DIAG-003` depends on this landing first (it needs a
    path-open timestamp source); `PATH-DIAG-002` and `PATH-DIAG-004` do not
    depend on this one.

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


