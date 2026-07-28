from libspec import Requirement, Constraint, UserStory

class HostnameDefaultsToMachineHostname(Requirement):
    """REQUIREMENT-ID: HOSTNAME-001

    When no `--hostname` is given (and no `default_hostname` is configured
    via `tetron up --hostname`), default to the machine's own OS hostname
    instead of a random noun (the old behavior). `hostname::generate_hostname`
    tries `machine_hostname()` (via `libc::gethostname`, sanitized) first,
    falling back to the random generator only if the OS hostname is
    unavailable or sanitizes down to nothing usable.

    A random hostname gave zero information about which machine a roster
    entry actually was, forcing cross-referencing `tetron status` by IP or
    connection order. The real hostname is immediately meaningful at the
    cost of exposing it to every peer on every network joined -- a
    conscious trade accepted for tetron's model (a private mesh you
    deliberately invite people to); `--hostname` still overrides this for
    anyone who'd rather not.

    `hostname::sanitize_hostname(raw) -> Option<String>`: keeps only the
    first label (OS hostnames are sometimes FQDN-ish, e.g. macOS's
    `MyLaptop.local`), lowercases ASCII letters/digits, collapses anything
    else (spaces, underscores, other punctuation) to a hyphen, trims
    leading/trailing hyphens, truncates to 63 characters (re-trimming a
    hyphen the truncation might land on), and returns `None` if nothing
    usable survives.

    Also loosened *explicit* `--hostname` handling at its three entry points
    (`create_join.rs`'s create and join paths, `runtime.rs`'s `activate`
    for `tetron up --hostname`): each now lowercases the input before
    validating, instead of hard-rejecting mixed case outright (e.g.
    `--hostname MyLaptop` previously errored; now accepted as `mylaptop`).
    `is_valid_hostname` itself is unchanged (still the strict char-class/
    length/hyphen-boundary predicate over an already-lowercased string) --
    other invalid characters (spaces, dots) are still a hard error for an
    explicit `--hostname`, only case is auto-corrected, since silently
    dropping characters from something a user typed on purpose seemed like
    the wrong default whereas case was the actual, specific complaint.

    The `is_valid_hostname` lowercase-only *restriction* itself was
    investigated (traced via `git log -p -- src/hostname.rs` to upstream
    commit `430f670`, "add Magic DNS with .pi domain resolution" -- avoiding
    DNS-label case-folding, a concern MINIMAL-012 removed entirely) but
    deliberately left in place: it's cheap to satisfy (lowercase on the way
    in, both for the machine-hostname path and the explicit-input path
    above) and a single canonical case avoids a second design question
    (would roster lookups like `kick`/`admin add` need to become
    case-insensitive to match a case-preserving hostname?) that has no
    upstream requirement it was resolving anyway.

    Found: 2026-07-17, logged as a TODO note earlier the same session,
    implemented same-day at the user's request as part of the CLI
    flags-and-defaults review.

    **Verified live, 2026-07-17,** on 3 bare-metal machines (aorus, xps,
    x10sra). aorus's real OS hostname (`590I-AORUS-ULTRA`, mixed case) came
    up as `590i-aorus-ultra` in `tetron status` with no `--hostname` passed;
    xps joined the same way and showed as `xps-17-9720` on both its own
    status and the coordinator's roster view, confirming the sanitized
    hostname round-trips correctly through the signed roster.
    """
    req_id = "HOSTNAME-001"


# --------------------------------------------------------------------------
# ADMIN-ADD-EASY-ID: tetron admin add should accept hostname or mesh IP
# --------------------------------------------------------------------------

class AdminAddAcceptsHostname(Requirement):
    """REQUIREMENT-ID: ADMIN-ADD-EASY-ID

    `tetron admin <NETWORK> add <IDENTITY>` must accept a hostname, mesh IP,
    or short id (first 10 hex chars of the endpoint_id). Previously it only
    accepted the raw short-id, which required looking up the peer's endpoint_id
    from `tetron status --json` and manually truncating to 10 characters --
    error-prone for humans.

    Use the same resolution logic as `tetron kick` (`resolve_peer_name`):
    match the argument as a hostname against the signed roster, then fall back
    to short-id prefix matching against endpoint IDs. This makes the admin-add
    workflow as easy as `tetron admin shallows add usbos-1`.

    Found: 2026-07-15, while writing the co-coordinator HOWTO section in
    README.md. The short-id-only requirement forced an awkward `--json` + manual
    truncation step for what should be a simple operation.

    **Correction, 2026-07-17:** this requirement's own text was wrong on two
    points, found during a CLI doc-comment-vs-handler audit. (1) "mesh IP" was
    never implemented -- `resolve_peer_name` only checks hostname, then falls
    back to short-id prefix matching; it never inspects an address. Dropped
    "mesh IP" from the `--help` text (`main.rs`) and the daemon's own error
    message (`admin.rs`), since it promised a capability that did not exist.
    (2) "Use the same resolution logic as `tetron kick`" was also wrong --
    `kick_member` was never changed to use `resolve_peer_name`; it resolves by
    short-id/endpoint-id prefix only (`resolve_short_id_any_network`),
    deliberately, because removing the wrong member needs a cryptographic
    identity, not a spoofable hostname. `resolve_peer_name`'s own rustdoc had
    drifted to claim it backs `kick` (leftover from an edit that moved
    `kick_member`'s real doc comment onto the wrong function) -- restored
    `kick_member`'s doc comment and rewrote `resolve_peer_name`'s to correctly
    name `admin_add` as its caller and state the principle: additive commands
    (`admin add`) may resolve friendlier identifiers; destructive commands
    (`kick`, `nuke --second`) require the short id. `AGENTS.md`'s CLI
    reference had the same "hostname is NOT accepted" error and was corrected
    to match.

    **Fix, 2026-07-17 (same day, follow-up):** `resolve_short_id_any_network`
    took a prefix of *any* length and returned the first member whose
    endpoint id started with it (`.find(...)`) -- no minimum length, and no
    check for more than one match. For `admin_add` this was a UX gap; for
    `kick_member` (the destructive caller) it was a real correctness bug: a
    short-enough or colliding prefix could silently resolve to the wrong
    peer with no warning the input was ambiguous. Fixed: now returns
    `Result<EndpointId, String>`, rejects any input under 10 characters (the
    length `tetron status` already displays, so copy-pasting from status
    always satisfies it) with a "too short" message, and collects all
    matches instead of stopping at the first -- more than one distinct match
    now errors as "ambiguous" naming every candidate's short id, rather than
    guessing. `resolve_peer_name` and its two callers (`admin_add`,
    `kick_member`) propagate the specific message instead of a generic one.
    A full (complete, untruncated) id was already inherently unambiguous
    before this fix and needed no change -- `starts_with` matches a string
    against itself trivially, and no two peers share a full endpoint id.

    Found an analogous, not-yet-fixed gap in the same function family:
    `resolve_peer_name`'s hostname match also returns the first cross-network
    hit with no ambiguity check (lower severity -- only backs the additive
    `admin_add`, not a destructive command). Logged in
    `DO-NOT-COMMIT/TODO.md` rather than fixed in this pass.
    """
    req_id = "ADMIN-ADD-EASY-ID"


# --------------------------------------------------------------------------
# CLI-VOCAB-001: unify the "which locally-known network" argument's name
# --------------------------------------------------------------------------

class LeaveArgumentRenamedToNetwork(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-001

    `tetron leave`'s positional was named `name` while `invite`/`admin` (and
    the same underlying lookup's IPC field) already used `network` -- three
    commands doing the identical `self.networks.get(string)` lookup with two
    different field names for no reason. Renamed `Leave`'s field to
    `network` end to end: `main.rs`'s `Command::Leave`, `cli/network.rs`'s
    `ipc_leave`, `tetron-proto`'s `IpcMessage::Leave`, `daemon/mod.rs`'s
    dispatch arm, and `daemon/mesh/runtime.rs`'s `leave_network` (signature,
    body, and its `#[tracing::instrument]` field). Pure rename -- the lookup
    mechanism itself (a plain map keyed by the mutable local network name)
    is unchanged.

    This is deliberately scoped to `leave` only, not the full rename
    described in `DO-NOT-COMMIT/TODO.md`'s "CLI-wide vocabulary/rename
    pass". `nuke` and `kick` also have this same `name`/`network`
    inconsistency (`nuke` still says `name`), but those two are slated to
    stop using the mutable-name lookup entirely in favor of a not-yet-built
    short-id resolution mechanism (mirroring `resolve_short_id_any_network`,
    fixed for peers in `ADMIN-ADD-EASY-ID`'s follow-up addendum). Renaming
    their field ahead of that mechanism would just relabel today's
    unresolved-by-cryptographic-identity lookup with a more-honest-sounding
    name it doesn't yet deserve -- the same class of doc-vs-behavior mismatch
    this session has otherwise been finding and fixing. `leave`, `invite`,
    and `admin` are not changing lookup mechanism, so unifying their field
    name has no such dependency and was safe to do now.

    Found: 2026-07-17, while auditing all five network-selecting commands
    (`leave`/`nuke`/`kick`/`invite`/`admin`) for consistency at the user's
    request.
    """
    req_id = "CLI-VOCAB-001"


class NukeKickResolveByNetworkShortId(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-002

    `nuke` and `kick` stop resolving "which network" through the mutable
    local display name (`self.networks.get(string)`, the same lookup
    `leave`/`invite`/`admin` still use) and instead require the network's
    own short id -- a prefix of its public key, matching the peer short-id
    convention (`fmt_short()`, 10 hex chars). This is the mechanism gap
    identified in `ADMIN-ADD-EASY-ID`'s follow-up addendum and
    `CLI-VOCAB-001`'s deferred scope: a local alias is user/coordinator-
    chosen, freely mutable, and can collide in meaning across networks --
    unfit as the sole identifier for a destructive, hard-to-undo action.
    There is deliberately no name/alias fallback: unlike peer resolution
    (where `admin add` may resolve a friendlier hostname because it's
    additive), both of these are destructive, so the short-id-only rule
    is absolute.

    New resolver: `MeshManager::resolve_network_short_id` (`daemon/mod.rs`,
    next to `resolve_short_id_any_network`, which it mirrors structurally).
    Rejects prefixes under 10 characters as too short, and rejects a prefix
    matching more than one joined network as ambiguous -- same discipline as
    the peer-side fix, applied to networks for the first time (previously
    there was no network-resolution-by-cryptographic-identity path at all,
    just the raw map lookup). Returns the resolved display name so
    `nuke_network`/`kick_member`'s existing bodies, which are keyed off that
    name throughout, need only a resolution step inserted at the top --
    shadowing the parameter -- rather than a rewrite.

    `tetron status` (`cli/status.rs`) now prints each network's short id
    unconditionally (a new `id <short>` line, computed once per
    `print_network` call and reused for both that line and the nuke-proposal
    hint below) -- without this the feature has nothing to copy from.
    Fixed two now-broken "run this command" hints that echoed the local
    name back at the user: `nuke_network`'s own "have another coordinator
    run `tetron nuke {name}`" message, and `status.rs`'s nuke-proposal
    hint -- both now embed the short id instead, since the alias no longer
    works as an argument to `nuke`.

    `main.rs`'s `--help` text for `Nuke.name`/`Kick.network` corrected from
    "Three-word network name"/"Network name" to explicitly say short id, not
    local name -- leaving the old text would have been a doc-vs-behavior
    lie, the same class of bug this session has spent most of its effort
    finding elsewhere. The field *names* (`name`, `network`) are
    deliberately left untouched -- renaming them is scoped to a later,
    separate pass covering all five commands' `--flags` together, per the
    user's explicit sequencing (internal mechanism first, user-facing
    labels last).

    **Verified live, 2026-07-17,** on 3 bare-metal machines (aorus, xps,
    x10sra): `nuke`/`kick` given the old alias both correctly errored
    `could not resolve network '<alias>'` instead of falling back to the
    name lookup; given the short id both resolved correctly (`nuke
    --cancel` reached the real solo-coordinator rejection; `kick` actually
    removed xps from the roster, confirmed on both sides and via
    CONVERGE-003's self-removal on the kicked node). A prefix under 10
    characters was correctly rejected as too short on both commands. Final
    `tetron nuke <short-id> --force` cleanly destroyed the test network; TUN
    device count stayed at 1 on all three machines throughout.
    """
    req_id = "CLI-VOCAB-002"


class CliFlagsVocabularyPass(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-003

    Executes the `--flags`/positionals half of the CLI vocabulary cleanup
    (`DO-NOT-COMMIT/TODO.md`'s "CLI-wide vocabulary/rename pass"), deferred
    behind the internal-mechanism work (`ADMIN-ADD-EASY-ID`'s follow-up,
    `CLI-VOCAB-001`, `CLI-VOCAB-002`) per the user's explicit sequencing.
    The original proposed table (written before `CLI-VOCAB-001`/`002`
    shipped) was stale in two ways, reconciled here rather than followed
    literally:

    1. It proposed renaming `Leave`'s field to `alias` -- but `Leave` was
       already renamed to `network` by `CLI-VOCAB-001`, and the user's
       later, stronger "I do not like the alias in the first place"
       objection ruled the word out entirely as a lookup-selector name.
    2. It proposed renaming `Nuke`/`Kick`'s network argument to `alias` --
       flatly wrong once `CLI-VOCAB-002` moved both off name-based lookup
       entirely onto short-id resolution; `alias` would have described a
       mechanism those commands no longer use.

    Resolved table (implemented):
    - `Create.name` -> `network_name` (`--name` -> `--network-name`)
    - `Join.name` -> `alias` (`--name` -> `--alias`)
    - `Join`'s positional `network_key` -> `invite_code` (CLI-facing only --
      see the wire-field correction below)
    - `Nuke.name` -> `net_id`, `Kick.network` -> `net_id` (matches the
      short-id mechanism `CLI-VOCAB-002` gave them; `Kick.peer` unchanged)
    - `AdminAction::Add.identity` -> `peer` (matches `Kick.peer` -- same
      concept, `peer` already wins internally: `PeerTable`, `peers.rs`)
    - `Leave`/`Invite`/`Admin`'s `network` field: **no change** -- already
      consistent with each other (`Leave` via `CLI-VOCAB-001`; `Invite`/
      `Admin` were already `network`), which resolves the open "no alias
      anywhere" question for free rather than requiring a rename.

    **Caught and reverted before landing:** `IpcMessage::Join`'s *wire*
    field is not the same thing as `main.rs`'s CLI-facing positional. The
    CLI positional genuinely is raw invite-code text (correctly renamed to
    `invite_code`), but `cli/network.rs`'s `ipc_join` decodes it locally
    (`invite::decode_invite_code`) before ever sending anything over IPC --
    the wire field always carries the resolved network *public key* (with
    the secret riding separately in `invite`), in both the decode-success
    and bare-room-id-fallback branches. Renaming the wire field to
    `invite_code` would have been factually wrong; it stays `network_key`,
    documented with a comment explaining why it looks like a mismatch with
    the CLI layer but isn't.

    **Scope addition beyond the literal table, for internal consistency:**
    `IpcMessage::Created`/`Joined`'s response field (`name` in both) was
    also renamed to `network`, matching what `Leave`/`Invite`/`Admin`
    settled on -- these responses echo back the same "resolved local
    display name" concept, and leaving them as `name` while the identical
    concept elsewhere says `network` would have introduced a new
    inconsistency instead of removing one.

    **Also propagated to internal parameter names** at each renamed field's
    boundary function (`create_network`, `join_network`, `nuke_network`,
    `kick_member`, `admin_add`) so the rename doesn't stop at the wire
    struct. `join_network_inner` already used `alias` internally (predating
    this pass) -- confirms the outer boundary was the actual inconsistency,
    not the deep internals. `create_network_inner`'s `custom_name` and the
    resolved-identity locals in `nuke_network`/`kick_member`/`admin_add`
    were deliberately left as-is -- already clear, not part of the
    user-facing surface this pass targets.

    **Found and fixed as side effects while touching `AGENTS.md`:** the CLI
    reference block still listed `tetron requests`/`accept`/`deny` --
    commands removed by `LIVE-001` (confirmed absent from `main.rs`'s
    `Command` enum), sitting two lines above the very paragraph documenting
    their removal. Removed. Also flagged (not fixed, logged to
    `DO-NOT-COMMIT/TODO.md`): `NetworkStatus.pending_requests` is
    vestigial -- still on the wire, still populated, but hardcoded to `0` at
    both construction sites in `diagnostics.rs` since `LIVE-001` removed
    the queue it used to reflect, and never read by any CLI display code.

    Verified via real `--help` output on all five renamed commands (not
    just build success) before considering this done.
    """
    req_id = "CLI-VOCAB-003"


class CliNetworkKeyVocabularyFollowup(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-005

    Further rename of `CLI-VOCAB-002`/`003`'s `net_id` -> `network_key`,
    prompted by a concrete discoverability gap those two passes didn't
    catch: `net_id` (the `Nuke`/`Kick` positional, shown as `<NET_ID>` in
    `--help`) and `tetron status`'s text-output label (`id`) were both
    spelled differently from `tetron status --json`'s field for the exact
    same underlying value, `network_key`. A user's actual path to this
    value is `tetron status --json | jq`, not reading prose -- grepping
    that JSON for `net_id` or `id` finds nothing, only `network_key`. Fixed
    by standardizing on `network_key` for every human-facing spelling
    (`--help`, status text label, docs) and propagating it back into the
    wire field too, rather than leaving the JSON field as the odd one out.

    Changed:
    - `IpcMessage::Nuke`/`Kick` (`tetron-proto/src/ipc.rs`): field `net_id`
      -> `network_key`.
    - `main.rs`'s `Command::Nuke`/`Kick` clap positional: `net_id` ->
      `network_key` (so `--help` now shows `<NETWORK_KEY>`), plus the
      `main()` dispatch match arms.
    - `cli/network.rs`'s `ipc_nuke`/`ipc_kick` parameters.
    - `daemon/mod.rs`'s `IpcMessage::Nuke`/`Kick` dispatch match arms, and
      `resolve_network_short_id`'s error string (now names `network_key`
      instead of "the short id" when a prefix is too short).
    - `daemon/mesh/runtime.rs`'s `nuke_network`/`kick_member` parameters
      and the `#[tracing::instrument]` field on `nuke_network`.
      `resolve_network_short_id`'s own parameter (`short`) is untouched --
      it is a generic resolver, not part of this user-facing vocabulary.
    - `cli/status.rs`: the text-output line changes from `id <short>` to
      `network_key <short>`, matching `--json`'s field name exactly.
    - Docs (`AGENTS.md`, `README.md`, `docs/HOWTO.md`, `docs/PROPOSAL.md`):
      `<net-id>`/`<net-id-from-status>` placeholders and "the `id` line"
      references updated to `<network-key>`/`<network-key-from-status>`/
      "the `network_key` line", each noting the value may be an
      unambiguous >=10-char prefix rather than the full key.

    **Deliberately unchanged:** `IpcMessage::Join`'s existing `network_key`
    field (always the full public key, decoded client-side from the
    invite code) and this rename's `Nuke`/`Kick` `network_key` (accepts a
    prefix) now share a field name for the same underlying concept --
    intentional, not a naming collision to resolve. `resolve_network_short_id`'s
    behavior (>=10-char prefix, ambiguous-prefix rejection, no name/alias
    fallback) is unchanged; only the names pointing at it moved.

    **Same-session follow-up: `Kick`'s second positional, `peer` ->
    `endpoint_id`.** The same mismatch existed one level deeper: `Kick.peer`
    (shown as `<PEER>` in `--help`) is resolved exclusively by
    `MeshManager::resolve_short_id_any_network`, which matches only against
    a member's endpoint id -- it never accepts a hostname, unlike
    `AdminAction::Add.peer`, which deliberately resolves hostname-first
    (`resolve_peer_name`). `tetron status --json`'s `PeerStatus` struct
    carries both `endpoint_id` and `hostname` fields side by side, so
    without this fix a user would have no way to tell, from the JSON alone,
    which of the two `kick` actually wants -- and guessing `hostname` (the
    more human-looking field) would silently fail to resolve. Renamed
    `IpcMessage::Kick.peer` -> `endpoint_id` (wire), `Command::Kick.peer` ->
    `endpoint_id` (`main.rs`, so `--help` shows `<ENDPOINT_ID>`),
    `cli/network.rs`'s `ipc_kick` parameter, `daemon/mod.rs`'s dispatch
    match arm, and `kick_member`'s parameter in `daemon/mesh/runtime.rs`
    (plus the two internal uses of it in that function). `AdminAction::Add`'s
    `peer` field is deliberately left alone -- it genuinely accepts a
    hostname, so `peer` still describes it accurately.

    **Cross-repo consequence, not part of this change's own scope** (same
    class of risk as `CLI-VOCAB-004`'s `Up`/`Down` wire rename): any code in
    `tetron-webui`/`tetron-systray` constructing `IpcMessage::Kick { peer,
    .. }` directly will fail to compile against this crate until updated
    there.
    """
    req_id = "CLI-VOCAB-005"


# --------------------------------------------------------------------------
# CLI-VOCAB-004: up/down renamed to resume/standby; resume's escalation removed
# --------------------------------------------------------------------------

class ResumeStandbyRename(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-004

    `tetron up`/`tetron down` renamed to `tetron resume`/`tetron standby`,
    full depth (CLI and wire protocol both), fixing two problems in the
    inherited-from-upstream naming (full analysis in
    `DO-NOT-COMMIT/DECISION_tetron_UpDown_Naming_And_Behavior.md`, not
    reproduced here):

    1. The state `down` produced was never itself called "down" anywhere
       in the product -- `tetron status` and every daemon log/message
       already said "standby" (`·standby·` marker, "on standby (still
       connected to peers)"). The verb and the resulting state's name
       didn't match. Renaming the verb to `standby` makes it match the
       noun that was already in universal use.
    2. `up` silently escalated scope on hidden state (`src/cli/
       service.rs`'s old `cmd_up`): with a daemon reachable it was the
       narrow mirror of `down` (just activate data plane); with none
       reachable it silently did everything `install` does (write the
       systemd unit/launchd plist, `systemctl enable && restart`, wait
       for the daemon, grant operator) *before* activating -- an
       undocumented, asymmetric side door into full installation. `resume`
       removes this escalation entirely rather than renaming it: it is
       always exactly one operation, matching `standby`'s existing
       single-meaning behavior. With no daemon reachable, `resume` now
       errors the same way regardless of caller privilege (collapses the
       old root/non-root branch into one message): "tetron service is not
       running. Install and start it with: sudo tetron install." No new
       verb is needed for the bootstrap case -- `cmd_install` already
       calls the exact same `install_and_start_service()` the old
       escalation fallback called, verified identical before this was
       written.

    Renamed, full depth:
    - `tetron-proto::IpcMessage::Up { hostname, network }` ->
      `Resume { hostname, network }`; `Down { network }` -> `Standby
      { network }`. Wire-level, not just CLI text -- any client
      (`tetron-webui` included) constructing these variants directly
      needs updating in lockstep.
    - `src/main.rs`'s `Command::Up`/`Down` -> `Command::Resume`/
      `Command::Standby`, same fields, same `--hostname`/`--network`
      flags.
    - `src/cli/service.rs`'s `cmd_up` -> `cmd_resume` (escalation removed
      per point 2 above); `src/cli/status.rs`'s `ipc_down` -> `ipc_standby`.
    - `src/daemon/mod.rs`'s IPC dispatch match arms follow the wire rename;
      `activate()`/`deactivate()` themselves are unchanged (they already
      only ever meant "the data plane," never named after the old verbs).

    **State label decoupled from the command verb for the active side.**
    `src/cli/status.rs`'s daemon-wide summary line (`let state = if active
    { "up" } else { "standby" }`) does not become `"resume"` -- "resume" is
    a verb, not a state adjective ("the service is in resume" reads wrong;
    "the service is active" reads right). It becomes `"active"` instead,
    which is not new vocabulary: it already matches the internal `active:
    bool` field used throughout (`net.active`, the JSON output's `"active":
    active`). The `standby` side needed no equivalent split -- "standby"
    already worked as both verb and state-noun before this change, so the
    per-network `·standby·` marker and daemon log text are unchanged.

    **Hard cutover, no soft-deprecation** -- matches the `CLI-VOCAB-003`
    precedent (last CLI vocabulary rename pass, also a hard cutover with no
    aliases kept). No hidden `up`/`down` compatibility aliases; a
    `CHANGELOG.md` entry is the only transition aid. `start`/`stop` are
    explicitly out of scope and unchanged -- they already mirror `systemctl
    start`/`stop` directly and were confirmed not to have either problem
    above.

    **Cross-repo consequence, not part of this change's own scope:**
    `tetron-webui` calls `IpcMessage::Up`/`Down` directly (`src/api.rs`) and
    routes `POST /api/up`/`/api/down`; this wire rename breaks its build
    against `tetron-proto`'s `main` until it is updated separately, in that
    repo, immediately after this ships (per the DECISION doc's sequencing:
    tetron ships first, tetron-webui fixed right after since it's a hard
    compile failure not a backlog item, tetron-systray scaffolded fresh
    against `Resume`/`Standby` only).
    """
    req_id = "CLI-VOCAB-004"


# --------------------------------------------------------------------------
# ADMIN-RECONNECT-CTRL: admin-grant must work after coordinator reconnect
# --------------------------------------------------------------------------

class AdminGrantRespawnsControlListener(Requirement):
    """REQUIREMENT-ID: ADMIN-RECONNECT-CTRL

    When a member's coordinator connection drops and the reconnect loop
    re-establishes it, a new control-listener task must be spawned on the new
    connection. Previously the control listener was only spawned once during
    initial join (attached to the initial connection). When that connection
    dropped the listener died, and the reconnect loop only respawned a forward
    reader -- never a control listener. As a result, any `AdminGrant` sent by
    the coordinator after a reconnect was silently lost, making co-coordinator
    promotion impossible after the coordinator had restarted.

    The fix: pass daemon-wide resources (promote_tx, pending_pongs) and
    per-network state (live_state, reconverge_notify) to the reconnect loop.
    On a successful reconnect, spawn a fresh `spawn_member_control_listener`
    on the new connection alongside the forward reader. The per-network state
    is delivered via oneshot channels because it does not exist when the
    reconnect loop is spawned (it is created inside `join_mesh_shared`, which
    runs after the reconnect loop starts but before any disconnect can occur).

    Found: 2026-07-15, while testing co-coordinator promotion on network
    "shallows". AORUS granted the network key to USB-OS via `tetron admin
    shallows add usbos-1`, which succeeded. USB-OS never received the grant
    because its daemon had reconnected after an earlier restart of AORUS,
    and no control listener was running on the new connection.
    """
    req_id = "ADMIN-RECONNECT-CTRL"


# --------------------------------------------------------------------------
# STATUS-001: expose each network's OS TUN interface name in `tetron status`
# --------------------------------------------------------------------------

class StatusShowsTunInterfaceName(Requirement):
    """REQUIREMENT-ID: STATUS-001

    Found 2026-07-18 auditing the CLI/IPC command surface now that a node
    can belong to several real, isolated networks (multi-segment TUN,
    `MULTISEG-002..007`): `NetworkHandle.tun_name` has existed in the
    daemon since that work landed, but was never put on the `NetworkStatus`
    wire type or printed by `tetron status`. With one network this never
    mattered; with several, there was no way to know which OS interface
    (`tun0`, `tun1`, ...) belongs to which network without guessing from
    `ip link show` order or grepping daemon logs — and that matters for
    writing host-firewall rules per network (see `STATUS-001`'s companion
    docs fix for the previously-fictional `iifname "tetron"` example).

    **Fix:** `tetron-proto::ipc::NetworkStatus` gained a `tun_name: String`
    field (`#[serde(default)]` so an older daemon's response — one built
    before this field existed — still decodes against a newer CLI, and a
    stored/replayed old test fixture still deserializes). `diagnostics.rs`'s
    `network_status()` populates it from `h.tun_name.lock().unwrap().clone()`
    at both construction sites (the normal path and the state-lock-poisoned
    fallback). `tetron status`'s text renderer (`cli/status.rs::print_network`)
    prints it as an `interface <name>` line alongside the existing `id`
    line, suppressed while the value is still the pre-attach placeholder
    (`"pending"`) or empty. `--json` gets it for free since `networks` in
    the JSON status output is `NetworkStatus` serialized directly.

    Found: 2026-07-18, same audit pass as the other "Multi-network
    command-surface follow-ups" items logged in `DO-NOT-COMMIT/TODO.md`.

    **Addendum, 2026-07-18 — companion docs fix**: `AGENTS.md`'s
    `MINIMAL-010` note and `docs/HOWTO.md`'s port-restriction example both
    showed `nft add rule inet filter input iifname "tetron" ...` — but
    `tun::create()` never calls `.name(...)` on the `tun` crate's
    `Configuration`, so the OS always auto-assigns `tun0`/`tun1`/etc. This
    predates multi-segment TUN entirely (the single old shared device was
    never actually named `tetron` either); with N networks there are now N
    auto-named interfaces and no fixed name to reference even in
    principle. Both docs now show a real `tun0` example and point at
    `tetron status`'s new `interface` line (this requirement) or
    `ip link show` for finding the right interface per network, instead of
    a name that was always fictional.
    """
    req_id = "STATUS-001"


# --------------------------------------------------------------------------
# ADMIN-ADD-NETWORK-SCOPE: resolve_peer_name scoped to the target network
# --------------------------------------------------------------------------

class AdminAddResolvePeerNameNetworkScoped(Requirement):
    """REQUIREMENT-ID: ADMIN-ADD-NETWORK-SCOPE

    Re-examined 2026-07-18 while auditing the CLI/IPC command surface for
    multi-segment TUN: `resolve_peer_name(name: &str)` (`daemon/mesh/
    runtime.rs`) searched *every* joined network's roster for a hostname
    match and returned the first hit — it had no `network` parameter at
    all, even though its only caller, `admin_add(network: &str, peer_str:
    &str)` (`daemon/mesh/admin.rs`), already has the target network in
    scope and never passed it through. Hostnames are only guaranteed
    unique *within* one network's roster (`resolve_collision` at
    admission), so with two joined networks each having an `alice`,
    `tetron admin <net-A> add alice` could resolve to network-B's `alice`
    instead of network-A's.

    **Not a silent-wrong-grant security bug**: `admin_add` looks up the
    resolved identity in `network`'s *own* `PeerTable`
    (`h.peers.peers_for_network_with_conn(network)`, MULTISEG-002's
    per-network table) before sending the `AdminGrant`, and errors with
    "could not find an active connection to `<identity>` on `<network>`"
    if that identity isn't actually connected there. A cross-network
    mis-resolution fails closed, not silently — but it is a real
    usability bug: if network-A's real, currently-connected `alice`
    exists, but `resolve_peer_name` happened to hit network-B's `alice`
    first (DashMap iteration order), the command failed with a confusing
    "could not find an active connection" error even though the intended
    target was right there and reachable, with no indication the wrong
    identity was resolved behind the scenes.

    Same root category as the short-id prefix-collision bug fixed
    2026-07-17 in `resolve_short_id_any_network` (that one now rejects
    ambiguous/too-short matches instead of guessing — see
    `ADMIN-ADD-EASY-ID`'s addendum). This fix is smaller: no separate
    "collect all matches, error on >1" step is needed the way the
    short-id fix needed one, because scoping the search to one network's
    roster makes cross-network ambiguity structurally impossible rather
    than something to detect after the fact.

    **Fix:** `resolve_peer_name` now takes `network: &str` and looks up
    the hostname match only in that network's own roster
    (`self.networks.get(network)`), instead of iterating `self.networks`.
    `admin_add`'s call site now passes its own `network` parameter
    through — the CLI already requires `tetron admin <network> add
    <peer>`, so the value was always available, just unused for this
    lookup. The short-id fallback (`resolve_short_id_any_network`) stays
    cross-network and unchanged: it already rejects ambiguous/too-short
    prefixes rather than guessing, so it was never the unsafe half of
    this function.

    Found: 2026-07-16 (original, less precise write-up). Root cause
    re-examined and narrowed 2026-07-18. Fixed: 2026-07-18.
    """
    req_id = "ADMIN-ADD-NETWORK-SCOPE"


# --------------------------------------------------------------------------
# STANDBY-PER-NETWORK: per-network data-plane standby via --network
# --------------------------------------------------------------------------

class UpDownAcceptOptionalNetworkScope(Requirement):
    """REQUIREMENT-ID: STANDBY-PER-NETWORK

    Found 2026-07-18 auditing the CLI/IPC command surface for
    multi-segment TUN: `tetron up`/`tetron down` (`activate()`/
    `deactivate()`, `daemon/mesh/runtime.rs`) were daemon-wide — one
    `MeshManager.active: Arc<AtomicBool>`, every loop over every joined
    network unconditionally. There was no way to take e.g. a "work"
    network's TUN offline at end of day while keeping "home" active, the
    way you'd physically unplug one of two NICs — a real gap once
    multi-segment TUN (`MULTISEG-002..007`) made "several genuinely
    isolated networks on one node" a normal, live configuration rather
    than a theoretical one.

    **Design (the "not yet scoped" gap this requirement closes):**
    `MeshManager.active` is a single flag, but the actual per-packet data
    gate needed to move to be per-network for `--network` to mean
    anything — the daemon-wide flag alone can't represent "net-a is up,
    net-b is on standby" at the same time. `NetworkHandle` gained its own
    `active: Arc<AtomicBool>`, and `forward::spawn_tun_writer` (the
    function that actually gates whether a received packet gets written
    to a TUN device) is now handed each network's own flag
    (`handle.active.clone()`, `attach_tun`) instead of the daemon-wide
    one. `MeshManager.active` survives, repurposed: it now only seeds a
    brand-new network's initial state at create/join/restore time
    (`create_and_attach_network_tun`'s existing "if the VPN is already
    active, bring this new network straight up" check) and is what an
    *unscoped* `activate()`/`deactivate()` call sets across the board —
    an unscoped `tetron up`/`down` is unchanged in effect (every network
    moves together) even though the mechanism underneath is now N
    independent per-network flags rather than one shared flag every
    writer read.

    **`activate`/`deactivate` signatures** gained `network: Option<&str>`.
    `Some(name)` restricts the loop to that one network (erroring if the
    name isn't a currently-joined network, rather than silently
    activating nothing) and uses that network's own `handle.active.swap`
    for idempotency (skip work if already in the target state) instead of
    the old single daemon-wide swap-guard. `None` preserves the original
    behavior exactly: it still flips `MeshManager.active` (for future new
    networks) and iterates every joined network. Both `IpcMessage::Up`
    and `IpcMessage::Down` gained a `#[serde(default)] network:
    Option<String>` field (defaulting to `None` so an old client's
    request still decodes as daemon-wide); `Command::Up`/`Command::Down`
    gained a matching `--network <name>` clap flag (network's local
    display name, as shown by `tetron status`).

    **Status visibility:** `NetworkStatus` gained `active: bool`
    (`#[serde(default)]` for wire back-compat), populated from each
    network's own `handle.active`. `tetron status`'s per-network line
    prints a `·standby·` marker when it's `false`. The top-level
    `StatusResponse.active` (the "up"/"standby" banner) changed from
    mirroring the single daemon-wide flag directly to "is at least one
    network's data plane up" (`statuses.iter().any(|s| s.active)`) — the
    banner's pre-existing meaning ("up" unless everything is on standby)
    is preserved without a wire-format change to that field, now just
    computed instead of stored.

    **Persistence, deliberately unchanged:** per-network standby state is
    not persisted to config, matching the pre-existing daemon-wide
    behavior — `run_daemon` always calls `activate(None, None)`
    unconditionally at boot (before this requirement and after), so a
    daemon restart already brought the whole VPN back up regardless of
    any prior `tetron down`; per-network standby inherits that same
    "doesn't survive a restart" property rather than introducing new
    persisted state.

    **Internal call sites updated:** `run_daemon`'s boot-time
    `activate(None)` → `activate(None, None)`; the shutdown handler's
    `deactivate()` → `deactivate(None)` (`bootstrap.rs`). Every
    `NetworkHandle` construction site (create/join/restore, plus the
    bare-bones test fixture in `attach_tun_is_self_healing_on_reattach_
    and_double_attach`) gained `active: Arc::new(AtomicBool::new(false))`
    — a freshly constructed handle always starts inactive;
    `create_and_attach_network_tun` is the one place that decides whether
    to immediately flip it, based on the daemon's current default state.

    **Live-tested:** not yet on real multi-machine hardware (same caveat
    as `STRANDED-COORDINATOR-WARN`, found in the same audit pass) —
    verified via a new unit test (`activate_deactivate_scope_to_one_
    network_when_given`, `daemon/mod.rs`'s `headless_tests`) that inserts
    two bare-bones networks and exercises scoped activate/scoped
    deactivate/unscoped activate/unscoped deactivate/unknown-network-name
    against real `activate()`/`deactivate()`, asserting exactly which
    network's `active` flag moved at each step. The real OS calls
    (`tun::set_link_up`/`route_peer_range`) fail against the test
    fixture's placeholder device name, which is expected and harmless —
    they're non-fatal everywhere in this codebase (logged as warnings,
    never propagated as an error), so the test's assertions are about
    the flag-scoping logic itself, not real TUN state. `reconcile.py`
    green (build/clippy/test, 216 tests — 215 prior + this one).

    Found: 2026-07-18, same audit pass as `STATUS-001`,
    `ADMIN-ADD-NETWORK-SCOPE`, and `STRANDED-COORDINATOR-WARN`. Fixed:
    2026-07-18.

    **Addendum, 2026-07-18 — live-tested on 4 real machines (3 bare-metal
    Linux: 590i-aorus-ultra/xps-17-9720/x10sra, plus an M1 MacBook Pro
    over macOS).** aorus coordinated two networks on distinct subnets
    simultaneously (`standby-a`, xps as the other member; `standby-b`,
    subnet `10.66.0.0/24`, the Mac as the other member) — `tetron
    status` showed two distinct interfaces (`tun0`/`tun1`) as expected
    (`STATUS-001`). `tetron down --network standby-a` on aorus dropped
    ping to xps to 100% loss while `standby-b`'s ping to the Mac stayed
    at 0% loss throughout, confirming real isolation, not just a status
    flag; `tetron up --network standby-a` recovered it to 0% loss.
    Separately, the Mac ran `tetron down --network standby-b` /
    `tetron up --network standby-b` **on its own side**, specifically to
    exercise the platform-specific route code (`route_peer_range`,
    `set_link_up`/`set_link_down`) `MACOS-001`/`MACOS-002` lived in —
    confirmed the route disappeared from macOS's own routing table on
    down (100% ping loss from aorus) and reappeared cleanly on up (0%
    loss). Finally, unscoped `tetron down`/`tetron up` (no `--network`)
    on aorus was confirmed to still move both networks together,
    matching the pre-existing daemon-wide behavior exactly. No bugs
    found. `reconcile.py` remained the gate for build/clippy/test
    throughout; this run is the real-hardware confirmation the spec
    entry above flagged as outstanding.
    """
    req_id = "STANDBY-PER-NETWORK"


class InstallOutputNamesConcreteAction(Requirement):
    """REQUIREMENT-ID: INSTALL-OUTPUT-001

    `sudo tetron install` used to run entirely silently until "waiting
    for daemon…" -- `ensure_service_installed` wrote the systemd unit /
    launchd plist with no output, and `install_and_start_service` ran
    `systemctl enable/restart` or `launchctl load` via `run_cmd`, which
    itself only ever prints on failure. So a user watching the command
    run saw nothing about what was actually happening on their machine
    (a privileged install writing a system service file and enabling
    it) until it was already done. Flagged live-testing macOS
    (2026-07-19): don't hide privileged/system-level actions just
    because the command that triggers them is short -- the command
    being short is not a reason for its output to be vague about what
    actually happened.

    Fix: `ensure_service_installed` (`src/cli/service.rs`) now prints
    the concrete unit/job name and the exact path being written before
    writing it -- `installing systemd service 'tetron' -> /etc/systemd/
    system/tetron.service` on Linux, `installing launchd job
    'com.tetron.vpn' -> /Library/LaunchDaemons/com.tetron.vpn.plist` on
    macOS. `install_and_start_service` similarly announces the
    enable/restart or load step before running it. Both functions have
    exactly one caller each (`cmd_install`, i.e. `sudo tetron install`),
    so this adds no noise to any other command path (`restart` uses its
    own `restart_service_and_wait`, which was already explicit about
    "restarted").
    """
    req_id = "INSTALL-OUTPUT-001"


class StatusOutputRedesign(Requirement):
    """REQUIREMENT-ID: STATUS-002

    `tetron status` is the primary information surface end users have, and
    Erik flagged it as difficult to read and ambiguous: unlabeled fields
    inconsistent with the labeled ones next to them, a bare `id` line with
    no indication of what it identified, and a `join <64-char-hash>` line
    duplicating that same value in full under a stale, actively misleading
    label (a bare room id/public key was never sufficient to join even
    before `LIVE-001`, and is explicitly discovery-only after it).

    Redesigned through iterative mockups in the (gitignored, not shipped)
    `DO-NOT-COMMIT/MOCKUP_tetron_status_output_redesign.md`, landing on:

    - **Daemon header**: `tetron v<version>  state <active|standby>
      endpoint <short>`, plus a `traffic` line (`bytes_tx`/`bytes_rx`,
      previously computed and sent over IPC but discarded by the text
      renderer -- `let _ = (packets_rx, packets_tx, bytes_rx, bytes_tx);`).
      `packets_rx`/`packets_tx` remain unused in text mode, still available
      via `--json`.
    - **Per-network header**: `network <name>   subnet <cidr>   admins
      <online>/<total>   members <online>/<total>   interface <tun_name>`.
      `subnet` is a new `NetworkStatus` field (CIDR string, formatted
      daemon-side from `membership::Subnet`'s bare `(Ipv4Addr, u8)` tuple,
      which has no serde/Display impl of its own) -- previously not
      exposed anywhere, despite subnet collision being an
      explicitly named troubleshooting category in this project
      (`SUBNET_COLLISION.md`, `SUBNET-BUG-001`). `admins online/total`
      needs no new wire field beyond the `is_coordinator` addition below --
      computed client-side in `status.rs` from `net.role.is_coordinator()`
      (self) plus each peer's `is_coordinator` + `connection.is_some()`.
    - **`network_key`**: kept, but truncated to a short prefix (~10 chars,
      matching `resolve_network_short_id`'s own `>=10`-char minimum -- both
      `nuke`/`kick` already accept a prefix, nothing lost), and shown only
      when the viewer's own role for that network is admin/coordinator. A
      plain member can't act on it regardless (`nuke`/`kick` would reject
      them independent of whether they know the value), so showing it to
      them was pure clutter. The NUKE-CONSENSUS pending-proposal hint's
      actionable `tetron nuke <key> ...` suggestion is likewise only
      included when the viewer has that value; a non-admin still sees a
      proposal exists, just without a command they couldn't use anyway.
    - **Peer table**: real column-aligned `role / host / ip / via`, the
      local node included as its own first row (`via` = `(you)`), rendered
      by a new `render_aligned_table` helper (`src/cli/status.rs`) that
      computes real per-column max width across all rows including the
      header -- the pre-existing `table()` helper explicitly does *not* do
      this ("No column alignment in plain mode"), so a new helper was
      needed rather than reusing it. `role` is `admin`/`member`, driven by
      a new `PeerStatus.is_coordinator: bool` field (the data already
      existed internally on `membership::Member`, just never threaded onto
      the wire type). `via` is `direct`/`relay`/`tor`/`offline`/`(you)` --
      covers every `ConnType` plus self plus disconnected, decided
      sufficient with no further states needed.
    - **Deliberately dropped from the default text view**: per-peer IPv6
      (own and peers'), and per-peer connection health (rtt/tx/rx byte
      counts). Both remain fully available via `--json`. IPv6 in
      particular was a real, discussed tension -- dual-stack is a shipped,
      deliberate feature (`IPV6-001..003`), and never showing it anywhere
      risks the feature becoming invisible by default permanently, not
      just hidden from casual users. A middle option (show only the
      viewer's own IPv6 once, since that's a single line regardless of
      peer count, while still dropping *peer* IPv6 from the table where
      the real per-row width cost lives) was raised and rejected in favor
      of the simpler full drop -- Erik's call, made knowingly rather than
      by default.
    - **`coordinator` -> `admin`, display string only.** `tetron admin
      <net> add/list` already used "admin" as the CLI command name for
      this exact concept (granting/listing the network key) while
      `tetron status`, error messages, and docs called it "coordinator" --
      an existing internal inconsistency, not a new term being
      introduced. Scoped narrowly: only `NetworkRole`'s `derive_more::
      Display` output (`#[display("coordinator")]` -> `#[display("admin")]`
      on the `Coordinator` variant) changed. The variant name itself,
      `is_coordinator`, `coordinator_count()`, and every spec requirement
      ID/prose referencing "coordinator" (`NUKE-CONSENSUS`,
      `STRANDED-COORDINATOR-WARN`, etc.) are unchanged -- same decoupling
      already used successfully this session for `resolve_network_short_id`'s
      internal `short` parameter staying put while user-facing labels moved.

    **Bundled in the same implementation pass**: `StatusResponse.
    pending_networks` removed as dead code (found while surveying
    available-but-unshown fields for this redesign, unrelated to it
    otherwise) -- its own doc comment claimed to reflect `AppConfig.
    pending_joins`, which `LIVE-001` removed entirely; the one
    construction site (`diagnostics.rs`) always built it as `Vec::new()`
    with a comment already admitting as much; zero consumers in either
    text or `--json` output. Exact same shape as `NetworkStatus.
    pending_requests`, already found and removed under `LIVE-001`'s own
    addendum -- this was that fix's twin, missed by the same cleanup
    pass. Bundled here rather than as a separate change since it lives on
    the exact `StatusResponse` struct this redesign already edits.

    Wire changes: `PeerStatus.is_coordinator: bool` (new, `#[serde(default)]`),
    `NetworkStatus.subnet: String` (new, `#[serde(default)]`),
    `StatusResponse.pending_networks` (removed). All three are
    `#[serde(default)]`-compatible or outright removed, so an old daemon's
    response still decodes against a new CLI (missing fields default;
    the removed field is simply never read, whether or not an old daemon
    still sends it).
    """
    req_id = "STATUS-002"


class NetworkStatusNetworkField(Requirement):
    """REQUIREMENT-ID: STATUS-NETWORK-FIELD-001

    `NetworkStatus.name` reads as stale/ambiguous next to every other
    per-network field (`subnet`, `network_key`, `nuke_consensus_threshold`)
    and the CLI's own `--network` flag naming. Unlike `GroupBlob` (durably
    published to the DHT, must decode data written months ago),
    `NetworkStatus` is a purely ephemeral RPC type regenerated fresh on every
    `tetron status` call -- so there is no "decode old data" concern here,
    only a "which binaries are talking to each other right now" one.

    **Why additive, not a hard rename, despite there being no third-party
    consumer:** confirmed by reading both consumer repos directly --
    `tetron-webui` and `tetron-systray` both depend on `tetron-proto` as a
    git dependency floating on tetron's `main` (not a pinned tag, by their
    own design) and both access `.name` as a **direct Rust field**, not just
    a loosely-typed JSON key. Both are already deployed across a real fleet
    (6 machines; 4 laptops additionally run `tetron-systray`), and the
    policy is to upgrade all three components together fleet-wide -- but
    two of the four systray laptops (`inspiron1`, `sneak`) need scheduling
    coordination rather than being immediately reachable, so "always
    upgraded together" is a policy, not a guarantee of atomicity. A hard
    rename would turn any slip in that coordination into a compile break in
    two repos the user owns, for a purely cosmetic naming improvement --
    disproportionate. Adding `network` alongside `name` (identical value,
    `#[serde(default)]`) makes the transition a non-event regardless of
    actual rollout order or timing.

    **Fix:** `NetworkStatus` gained `network: String` (`#[serde(default)]`)
    alongside the existing `name: String` (left as-is, not `#[deprecated]`
    -- that attribute would trip `clippy -D warnings` at every site still
    populating it during the transition, for no benefit over a doc comment).
    Both daemon-side construction sites (`src/daemon/mesh/diagnostics.rs`,
    the lock-read-failure fallback and the normal path) populate both fields
    with the identical value. Tetron's own CLI (`src/cli/status.rs`) already
    switched its reads to `.network` -- no reason for tetron's own code to
    lag the field it just introduced.

    **Fleet cleanup, tracked not scheduled:** removing `name` for real is a
    follow-up, gated on confirming every one of the 6 daemons and all 4
    systray laptops (plus wherever `tetron-webui` ends up deployed) are
    running a build that reads `.network`. See `DO-NOT-COMMIT/TODO.md`'s
    dedicated checklist -- until every box is checked, `name` stays.

    **Companion changes in sibling repos (same wave, not gated on this
    commit):** `tetron-webui/src/api.rs` switches its own field read from
    `n.name` to `n.network` (and its own re-exposed JSON key, `"name"` ->
    `"network"`, plus the matching `static/app.js` references) and
    `tetron-systray/src/main.rs` switches its ~7 `net.name` sites to
    `net.network`. Both are gated on `cargo update -p tetron-proto` picking
    up this field, which itself is gated on this commit reaching the public
    GitHub remote (not this agent's action).
    """
    req_id = "STATUS-NETWORK-FIELD-001"


class StatusMemberCountExcludesAdmins(Requirement):
    """REQUIREMENT-ID: STATUS-003

    Found live on a real multi-admin network (USER's "shallows" network,
    2026-07-22): `tetron status`'s per-network header line showed `admins
    2/2   members 4/5`, but the peer table right below it listed only 4
    non-admin members total (one, `air`, offline) -- the "members" total
    should have read `3/4`, not `4/5`.

    **Root cause:** `print_network` (`src/cli/status.rs`, added by
    `STATUS-002`) computed the `members` column from *all* peers, admins
    included:

    ```rust
    let online = net.peers.iter().filter(|p| p.connection.is_some()).count();
    ...
    "members {online}/{}", net.peers.len()
    ```

    `net.peers` (the wire `PeerStatus` list) holds every peer regardless of
    role, so an admin peer (in the reported case, a co-coordinator with a
    live connection) was counted into both the numerator and denominator of
    "members" -- on top of already being counted in `admins` just to its
    left. Both header numbers were inflated by exactly one for each online
    admin peer; a network with only one admin (self, never in `net.peers`)
    would never have shown the bug, which is why `STATUS-002`'s own
    live-testing pass didn't catch it.

    **Fix:** `online` and the denominator both filter to `!p.is_coordinator`
    before counting, matching the `admins_online`/`admins_total` pair's own
    care to count each role exactly once. `--json` output was never
    affected -- `PeerStatus.is_coordinator` and `connection` were already
    correct per-peer; only the derived text-mode aggregate was wrong.
    """
    req_id = "STATUS-003"


class AdminAddHostnameResolutionCaseInsensitive(Requirement):
    """REQUIREMENT-ID: STATUS-004

    Found live immediately after `STATUS-003`, same "shallows" network:
    `tetron admin shallows add erikk-ThinkPad-P1` failed with `could not
    resolve peer 'erikk-ThinkPad-P1'`, even though that exact host was
    listed in `tetron status` moments earlier -- as `erikk-thinkpad-p1`.

    **Root cause:** every hostname a member can ever have is lowercased at
    creation (`hostname::sanitize_hostname`, called from `generate_hostname`
    and any explicit `--hostname`) -- OS hostnames especially are routinely
    mixed-case (`erikk-ThinkPad-P1` was this host's actual OS hostname), so
    a user recalling or retyping it from memory has every reason to type it
    back with its original casing. `MeshManager::resolve_peer_name`
    (`src/daemon/mesh/runtime.rs`) compared with a case-sensitive `==`,
    so the mismatch was silently a no-match rather than a resolvable typo.

    **Why this is safe, not just convenient:** because every stored
    hostname is already guaranteed lowercase, two roster entries can never
    differ *only* by case -- there is no real hostname a case-insensitive
    match could confuse for another. Loosening the comparison forgives
    exactly one thing: a user's own capitalization habits, never a
    genuinely ambiguous choice between two peers.

    **Fix:** `resolve_peer_name`'s hostname branch now compares with
    `str::eq_ignore_ascii_case` instead of `==`. Scoped narrowly to this one
    resolver -- `resolve_short_id_any_network` (short id / endpoint id
    prefix matching, used by `kick`/`nuke --second`) is unaffected and
    correctly stays exact: those are cryptographic identifiers a user is
    expected to copy from `tetron status` output verbatim, not recall from
    memory, and hex ids carry no meaningful capitalization ambiguity to
    begin with.
    """
    req_id = "STATUS-004"


# --------------------------------------------------------------------------
# CONFIG-AUDIT-002: five more hardcoded constants become `tetron config set`
# keys, matching HARDEN-005's precedent
# --------------------------------------------------------------------------

class ConfigurabilityAuditBatchTwo(Requirement):
    """REQUIREMENT-ID: CONFIG-AUDIT-002

    A 2026-07-24 pass through the TODO's "Configurability audit" section
    (opened after HARDEN-005 shipped `ratelimit.*` as "batch 1") turns five
    more compiled-in constants into `tetron config set <key> <value>` keys,
    the standing "configurable knobs over hardcoded values" preference. All
    five are new flat `Option<T>` fields on `AppConfig`/`Settings`
    (`src/config.rs`) -- unrelated to each other conceptually, so no nested
    sub-struct the way `RateLimitConfig` groups six related fields -- each
    `None` meaning "use the compiled default"; an empty value resets, same
    convention as every existing key. Applies on `sudo tetron restart` like
    every other `tetron config set` key -- none of these five live-reload
    mid-run.

    **`nuke-proposal-ttl`** overrides `membership::NUKE_PROPOSAL_TTL_SECS`
    (compiled default 24h). `active_nuke_proposers`/`nuke_consensus_reached`/
    `resolve_nuke_proposer` gained a `ttl_secs: u64` parameter (previously
    read the module constant internally) so they stay pure functions of their
    arguments; both real call sites (`diagnostics::status`,
    `runtime::nuke_network`) resolve `config::load().ok().and_then(|c|
    c.nuke_proposal_ttl).unwrap_or(NUKE_PROPOSAL_TTL_SECS)` once per call and
    thread it through, rather than the membership functions reading config
    themselves (keeps `membership.rs` free of a `config` dependency).
    Deliberately a *global* daemon setting, not paired with the per-network
    `nuke_consensus_threshold` the way the original TODO note mused it might
    naturally belong (`GroupBlob`/`tetron create --nuke-consensus`) --
    revisit as a per-network `tetron create` flag instead if that turns out
    to matter in practice; going global for now keeps this batch free of any
    wire-format/`GroupBlob` change.

    **`listen-port`** overrides `transport::TETRON_LISTEN_PORT` (compiled
    default 43737). Confirmed daemon-wide, not per-network: one shared iroh
    `Endpoint`/UDP socket serves every joined network (their isolation is by
    ALPN, not by port), so there is exactly one real call site
    (`bootstrap::build_daemon`, which already has `app_config` loaded at that
    point) resolving `app_config.listen_port.unwrap_or(transport::
    TETRON_LISTEN_PORT)` and passing it into `transport::
    create_endpoint_with_alpns`'s new `listen_port: u16` parameter.

    **`poller-interval`** overrides the group poller's hardcoded
    `Duration::from_secs(60)` tick (`reconverge::spawn_group_poller`). Read
    once at spawn time (not per tick, matching every other config-backed
    daemon setting -- none of them live-reload mid-run either); a changed
    value takes effect on the next `tetron restart`, same as `listen-port`.
    This same function also gained `SYNC-001`'s manual-trigger `Notify`
    parameter in the same change (they touch the same `tokio::select!`), so
    the two requirements share one commit rather than the usual
    one-commit-per-requirement split -- see `SYNC-001`'s own docstring for
    why forcing them apart would have meant literally writing and then
    discarding an intermediate, half-wired version of the same function.

    **`log-retention`** overrides the hardcoded `.max_log_files(7)` in
    `main::init_tracing` (days). `init_tracing` runs before most other daemon
    init, at the very top of `main()` -- but `config::load()` is a plain sync
    filesystem read with no dependency on tracing being up yet, so calling it
    this early is safe.

    **`invite-default-expiry`** overrides the `None => 7 * 24 * 3600` default
    in `invite_handler::invite_create` (used when `tetron invite create` is
    called without `--expires`). `Option<u64>` semantics mirror `--expires`'s
    own convention: `None` (unset) = compiled default (7 days), `Some(0)` =
    the *configured* default is "never expires" (matching `--expires 0`/
    `--expires never`), `Some(n)` = configured default of `n` seconds. The
    auto-minted invite printed by `tetron create` (a second, separate
    hardcoded `7 * 24 * 3600` literal in `create_join.rs`, found while
    auditing every occurrence of that literal) now resolves the same
    configured default too, rather than only the explicit `invite create`
    path honoring it -- consistency the original TODO note didn't call out
    but which follows directly from "one default, one place it's defined."

    **Shared prep:** `parse_duration` (human-readable duration strings --
    `"24h"`, `"7d"`, `"30m"` -- into seconds) moved from a private fn in
    `invite_handler.rs` to `pub(crate) fn` in `config.rs`, since both
    `invite-default-expiry` and `nuke-proposal-ttl`'s `config_set` parsing
    need it and `config.rs` must not depend on `daemon::mesh`. Its existing
    unit tests moved with it; `invite_handler.rs` now calls
    `config::parse_duration`.
    """
    req_id = "CONFIG-AUDIT-002"


# --------------------------------------------------------------------------
# SYNC-001: manual DHT/group-poller trigger (`tetron sync`)
# --------------------------------------------------------------------------

class ManualGroupPollerTrigger(Requirement):
    """REQUIREMENT-ID: SYNC-001

    Discussed and requested the same session as `CONFIG-AUDIT-002`: today the
    group poller (`reconverge::spawn_group_poller`, one per joined network,
    coordinator or member) only ever wakes on its own timer or daemon
    shutdown -- there is no way to ask "check right now" after an action a
    user knows just changed the blob (minting an invite, granting admin) and
    wants a peer to notice sooner than the configured interval.

    **Mechanism:** `NetworkHandle` (`src/daemon/mod.rs`) gains
    `poller_notify: Arc<tokio::sync::Notify>`. Unlike the existing
    `dht_notify: Option<Arc<Notify>>` (coordinator-only, drives *publish*
    after admission/kick), the group poller runs on every node regardless of
    role -- it *fetches* -- so this is a separate, always-constructed field
    (never `Option`), built fresh at every real `NetworkHandle` construction
    site (`create_join.rs`'s create and member-join paths, `runtime.rs`'s
    `restore_coordinator_network`) and cloned into `spawn_group_poller`'s new
    `notify: Arc<Notify>` parameter. The one exception:
    `create_join.rs::try_dht_fallback_join`'s degraded join path (spawns only
    `spawn_reconnect_loop`, no poller at all, pre-existing behavior) still
    constructs the field for struct-literal completeness, but a `tetron sync`
    against a network stuck on that path is a harmless no-op until/unless
    that path grows a poller of its own.

    **Cooldown:** a spammed manual trigger must not reduce the *effective*
    poll interval below a floor, since each tick does a real DHT resolve (and
    potentially a blob fetch). `spawn_group_poller`'s `tokio::select!` gains a
    third arm, `_ = notify.notified() => { ... }`, guarded by a `last_poll:
    Instant` tracked across iterations: if less than
    `MIN_MANUAL_SYNC_INTERVAL` (2s) has elapsed since the last tick (timer or
    manual), the manual wake `continue`s straight back to `select!` without
    doing the resolve. A timer-driven tick is never held back by this check
    -- the configured interval itself already gates that path -- only the
    manual-trigger arm consults the cooldown.

    **IPC + CLI:** `IpcMessage::Sync { network: Option<String> }`
    (`tetron-proto`), handled by `MeshManager::sync_now` (`daemon/mesh/
    runtime.rs`) -- resolves the optional `--network` the same way
    `resume`/`standby` do (exact local-name match against `self.networks`,
    unscoped means every joined network), then calls `.notify_one()` on each
    target's `poller_notify`. `tetron sync [--network <name>]` (`src/main.rs`
    + `cli::status::ipc_sync`), aliased shape to `resume`/`standby`'s
    optional `--network` (not `leave`/`invite`/`admin`'s required positional
    argument) since this is an operational nudge optionally scoped to one
    network, not an admin/destructive action needing per-network
    disambiguation.

    **Authorization:** investigated whether `check_authorized`
    (`daemon/mod.rs`) actually exempts every read-only IPC op the way
    `AGENTS.md`'s "Privilege & access" section describes ("reads... open to
    any local user") before deciding `Sync`'s level. Found the code only ever
    exempted the single literal `IpcMessage::Status` -- `AdminList`/
    `InviteList` are *not* separately exempted despite reading like they
    should be per that doc -- a small pre-existing doc/code gap, noted in
    `DO-NOT-COMMIT/TODO.md` but intentionally not touched by this
    requirement (out of scope; unrelated to `Sync`). `Sync` itself joins the
    exemption bucket alongside `Status` (`matches!(req, IpcMessage::Status |
    IpcMessage::Sync { .. })`): it causes no local mutation, only asks the
    daemon to do a refresh it was already going to do, sooner.

    **Shared commit with `CONFIG-AUDIT-002`:** both requirements' code
    changes converge on the same lines of `spawn_group_poller` (the
    `poller-interval` config read and this requirement's new `notify`
    parameter/third `select!` arm are inseparable edits to the same
    function signature and loop body), so splitting them into the usual
    one-commit-per-requirement pattern would have meant writing and
    discarding a half-wired intermediate version purely to satisfy commit
    granularity -- not worth the churn for two co-designed, co-requested
    features that shipped, built, and were tested together.
    """
    req_id = "SYNC-001"


class CliExitCodeReflectsDaemonError(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-006

    Found live 2026-07-27 on the very first real run of the new
    `tetron-testsuite` addon's `regression` test: `tetron create --subnet
    10.88.0.0/24` on a node that already has that subnet correctly printed
    the documented refusal ("! create failed ... overlaps a network this
    node already has") to stderr, but the process exited **0** -- the test
    script's own `[[ $? -ne 0 ]]` check failed even though the daemon had
    genuinely refused the request. Not a test-script bug: every one of the
    12 call sites across `src/cli/{network,admin,status,invite,service}.rs`
    that match on `ipc::IpcMessage::Error { message }` call `print_error`
    (stderr only) and then fall through to the enclosing function's
    unconditional trailing `Ok(())` -- so the whole CLI has been
    unscriptable via exit code on any daemon-side refusal, across every
    affected command (`create`, `join`, `nuke`, `kick`, `leave`, `status`,
    `standby`, `sync`, `resume`, `install`, `invite`, `admin`), since
    whenever each was written.

    **The fix already had a correct precedent in the same codebase**,
    untouched by this bug: `cmd_set_operator` (`src/main.rs`) already does
    `print_error(...); std::process::exit(1);` on its own `Error` arm. All
    12 sites are brought in line with that exact shape -- `print_error`
    keeps producing the same human-readable stderr message (no output
    format change), immediately followed by `std::process::exit(1)`
    instead of falling through to `Ok(())`. Deliberately not a bigger
    refactor (e.g. threading a `Result` all the way through `main`'s
    dispatch to centralize exit-code handling in one place) -- that would
    touch far more surface for the same observable behavior, when a
    minimal, already-precedented fix at each existing match arm is
    sufficient and lower-risk.

    Verified by the existing `build`/`clippy`/`test` gates in
    `reconcile.py` (a `Requirement`, not a `Constraint` -- no new
    curated-token gate needed, this is structural/behavioral, not a
    string-literal regression) plus live re-confirmation via the same
    `tetron-testsuite regression` test that found it: its
    `SUBNET-COLLISION-001` check (`tests/regression.sh` in that repo)
    asserts a non-zero exit on the overlap refusal and a zero exit when
    `--force` succeeds, exercising exactly this fix end to end against a
    real daemon in a real VM, not just a compiled-and-linted binary.
    """
    req_id = "CLI-VOCAB-006"


class JsonMemberCountExcludesSelf(Requirement):
    """REQUIREMENT-ID: STATUS-005

    Found live 2026-07-27 on `tetron-testsuite`'s `core-smoke` test, its
    first real run: right after node2 joined node1's network, node1's
    `tetron status --json` showed `"member_count": 2` for a network with
    exactly one other member (`peers` correctly held a single entry for
    node2) -- a script asserting `member_count == 1` after one join fails
    even though the join itself worked perfectly.

    **Same root shape as `STATUS-003`, different call site.** `STATUS-003`
    fixed the *text-mode* "members X/Y" aggregate in `src/cli/status.rs`
    (which was counting admin peers twice) and stated at the time "`--json`
    output was never affected -- only the derived text-mode aggregate was
    wrong." That was true for the admin-double-counting bug `STATUS-003`
    fixed, but not true in general: `network_status`
    (`src/daemon/mesh/diagnostics.rs`) computes the `member_count` field
    from `s.members.all().len()` -- the full roster, self included -- while
    `peers` (built two lines later from the same roster) explicitly filters
    `|m| m.identity != my_id`. `member_count` and `peers.len()` had never
    agreed on a 2-member (self + one other) network, and `AGENTS.md`'s own
    documented CLI behavior ("member count excludes self") was violated by
    the JSON field specifically -- the text-mode output was fine, since
    `src/cli/status.rs`'s own `members_total`/`online` are derived from
    `peers` directly (already excluding self) and never read
    `member_count` at all.

    **Not a hypothetical -- already live in a shipped product.**
    `tetron-webui/static/app.js` displays `net.member_count` directly as
    its "members" stat, so this bug has been showing every network's admin
    dashboard one member high since whenever that field was added. No code
    change needed in `tetron-webui` itself once this is fixed -- it already
    just reads the field as-is.

    **Fix:** `count` in `network_status` now filters by `m.identity !=
    my_id` before counting, matching `peers`'s own filter exactly (`s.roster()`
    is `s.members.all().into_iter().cloned().collect()`, so the two
    computations now walk the identical underlying set with the identical
    predicate).
    """
    req_id = "STATUS-005"
