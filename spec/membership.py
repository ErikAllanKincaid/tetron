from libspec import Requirement, Constraint, UserStory

# --------------------------------------------------------------------------
# CONVERGE-001: Co-coordinator publish race
# --------------------------------------------------------------------------

class CoCoordinatorPublishRace(Requirement):
    """REQUIREMENT-ID: CONVERGE-001

    When a promoted co-coordinator admits new members and publishes an
    updated blob to the DHT, the original coordinator may overwrite it with
    a stale blob on its 300s periodic publish timer. The cascade:

    1. Co-coordinator publishes updated blob (members: orig + co + new1 + new2)
    2. Original coordinator's 300s timer fires, publishes stale blob
       (members: orig + co only) to the SAME DHT key
    3. Co-coordinator's 60s group poller sees DHT hash regressed, fetches
       old blob, overwrites its in-memory state
    4. Members admitted by co-coordinator vanish from both coordinators

    Root cause: multiple coordinators publish to the same DHT key without
    coordination. The lazy publisher on co-coordinators has no dht_notify
    handle and uses polling (10s). The original coordinator's publisher
    overwrites the DHT on notify or 300s timer, regardless of whether the
    DHT already has a newer blob.

    Fix (read-before-write):

    Both `spawn_network_publisher` (original coordinator) and
    `spawn_lazy_publisher` (co-coordinator) add a DHT read before each
    publish. The rule:

    - Track `last_published_hash` (the hash we most recently published).
    - Before publishing, resolve the current DHT record via
      `dht::resolve_network` using the client + network public key
      (derived from `net_secret_key.public()`).
    - Publish if: `last_published_hash` is None (first publish), OR
      the DHT hash matches `last_published_hash` (no one else has
      published since we did), OR the DHT has no record yet.
    - Skip (do not publish) if: the DHT hash differs from both our local
      hash and `last_published_hash`. This means another coordinator
      published a newer blob. The 60s group poller on all nodes will
      fetch and reconcile it within one cycle.

    This prevents the 300s timer from ever overwriting a newer blob. When
    the original coordinator's timer fires but the DHT hash differs from
    `last_published_hash`, the publisher skips the cycle.

    The coordinator MUST also run a group poller (spawned in
    `spawn_coordinator_background_tasks`) to discover blob updates from
    co-coordinators. Without it, the coordinator's in-memory state is
    permanently stale if only co-coordinators publish changes.

    Found: 2026-07-16, e2e test with aorus (original coordinator) and
    xps-17-9720 (co-coordinator) on network "test-tetronnet"
    (10.55.55.0/24).
    """
    req_id = "CONVERGE-001"


# --------------------------------------------------------------------------
# CONVERGE-002: Stale DHT restore on coordinator restart (consequence of
# CONVERGE-001)
# --------------------------------------------------------------------------

class StaleDhtRestore(Requirement):
    """REQUIREMENT-ID: CONVERGE-002

    When the DHT record points to a stale blob (CONVERGE-001), a restarting
    coordinator fails to find the blob bytes at any seed peer and falls back
    to its config file, producing a roster with only the coordinator itself.
    Other members are denied with "no invite presented" because the
    coordinator does not recognize them.

    This is a CONSEQUENCE of CONVERGE-001, not a separate root cause. With
    the CONVERGE-001 (read-before-write) fix, the DHT record always points
    to the latest blob, so a restarting coordinator can find and fetch it.

    Additional hardening: if the DHT fallback fails, the restored
    coordinator should trigger an immediate reconverge (not wait 60s) so
    it discovers the latest blob faster.

    Found: 2026-07-16, consequence of CONVERGE-001.
    """
    req_id = "CONVERGE-002"


# --------------------------------------------------------------------------
# CONVERGE-003: Removed member never cleans up locally (ghost member)
# --------------------------------------------------------------------------

class SelfRemovalNoCleanup(Requirement):
    """REQUIREMENT-ID: CONVERGE-003

    A node that is dropped from the authoritative roster (kicked, or a
    casualty of the CONVERGE-001 publish race) never tears itself down
    locally. Two code paths detect "we are no longer a member":

    1. `spawn_group_poller`'s 60s tick: on detecting self-removal it logs
       `we have been removed from the network` and `break`s its own loop —
       nothing else. The reconnect loop keeps running.
    2. `reconverge_and_apply` (the debounced worker driven by `MemberSync`/
       `BlobUpdated` triggers — the path a live `tetron kick` actually
       exercises, well before the 60s poller would notice): it has *no*
       self-removal check at all. It silently applies a roster that
       excludes the local node, with no detection, warning, or cleanup.

    Neither path stops `spawn_reconnect_loop`, removes the network from
    persisted config, or updates `tetron status`. The observable result: a
    removed node keeps its stale config and keeps redialing coordinators,
    each attempt denied with "no invite presented" (it is a fresh unknown
    peer as far as `CoordinatorAcceptState::handle_connection` is
    concerned) in a tight ~5-6s crash loop — while its own `tetron status`
    keeps reporting a healthy, fully-connected membership indefinitely. No
    ping, no ssh, no traffic of any kind actually moves, and nothing
    anywhere logs an error a user would see. This is the same class of
    silent data-plane failure as SUBNET-BUG-001, and it also means a
    legitimately `tetron kick`-ed member was never actually cleaned up
    locally — closing its connection with `KICK_CODE` alone does not stop
    it from redialing forever.

    Fix: extract a shared `member_removed(members, approved, my_id)` check
    used by both `spawn_group_poller` and `reconverge_and_apply`. On
    detecting self-removal, signal the network name over a new `left_tx`/
    `left_rx` mpsc channel (mirroring the existing `promote_tx`/
    `promote_rx` AdminGrant-promotion signal — background tasks hold only
    field clones, not the full `MeshManager`, so they hand off to the main
    daemon loop that does). `serve_ipc` drains `left_rx` and calls
    `MeshManager::handle_removed_from_network`, which runs the same
    teardown as `tetron leave` (cancel the network's token — stopping the
    reconnect loop, poller, and publisher in one step since they all select
    on it — drop peers, unregister the ALPN, delete the network from
    config). `reconverge_and_apply` checks self-removal *before* applying
    the fetched roster to local state, so it never installs a self-less
    membership list in the first place.

    Out of scope for this fix: the CONVERGE-001 publish race itself can
    still let an objectively-stale-but-later-written blob win over a
    genuinely newer admission (no logical/version clock arbitrates the
    two) — that is the root cause of *why* a member can be wrongly
    dropped, tracked separately in docs/TODO.md. This fix addresses the
    local cleanup once removal is (correctly or incorrectly) detected, not
    the DHT race that produces a false removal.

    Found: 2026-07-16, network "converge-test" — X10SRA admitted by
    co-coordinator xps-17-9720, then lost to a CONVERGE-001-style stale
    publish from the original coordinator (590i-aorus-ultra); X10SRA's own
    `tetron status` kept reporting it as a healthy 3-member network for
    25+ minutes with zero working connectivity.
    """
    req_id = "CONVERGE-003"


# --------------------------------------------------------------------------
# CONVERGE-005: Monotonic generation counter closes the CONVERGE-001 publish
# race at its root (raw hash comparison could not tell newer from stale)
# --------------------------------------------------------------------------

class MonotonicBlobGeneration(Requirement):
    """REQUIREMENT-ID: CONVERGE-005

    CONVERGE-001's read-before-write guard (a9b0afa) compared raw DHT hashes:
    it could tell "did the record change under me" but not "is that change
    actually newer." Reproduced live twice more on 2026-07-16 with a9b0afa +
    6b2954d already deployed: once a publisher saw a DHT hash it didn't
    recognize, it deferred to that hash *permanently* — every subsequent
    publish attempt saw the same "mismatch" and skipped, even when the
    publisher's own state was objectively the correct, newer one (it had just
    admitted a member the DHT's blob didn't know about). A slower coordinator's
    stale periodic republish could out-write a co-coordinator's fresher
    admission purely by landing in the DHT later, permanently burying the
    newer state. Root cause: no logical clock, only wall-clock write order.

    A second, compounding gap: `spawn_group_poller`'s blob-fetch only tried
    live `PeerTable` connections over the `iroh-blobs` ALPN, with no seed-peer
    fallback (unlike `reconverge_and_apply`'s `fetch_verified_blob`, which
    tries both). Observed failing with "could not fetch updated group blob
    from any peer" even while a live, traffic-passing mesh connection to the
    same peer was up — so a coordinator could detect a hash change and still
    never converge on it.

    Fix: add `generation: u64` to `GroupBlob` (msgpack, `#[serde(default)]`
    for pre-generation compatibility) and to the signed pkarr network record
    (`g,<n>` TXT field, mirroring the existing `g,<n>` cert-generation-floor
    record in `dht.rs` — same pattern, same file, already precedented).
    `NetworkState` carries the same field: `refresh_snapshot()` recomputes
    hash/bytes from whatever generation is already set (adopting a fetched
    blob sets it directly, never bumped); a new `bump_generation_and_refresh()`
    increments it first, called from every genuine *local* content mutation
    (admit, kick, invite create/revoke, admin grant) instead of plain
    `refresh_snapshot()`.

    `dht_read_before_write` (`publish.rs`) is rewritten around generation, not
    hash: publish whenever the DHT sits at a strictly lower generation than
    ours (regardless of whether we recognize its hash — this is the actual
    fix, closing the permanent-wedge failure mode), defer whenever it's
    strictly higher. An exact generation tie (two coordinators independently
    mutated from the same base) is left alone rather than fought over — the
    loser's own next local mutation bumps past the tie and wins outright,
    rather than the two publishers flip-flopping forever. `spawn_group_poller`
    now gates its fetch on `remote_generation > current_generation` (not raw
    hash inequality) and fetches via `fetch_verified_blob` (peer + seed
    fallback) instead of its own live-peer-only loop, closing the second gap.
    `reconverge_and_apply` adds a defense-in-depth downgrade guard: a freshly
    fetched blob is only applied if `data.generation >= current_generation`,
    so a lagging seed peer's stale copy can never regress local state even if
    it happens to still verify against a signed hash.

    Verified live on 3 bare-metal machines (590i-aorus-ultra, xps-17-9720,
    X10SRA), reproducing the exact scenario that wedged permanently before
    this fix (co-coordinator xps admits x10sra while original coordinator
    aorus is offline/stale): aorus's log shows
    `group blob changed current_generation=0 remote_generation=3` and
    correctly fetches and applies the 3-member blob within one 60s poller
    cycle, with zero manual restart — where it previously stayed wedged at 2
    members indefinitely. Re-ran a `tetron kick` afterward to confirm
    CONVERGE-003's cleanup-on-removal still fires correctly alongside the new
    generation logic (it does, and faster — MemberSync-triggered rather than
    poller-bound).

    Out of scope: an exact generation tie with genuinely divergent content
    (two coordinators admitting different members from the same base
    generation) is not merged — one side's mutation is deferred until its own
    next local change bumps past the tie. A true CRDT merge is not attempted;
    this is judged sufficient since admission itself is idempotent (a deferred
    admit can simply be retried).

    Found: 2026-07-16, network "converge-test" across 590i-aorus-ultra,
    xps-17-9720, X10SRA — the CONVERGE-001 race reproduced twice more with its
    original fix already deployed, prompting the root-cause fix here.
    """
    req_id = "CONVERGE-005"


# --------------------------------------------------------------------------
# CONVERGE-006: Member boot-restore has no config fallback (asymmetric with
# the coordinator restore path)
# --------------------------------------------------------------------------

class MemberRestoreConfigFallback(Requirement):
    """REQUIREMENT-ID: CONVERGE-006

    `connect_all_networks`'s member-restore path (`join_network_inner(initial
    = false)`) calls `resolve_and_fetch_blob`, which has zero resilience to a
    transient DHT/network hiccup at boot: it resolves the pkarr record, then
    fetches the blob from one of the record's seed peers over iroh-blobs, with
    no local blob-store fallback and no persisted-config fallback. If pkarr
    resolution fails (relay unreachable, DNS not yet up) or none of the seed
    peers happen to be dialable at that exact instant, it returns `Err`,
    `join_network_inner` propagates it, and `connect_all_networks` just logs a
    warning and moves to the next network. The network is silently absent for
    that daemon's entire runtime -- invisible in `tetron status`, with no
    retry, no backoff, recoverable only by noticing and running `sudo tetron
    restart` again.

    This is asymmetric with the *coordinator* restore path:
    `restore_member_roster` (`runtime.rs`) tries the local blob store first,
    then DHT/seeds, and if DHT resolution fails outright, falls back to the
    persisted config roster (`NetworkConfig.members`/`.approved`) rather than
    erroring out -- a restarting coordinator degrades gracefully to a
    possibly-stale-but-non-empty roster. A restarting member gets none of
    that, even though the same config-roster data is already persisted for
    members too (`persist_join_config` writes it on every successful
    join/reconnect) -- it is simply never consulted on this path.

    Fix: on `join_network_inner`'s boot-restore call only (`!initial` -- a
    fresh `tetron join` still fails loudly, which is correct: there is no
    prior membership to fall back to), if `resolve_and_fetch_blob` fails,
    build a `GroupBlob`-shaped fallback directly from the persisted
    `NetworkConfig` (mirroring `restore_member_roster`'s config-fallback
    branch): `members`/`approved` from config, `generation: 0` (informational
    only for a member, which never publishes), `subnet` from
    `config::node_subnet()` (safe per the SUBNET-BUG-001 invariant that an
    already-joined member's node subnet already matches its network's), empty
    `suggested_firewall`/`reusable_keys`/`invites` (not persisted per-member;
    the next successful reconverge repopulates them like any other stale
    field). If the config lookup also comes up empty (no member entries
    persisted, e.g. this network was never actually joined), propagate the
    original error unchanged.

    A fresh `dial_reconnect` still runs against this fallback blob exactly as
    it already does for a live-fetched one, so the existing "coordinator
    offline at restore, reconnect loop will retry" degrade-and-recover
    machinery is unaffected -- this only widens what counts as "got *a*
    roster to start from" to include the persisted config, not just a live
    DHT/seed fetch.

    Found: 2026-07-16, while investigating CONVERGE-004 (a related but
    distinct finding: initial diagnosis of "no poller ever spawns on
    boot-restore" was live-reverified and found inaccurate for the ordinary
    reconnect-succeeds case -- `finalize_join` always spawns one. The real gap
    was traced to this narrower resolve/fetch-failure window instead.
    """
    req_id = "CONVERGE-006"


class InviteStore(Requirement):
    """REQUIREMENT-ID: INVITE-001

    tetron gains a per-network invite store at
    `<config_dir>/invites/<network>/<invite-id>.toml`. Each file holds:
    - `id`: 8-byte random hex invite identifier (also the filename stem).
    - `secret_hash`: blake3 hex of the invite secret (64 hex chars), so the
      plaintext secret is never persisted.
    - `created_at`, `expires_at` (0 = never): unix timestamps.
    - `used`: bool, set true on single-use redemption.

    The store directory auto-creates under the config dir via the existing
    `config_dir()` helper. No new top-level config keys.
    """
    req_id = "INVITE-001"


class InviteMinting(Requirement):
    """REQUIREMENT-ID: INVITE-002

    The coordinator daemon can mint invite keys. On `invite_create`:
    1. Generate a random 16-byte secret.
    2. Compute its blake3 hash.
    3. Persist the hash + metadata in the invite store (INVITE-001).
    4. Return the printable invite key: `bs58(network_pubkey(32) ||
       coordinator_pubkey(32) || secret(16))`, using the existing
       `invite::encode_invite_code()`.

    The invite key encodes the minting coordinator's pubkey so the joiner
    knows which coordinator to dial. If the minting coordinator goes offline
    before the invite is redeemed, the joiner must wait or obtain a fresh
    invite from another coordinator (cross-coordinator gossip is deferred).
    """
    req_id = "INVITE-002"


class InviteStoreValidation(Requirement):
    """REQUIREMENT-ID: INVITE-003

    On join with `invite_secret` set, `redeem_invite_and_admit` in
    accept.rs checks the local invite store (INVITE-001) before falling
    back to `GroupBlob.reusable_keys` validation (D1 compat path):

    1. Hash the presented secret.
    2. Look up the hash in the store.
    3. If found and not expired and not used:
       - Mark single-use invites as `used = true`.
       - Auto-admit the joiner (skip pending queue).
    4. If not found, expired, or already used:
       - Send `JoinDenied`.

    Single-use invites are burned on first successful redemption.
    """
    req_id = "INVITE-003"


class CliInviteSubcommand(Requirement):
    """REQUIREMENT-ID: INVITE-004

    New CLI subcommand:

        tetron invite <network> create [--expires <duration>]
        tetron invite <network> list
        tetron invite <network> revoke <invite-id>

    `create` prints the invite key and its invite-id. `list` shows
    outstanding invites (id, status, age, expiry). `revoke` marks an invite
    as used so it cannot be redeemed. `tetron invite` with no subcommand
    shows subcommand help.

    The initial `cli/invite.rs` (currently requests/accept/deny handlers)
    is renamed to `cli/requests.rs` to avoid confusion; the invite handlers
    live in a new `cli/invite.rs`.
    """
    req_id = "INVITE-004"


class InviteIpcOps(Requirement):
    """REQUIREMENT-ID: INVITE-005

    New IPC messages for invite operations (tetron-proto/src/ipc.rs):

    - `InviteCreate { network, expires: Option<String> }` ->
      `InviteCreated { invite_key, invite_id, expires_at }`
    - `InviteList { network }` ->
      `InviteListResponse { invites: Vec<InviteInfo> }`
    - `InviteRevoke { network, invite_id }` ->
      `Ok`

    Daemon-side handlers `MeshManager::invite_create`,
    `MeshManager::invite_list`, `MeshManager::invite_revoke` in a new
    `daemon/mesh/invite_store.rs` module.
    """
    req_id = "INVITE-005"


class PostCreateInitialInvite(Requirement):
    """REQUIREMENT-ID: INVITE-006

    `tetron create` auto-mints one single-use invite key and returns it in
    the `Created` IPC response alongside the room id. The CLI displays it
    as the primary way for peers to join:

        created muddy-sunset-whale
          address  10.88.0.1  ·  abcd…1234
        ──────────────────────────────────────────────
        next: tetron join <invite-key>    single-use invite
              tetron invite <net> create  mint another invite
              tetron up                   activate the VPN

    The room id is still printed (it identifies the network to `create` more
    invites for), but the join hint references the invite key instead.
    """
    req_id = "INVITE-006"


class InviteKeyPrimaryAdmission(Requirement):
    """REQUIREMENT-ID: INVITE-007

    Invite keys are the primary enrollment method. The admission priority
    in `CoordinatorAcceptState::handle_connection` is:

      1. Invite secret presented in JoinRequest  -> redeem_and_admit
      2. Reusable key (D1 compat)                -> admit
      3. No invite, Restricted network           -> queue for live approval (fallback)

    The room id is discovery-only: it identifies the network but does not
    suffice to join without an invite key. `tetron join <room-id>` (no
    invite) lands in the pending queue (step 3 above) and waits for a
    coordinator to run `tetron accept`.

    FUTURE (not yet implemented): remove the pending queue entirely so that
    an invite key is required in all cases and `tetron join <room-id>` fails
    with a message directing the user to obtain an invite key. For now, the
    live-approval fallback remains so an operator can manually admit a peer
    who has the room id but no invite.

    The wire protocol still accepts `JoinRequest` without `invite_secret`
    on open networks (D1 compat for full-tetron open-mode networks), but
    tetron only creates closed networks.
    """
    req_id = "INVITE-007"


class InviteFormatUnchanged(Requirement):
    """REQUIREMENT-ID: INVITE-008

    The invite code format is unchanged from upstream:
    `bs58(network_pubkey(32) || coordinator_pubkey(32) || secret(16))`.
    The existing `invite::encode_invite_code` and `decode_invite_code` in
    `src/invite.rs` are reused as-is. The CLI `ipc_join()` in
    `src/cli/network.rs` already detects invite codes vs room ids via
    `decode_invite_code` and sends the secret in `JoinRequest.invite_secret`
    -- no change needed on the joiner side.
    """
    req_id = "INVITE-008"


class InviteExpiryDefault(Requirement):
    """REQUIREMENT-ID: INVITE-009

    Invite keys expire by default. `tetron invite create` without `--expires`
    mints an invite that expires in 7 days. The `--expires` flag accepts
    durations ("24h", "7d", "30d") to override. To create an invite that
    never expires, pass `--expires 0` or `--expires never`.

    `InviteStore::create` defaults `ttl_secs: None` to `7 * 86400` (7 days)
    instead of no expiry. An `expires_at` of 0 means no expiry (opt-in).

    **Correction, 2026-07-17:** `invite_create`'s own rustdoc (`invite_
    handler.rs`) had drifted to say "If absent the invite never expires,"
    directly contradicting the `None => 7 * 24 * 3600` default four lines
    below it and this requirement's own text. The 7-day default is correct
    and intentional (kept as-is); only the stale comment was wrong. Fixed to
    match.
    """
    req_id = "INVITE-009"


# --------------------------------------------------------------------------
# INVITE-CHECKSUM-001: invite codes carry a blake3 checksum (upstream
# ba15684 `feat(invite)`, ported from the 2026-08-05 upstream review)
# --------------------------------------------------------------------------

class InviteCodeChecksum(Requirement):
    """REQUIREMENT-ID: INVITE-CHECKSUM-001

    The invite code is `bs58(network_pubkey(32) || secret(16))` — 48 bytes
    (the BLOB-001 format; it already superseded INVITE-008's coordinator-
    pinning shape) — with no integrity check. base58 carries no error
    detection of its own, so a dropped/garbled character can still decode
    to a payload of the right length: a "well-formed" invite for a network
    that doesn't exist. The user sees a confusing join failure much later
    instead of "invalid invite code".

    Fix (ported from upstream `ba15684`): `encode_invite_code` appends 4
    bytes of `blake3(payload)` after the 48-byte payload (52 bytes total
    before base58). `decode_invite_code` accepts BOTH the checksummed form
    (verifies the 4-byte checksum, rejecting on mismatch) and the legacy
    unchecksummed 48-byte form — codes already handed out keep working.
    The break is one-directional: new codes are 4 bytes longer, so an OLD
    peer cannot redeem them, but no in-flight migration is needed (both
    ends ship in the same release).

    Documented boundary: a corruption that shrinks the payload by exactly
    those 4 bytes lands on the legacy shape and skips the check — closes
    when legacy support is dropped in a future protocol version.

    `src/invite.rs`'s existing tests (`code_roundtrip`,
    `decode_rejects_bad_length`) extend; new coverage: legacy-48 decode,
    checksum-mismatch rejection, encoded-length assertion, and
    `is_bare_room_id` discrimination. Pure join-path UX; the invite code
    never crosses the wire (decoded client-side into `network_key` +
    `invite_secret` before IPC, per INVITE-008), so there is no wire or
    `tetron-proto` change.

    **Follow-up, 2026-08-05 (found by independent code review of the
    initial commit):** `cli/network.rs`'s `ipc_join` matched on
    `Err(_)` and treated *any* decode failure as a bare room id — which
    would have swallowed the specific "checksum mismatch" error behind
    the daemon's generic "a valid invite key is required" denial. Fixed
    by adding `invite::is_bare_room_id` (base58 decodes to exactly 32
    bytes): only a genuine room id falls through to the daemon; a
    48/52-byte-shaped failure (corrupted or mistyped invite) now returns
    the specific decode error up front.

    Independent of DHT-ERRCAUSE-001, TUN-SENDERCACHE-001, and
    IPV4-MIN-IHL-001: disjoint files, no shared state. May land in any
    order.

    Found: 2026-08-05, upstream rayfish review `a56b4b9..b002168`
    (`DO-NOT-COMMIT/REVIEW_upstream-rayfish_2026-08-05.md`, item 1).
    """
    req_id = "INVITE-CHECKSUM-001"


class RemoveLiveApproval(Requirement):
    """REQUIREMENT-ID: LIVE-001

    Remove the live-approval admission path entirely. Invite keys are the
    only way onto a tetron network. Removed:

    - Pending join queue (`pending: HashMap<EndpointId, PendingJoin>`) and
      `PendingJoin` struct in `NetworkState`.
    - `evict_oldest_pending`, `MAX_PENDING_JOINS`.
    - `ControlMsg::JoinPending` sender (decode-only kept for D1 compat).
    - `MeshManager::list_requests`, `accept_request`, `deny_request` and
      their IPC dispatch.
    - IPC variants `Requests`, `AcceptRequest`, `DenyRequest`,
      `PendingRequests`, `PendingRequestInfo`.
    - CLI commands `tetron requests`, `tetron accept`, `tetron deny` and
      `src/cli/requests.rs`.
    - Daemon handler file `src/daemon/mesh/invite.rs` (entirely replaced by
      `invite_handler.rs` for invite-key operations).
    - Config `PendingJoinEntry`, `pending_joins` field,
      `add_pending_join`/`remove_pending_join`.
    - Pending-joins restart loop in `connect_all_networks`.
    - `was_approved` parameter on `admit_peer`.
    - `owner_admits` function in `accept.rs` (paired-device D1 shortcut).

    The `approved` field in `GroupBlob` and `ApprovedList` type are
    retained for D1 compat decode only — a full-tetron coordinator may
    publish an approved list that tetron nodes must decode without error.
    tetron coordinators never write to it.

    **Follow-up, 2026-07-18:** one vestige survived this removal —
    `NetworkStatus.pending_requests: usize` (`tetron-proto/src/ipc.rs`)
    stayed on the wire and kept being populated by `diagnostics.rs`'s
    `network_status()`, but hardcoded to `0` at both construction sites
    (nothing left to count once the `PendingJoin` queue was gone) and never
    read by any CLI display code (confirmed via grep — zero hits in
    `src/cli/*.rs`). Found while fixing a related stale doc reference
    (`AGENTS.md` still listing the removed `tetron requests`/`accept`/
    `deny` commands). Removed: the field itself, its hardcoded-`0`
    construction sites, and the `pending_requests` element of
    `network_status()`'s destructured tuple (folded into a 3-element tuple
    with `members`/`member_count`/`nuke_proposals`). Not part of the signed
    `GroupBlob`/its canonical hash, so — unlike `suggested_firewall`'s
    removal — this carried no wire-compat hashing concerns, just a
    `NetworkStatus` field drop.

    **Second follow-up, 2026-07-20:** `pending_requests`'s exact twin,
    `StatusResponse.pending_networks: Vec<String>`, was missed by the
    follow-up above. Found while surveying available-but-unshown fields
    for the `STATUS-002` status redesign, unrelated to it otherwise. Its
    own doc comment claimed to reflect `AppConfig.pending_joins`, which
    this same requirement (`LIVE-001`) removed entirely; the one
    construction site (`diagnostics.rs`) always built it as `Vec::new()`,
    with a comment already admitting as much (`// LIVE-001 removed the
    pending-join queue; always empty.`). Verified zero consumers in
    *either* output mode this time — `grep -rn "pending_networks"
    src/cli/*.rs` returns nothing, and `status.rs`'s `--json` `json!({...})`
    block doesn't even include the field. Removed: the field, its
    construction site, and its slot in `status()`'s destructured tuple.
    Bundled into the `STATUS-002` implementation commit rather than a
    separate change, since it lives on the exact `StatusResponse` struct
    that redesign was already editing.
    """
    req_id = "LIVE-001"


class PeerAddressCache(Requirement):
    """REQUIREMENT-ID: CACHE-001

    tetron saves known peer addresses (endpoint ID, direct addresses, relay
    URL, last seen timestamp) to a flat file on disk on graceful shutdown and
    periodically every 5 minutes. On startup, the cache is loaded and iroh's
    peer table is seeded before any DHT lookup.

    After an all-offline gap, the first member back tries each cached address
    directly. If any other member is also back, the QUIC handshake succeeds
    and the mesh is live without DHT or relay bootstrap. Stale addresses are
    harmless because iroh verifies endpoint identity via the QUIC crypto
    handshake (wrong address = connection failure, not wrong peer).

    Format: flat msgpack file at `<config_dir>/peercache.msgpack` containing
    `Vec<CacheEntry>` where each entry holds endpoint_id (32 bytes),
    known_addresses (Vec<SocketAddr>), relay_url (Option<String>), and
    last_seen (u64 unix timestamp). Entries older than 30 days are pruned on
    load. Writes are atomic (write to temp file, rename).
    """
    req_id = "CACHE-001"


class PeerAddressCacheEviction(Requirement):
    """REQUIREMENT-ID: CACHE-002

    `PeerAddrCache` (CACHE-001) only ever inserts into its in-memory
    `HashMap<EndpointId, (Vec<TransportAddr>, u64)>` (`update`,
    `peercache.rs:87-91`) -- nothing removes an entry once written. The
    30-day age-based pruning documented at the top of the module ("Entries
    older than 30 days are pruned on load") only actually runs in
    `PeerAddrCache::new`, at daemon startup. `spawn_periodic_save`'s
    5-minute save tick calls `save()`, which just serializes whatever is
    currently in the map -- it does not filter by age. A daemon that stays
    up longer than 30 days without a restart keeps every distinct peer
    it has ever connected to resident in memory (and re-persisted to disk
    every 5 minutes) indefinitely, growing with total unique peers ever
    seen rather than with current membership or traffic.

    Found triaging the 2026-08-02 memory-leak audit (Finding #2,
    `DO-NOT-COMMIT/tetron_memleak.md`); confirmed present by direct
    inspection against `main` 2026-08-06.

    Fix: apply the same age filter `new()` already uses to `save()`, so
    the in-memory map itself is pruned on the same 5-minute cadence it is
    written to disk on, not only at the next process start. `lookup`/
    `update` are otherwise unchanged -- an entry re-updated within the
    30-day window keeps refreshing its `last_seen` and survives normally;
    only entries that have gone stale in memory are dropped.

    Independent of CONVERGE-009 -- different subsystem (peer address
    persistence vs. roster-prune suppression), no shared state.
    """
    req_id = "CACHE-002"


class InviteInBlob(Requirement):
    """REQUIREMENT-ID: BLOB-001

    Move invite storage from machine-local files (`InviteStore`,
    `invites/<network>/<id>.toml`) into the signed `GroupBlob`. An invite is
    an `InviteEntry` struct in the blob:

        struct InviteEntry {
            secret_hash: String,    // blake3 hex
            created_by: EndpointId,
            created_at: u64,
            expires_at: u64,        // 0 = permanent
            used: bool,
        }

    Minting an invite adds an entry to the in-memory blob, signs it, and
    republishes to the DHT. Validating a presented secret: any online
    coordinator hashes the secret and checks the blob's invite table for a
    matching, not-expired, not-used entry. On admission the entry is removed
    (not just marked used) to bound blob size and prevent replay.

    The invite code encoding changes from
    `bs58(pubkey(32) || coordinator(32) || secret(16))` to
    `bs58(pubkey(32) || secret(16))` -- the coordinator endpoint ID is
    dropped so the joiner dials any peer, not the minting machine.

    Supersedes INVITE-001 (machine-local store), INVITE-002 (machine-local
    minting), INVITE-003 (machine-local validation), INVITE-008 (old format),
    and INVITE-009 (expiry logic -- still applies but against blob entries).

    Fetch-before-publish merge is required so concurrent mints from multiple
    coordinators do not clobber each other's entries (the merge logic from
    the PRIVILEGE_TIERS.md design is reused).

    Replay race mitigation: a local reject cache per coordinator (set of
    recently-admitted secret hashes, TTL 5 minutes) plus `InviteUsed` gossip
    (wire message broadcast on admission) prevents a used invite from being
    accepted by a coordinator who has not yet received the updated blob. Once
    the updated blob propagates via DHT poll (~30-60s), the reject cache
    entry expires naturally.
    """
    req_id = "BLOB-001"


class MultiCoordinatorRoutine(Requirement):
    """REQUIREMENT-ID: COORD-001

    `tetron admin add <net> <identity>` is the documented practice for making
    a fully trusted user a coordinator. Every fully trusted member should be
    granted the network key. This eliminates the single-point-of-failure
    where only one machine can admit, mint, kick, or publish.

    The CLI command already exists and works. No code changes are needed.
    Implementation consists of:
    - Update `docs/HOWTO.md` to recommend `admin add` as a routine post-join
      step for every trusted user.
    - Update `docs/TODO.md` to mark multi-coordinator as the expected default.
    - Update `README.md` quickstart to show `tetron admin add` after join.
    """
    req_id = "COORD-001"


# --------------------------------------------------------------------------
# KICK-REQUIRES-ID: kick requires endpoint-id, not hostname/IP
# --------------------------------------------------------------------------

class KickRequiresEndpointId(Requirement):
    """REQUIREMENT-ID: KICK-REQUIRES-ID

    `tetron kick <net> <peer>` must resolve the peer by its cryptographic
    identity (endpoint id / short id) only. No hostname or mesh IP
    resolution. The previous behavior accepted a hostname, mesh IP, or
    short id via `resolve_peer_name`, which made it possible to kick the
    wrong member if two peers shared a similar name or if the operator
    misread a mesh IP.

    Kicking is a destructive action (removes a member from the roster and
    severs all connections). Using the endpoint id as the sole identifier
    ensures the operator is explicitly naming the target by its
    cryptographic identity, not by a human-friendly alias that could be
    ambiguous.

    Implementation: `kick_member` in `runtime.rs` calls
    `resolve_short_id_any_network` directly instead of `resolve_peer_name`.
    The `resolve_peer_name` helper is unchanged and still used by `admin
    add`, where friendly hostname resolution is appropriate.

    Doc updates: CLI help text, HOWTO.md, and README.md updated to show
    short-id-only form.
    """
    req_id = "KICK-REQUIRES-ID"


# --------------------------------------------------------------------------
# NUKE-CONSENSUS: require at least two coordinators to nuke a network
# --------------------------------------------------------------------------

class NukeRequiresConsensus(Requirement):
    """REQUIREMENT-ID: NUKE-CONSENSUS

    `tetron nuke <net>` used to be runnable by any single coordinator,
    immediately publishing an empty DHT record (poisoning the pkarr record)
    and calling `leave_network`. This meant a single compromised or reckless
    coordinator could destroy the network irrecoverably.

    Require at least two coordinators to approve a nuke, **unless there is
    only one coordinator** in the network. A solo coordinator has no one to
    second and retains the original unilateral nuke behavior.

    Detection: count coordinators from the signed roster
    (`Member.is_coordinator == true`, `membership::coordinator_count`). If
    total coordinators >= 2, the nuke is a two-phase proposal; if exactly 1,
    the nuke proceeds immediately (original behavior, unchanged).

    Implemented flow (coordinators >= 2) -- **command-driven only, no
    automatic background trigger** (deliberately narrowed from an earlier
    draft of this spec that had any coordinator's reconverge/poller loop act
    on an observed blob; see "Scoped down" below):

    1. `GroupBlob.nuke_proposals: BTreeMap<String, u64>`, keyed by the
       proposing coordinator's **full identity string** (not
       `EndpointId::fmt_short()` as originally drafted -- a map key must be
       collision-free, and two coordinators' short ids could theoretically
       collide; short ids are used only for CLI display/matching via
       `membership::resolve_nuke_proposer`). Value is the Unix-seconds
       proposal timestamp. `#[serde(default, skip_serializing_if =
       "BTreeMap::is_empty")]`, matching `reusable_keys`/`invites`'s
       convention, so old blobs decode unchanged and an empty map serializes
       to nothing.

    2. `tetron nuke <net>` on a coordinator (`MeshManager::nuke_network`)
       adds the coordinator's own entry to `nuke_proposals`, bumps the
       generation, and checks the *local* result immediately:
       - If that addition itself brings the count of distinct, unexpired
         proposers to two or more (`membership::nuke_consensus_reached`),
         this same call executes the nuke right there -- publishes the empty
         tombstone record (`MeshManager::publish_nuke_tombstone`) and calls
         `leave_network` -- synchronously, no waiting on reconverge.
       - Otherwise it persists + publishes the proposal blob (same
         persist-then-notify pattern as `invite_create`/`invite_revoke`) and
         returns "N/2 coordinators required".

    3. `--second <short-id>` (`membership::resolve_nuke_proposer`) validates
       the named proposal is currently active before proceeding identically
       to a bare `tetron nuke <net>` -- an explicit safety check when there
       are more than two coordinators, not a different code path.

    4. `tetron nuke <net> --cancel` removes the caller's own entry from
       `nuke_proposals` and republishes (not destructive, no consensus check).

    5. Proposals auto-expire via a 24h TTL
       (`membership::NUKE_PROPOSAL_TTL_SECS`,
       `membership::active_nuke_proposers`) -- filtered at read time
       (consensus check, `--second` resolution, `tetron status` display), not
       actively pruned from the map on mutation.

    6. `tetron status` surfaces active (unexpired) pending proposals
       (`NetworkStatus.nuke_proposals`, `ipc::NukeProposalInfo`) so members
       can see a nuke is being considered. This is synced into
       `NetworkState.nuke_proposals` on every reconverge
       (`reconverge_and_apply`, `spawn_group_poller`) -- but purely for
       display; see "Scoped down" below.

    **Scoped down from the original draft (2026-07-17, before
    implementation):** an earlier version of this spec had a coordinator's
    background reconverge/poller loop independently notice an
    already-consensus-reached blob (e.g. a third coordinator who never ran
    `tetron nuke` at all) and execute the tombstone-publish on its own. That
    was deliberately cut: verifying an automatic, background-triggered,
    irreversible action needs the same kind of live multi-coordinator race
    testing CONVERGE-001 needed across two rounds before it was actually
    correct, and the payoff was narrow. What remains is strictly
    command-driven: the *only* code path that can ever publish the
    destructive tombstone is the synchronous `nuke_network` handler. The
    trade: two coordinators proposing at nearly the same instant (before
    either sees the other's write) can leave the blob showing 2 valid
    proposers with nobody having triggered execution -- resolved by either
    coordinator running `tetron nuke` once more, which then sees the merged
    count and finishes it. A liveness gap, not a safety gap -- it fails
    toward *not* destroying the network automatically, not toward an
    unexpected automatic destruction.

    Found: 2026-07-16, during multi-coordinator audit. Race C (no coordinator
    revocation) makes nuke the only way to remove a compromised coordinator.
    Requiring consensus prevents a single key holder from destroying the
    network, while the solo-coordinator exception avoids locking out networks
    that have never promoted a co-coordinator.

    **Two bugs found and fixed via live 3-machine testing, 2026-07-17**
    (neither could have been caught by unit tests alone -- both are
    distributed-convergence failures that only manifest with real network
    latency and real coordinator restarts):

    1. The tombstone's `(hash, generation)` pointer reached the DHT
       correctly, but the actual empty-blob *bytes* were never persisted
       anywhere fetchable -- the executing coordinator calls `leave_network`
       (closing its connections) immediately after publishing, so it was
       typically the only node that ever held them, and every other node's
       `fetch_verified_blob` attempt failed forever ("could not fetch
       updated group blob from any peer or seed"). `member_removed`
       (CONVERGE-003) never fired for remaining members. Predates
       NUKE-CONSENSUS (the original single-coordinator nuke had the
       identical gap) but only surfaces with other members present to
       notice the failure. Fixed by recognizing that a tombstone's content
       is fully deterministic given just its generation (always empty
       members/approved/etc.) -- `membership::try_decode_tombstone`
       reconstructs and verifies it locally, tried before ever attempting a
       peer fetch, sidestepping the distribution problem entirely.

    2. `spawn_group_poller`'s generation comparison treated an exact tie
       (`remote_generation <= current_generation`) as "nothing new" even
       when the hash differed. This is a general liveness bug, not
       nuke-specific: a node's own unrelated local mutations (e.g. pruning
       a peer that gracefully left) can independently advance its
       generation to the same number a different coordinator's mutations
       reached, purely by coincidence -- observed twice in this session via
       two different mechanisms. Once tied, the node would never fetch
       again for that network, regardless of how different the actual
       content became. Fixed: `poller_should_fetch` now also fetches on an
       exact-generation tie if the hash differs; a tie with a matching hash
       still correctly skips as a no-op.

    A third, separate, pre-existing issue was found during this testing but
    deliberately **not** fixed here (needs its own dedicated design, not a
    tail-end change to this spec): a coordinator's unconditional first
    publish after restart (`dht_read_before_write`'s `if
    last_published.is_none() { return true; }`) can resurrect stale state
    if that restart's restore fell back to local config (DHT/blob
    unreachable). Logged as a new TODO, out of scope for NUKE-CONSENSUS.
    """
    req_id = "NUKE-CONSENSUS"


class NukeConsensusThresholdConfigurable(Requirement):
    """REQUIREMENT-ID: NUKE-CONSENSUS-THRESHOLD-001

    NUKE-CONSENSUS's proposer threshold ("2 or more distinct, unexpired
    proposers") was hardcoded regardless of how many coordinators a network
    actually has -- with 100 coordinators, 2 agreeing is not meaningful
    consensus. Made configurable at creation, `tetron create
    --nuke-consensus <n>` (default 2, must be >= 2 -- a value of 0 or 1 would
    let a single coordinator nuke unilaterally once a second coordinator
    exists, defeating the reason NUKE-CONSENSUS exists at all), following the
    exact same treatment `--subnet` already established: fixed once at
    creation, carried in the signed `GroupBlob` (`nuke_consensus_threshold:
    u32`, `#[serde(default = "default_nuke_consensus_threshold")]` so a blob
    predating this field decodes as the historical hardcoded value of 2) and
    the persisted `NetworkConfig` (same shape, same default), never mutated
    by any later command -- no `tetron config`/`admin` path touches it.

    **Why the blob, not just local config:** `nuke_network`'s consensus
    check runs locally on whichever coordinator executes it (deliberately --
    see NUKE-CONSENSUS's own "no reconverge-triggered automatic execution"
    invariant, unchanged here). If the threshold were only a local,
    unsynced config value, coordinators on the same network could silently
    disagree about what "consensus" even means, and `tetron status` could
    never show one authoritative answer. Putting it in the signed blob
    doesn't add a new integrity guarantee beyond what the existing
    trust model already provides (a key holder is already fully trusted --
    it can already do plenty else with the key), but it does make the
    configured value visible and consistent across every coordinator and
    member, and immune to the SUBNET-DRIFT-001 class of restart-induced
    drift -- exactly the same justification `subnet` itself already
    established.

    **Threading, following `subnet`'s own call-site pattern exactly (added
    as the new trailing positional argument everywhere, so no existing
    argument had to move):** `canonical_group_bytes`/`group_blob_hash`
    (`membership.rs`) gained a `nuke_consensus_threshold: u32` parameter;
    `NetworkState`/`NetworkConfig`/`JoinParams`/`RestoredRoster` each gained
    the matching field; every construction site (`create_network_inner`,
    `build_initial_roster`, `build_member_state`, `restore_member_roster`/
    `restore_coordinator_network`, the DHT-fallback `dummy_state`, the
    member reconnect `state_from_blob` closure) threads it through --
    preferring the freshly fetched blob's value when one is available, the
    persisted config only as a fallback (mirroring the `subnet` precedent).
    `nuke_network`'s own check (`runtime.rs`) now reads
    `state.nuke_consensus_threshold` instead of a bare `2`, and the
    proposal-pending message reports `{active_count}/{threshold}` instead of
    the old hardcoded `/2`.

    **Tombstones are the one place that deliberately does NOT read this
    field:** `publish_nuke_tombstone` and its matching local reconstruction,
    `try_decode_tombstone`, both pass the fixed
    `default_nuke_consensus_threshold()` (2) regardless of the real
    network's configured value. A tombstone's content must be fully
    deterministic given just its `generation` (pre-existing invariant, see
    NUKE-CONSENSUS's own tombstone-reconstruction note) -- it carries no
    other network state, so there is nothing for a per-network threshold to
    mean there. Both call sites must agree on the exact same fixed constant
    for the hash to verify; using the named default function (rather than a
    bare literal in each place) keeps that agreement obvious rather than
    coincidental.

    `NetworkStatus.nuke_consensus_threshold` (`tetron-proto`) surfaces the
    configured value via `tetron status`/`--json`, `#[serde(default = ...)]`
    so an older daemon's response still decodes.

    CLI: `Command::Create` gained `--nuke-consensus <n>`; `ipc_create`
    validates `>= 2` client-side for an immediate error (daemon re-validates
    authoritatively, same division of labor as `--subnet`'s CIDR parsing).
    `--help` text for `Nuke`/README/AGENTS.md updated to say "the network's
    configured threshold (default 2)" rather than a hardcoded "two".
    """
    req_id = "NUKE-CONSENSUS-THRESHOLD-001"


# --------------------------------------------------------------------------
# DIAL-001: background, concurrent, timeout-bounded roster dials
# --------------------------------------------------------------------------

class BackgroundConcurrentBoundedDials(Requirement):
    """REQUIREMENT-ID: DIAL-001

    Three related dial-blocking gaps, identified while triaging upstream
    rayfish fixes (02dd60e, fe3f3c0, b26c26b) against tetron's current
    `join.rs`/`create_join.rs`/`runtime.rs` and confirmed still present by
    direct code inspection — not assumed from the upstream commit messages:

    1. `join_mesh_shared`'s `connect_to_roster_peers` (member join/reconnect)
       dialed the rest of the roster serially and `.await`ed the whole loop
       before the join completed. A single unreachable roster member (a
       stale, offline device still listed) blocked the *entire* join/reconnect
       on iroh's uncapped internal handshake timeout before any other peer
       connected — even though the coordinator link was already up and the
       network was otherwise usable.

    2. `dial_all_members` (the coordinator-restore full-mesh dial, used by
       both `create_network_inner` and `restore_coordinator_network`) was a
       plain serial `for member in members { ...await... }` loop with no
       timeout at all — confirmed by direct read, not inherited from
       upstream's history. Restore time scaled linearly with roster size and
       could stall indefinitely on one dead peer.

    3. `restore_coordinator_network` `.await`ed that entire serial,
       unbounded `dial_all_members` call *before* `self.networks.insert(...)`
       — confirmed by reading the function directly. `tetron status` run in
       that window (routinely triggered right after `sudo tetron restart`)
       reported no active networks at all, even though the config and local
       roster were completely intact, for as long as the slowest/least
       reachable roster member took to resolve.

    Fix, applied together since all three are faces of the same root cause
    (serial + unbounded dialing blocking usability):

    - `connect_to_roster_peers` becomes `spawn_roster_peer_dials`: the
      coordinator/initial peer link is registered synchronously (as before),
      then the rest of the roster dials concurrently in a spawned background
      task (`futures::stream::FuturesUnordered`), each bounded by
      `MESH_PEER_DIAL_TIMEOUT` (30s — generous since it's off the boot path)
      and cancellation-aware via the network's token. The join/reconnect
      completes as soon as the initial link is up; peer links fill in as they
      connect, and the existing reconnect loop recovers any that time out.
    - `dial_all_members` gains the same `FuturesUnordered` concurrency and a
      `DIAL_TIMEOUT` (10s — tighter, since this dial runs proactively on
      every restore regardless of whether a peer will ever answer, and the
      per-peer reconnect loop is the real recovery path either way, not this
      one-shot proactive dial).
    - `restore_coordinator_network` inserts the `NetworkHandle` into
      `self.networks` before the (now backgrounded, non-blocking-in-spirit
      but still practically fast) `dial_all_members` call, so the network is
      visible to `tetron status`/IPC as soon as local restore completes,
      matching the ordering `create_network_inner` already effectively gets
      for free.

    tetron's dials never carry a real `DeviceCert` (device pairing was
    removed by MINIMAL-004; `device_cert: None` is hardcoded at every
    `MeshHello` site already), so unlike the upstream commits this fix
    carries no device-cert plumbing — one parameter fewer throughout.

    Found: 2026-07-16, triaging rayfish commits 02dd60e/fe3f3c0/b26c26b for
    tetron applicability. All three confirmed missing by direct code
    inspection of tetron's current `join.rs`/`create_join.rs`/`runtime.rs`,
    not assumed from upstream history — tetron's fork point and subsequent
    MINIMAL-* rewrites make no guarantee upstream fixes were ever inherited.
    """
    req_id = "DIAL-001"


# --------------------------------------------------------------------------
# DHT-ERRCAUSE-001: join-path DHT resolution error stutter + missing cause
# (upstream 24b6a03 `fix(dht)`, ported from the 2026-08-05 upstream review)
# --------------------------------------------------------------------------

class DhtResolveErrorCause(Requirement):
    """REQUIREMENT-ID: DHT-ERRCAUSE-001

    Every DHT discovery failure during join rendered as
    `failed to resolve network record: failed to resolve network record:
    Service 'pkarr' failed` — the same context wrapped twice, once in
    `dht::resolve_network_packet` (`src/dht.rs`, `map_err(|e| anyhow!(
    "failed to resolve network record: {e}"))`) and again at the call site
    (`src/daemon/mesh/create_join.rs`'s `resolve_and_fetch_blob`, `.context(
    "failed to resolve network record")`). The inner `{e}` formatting also
    throws away the pkarr source chain that holds the real cause — `{e:#}`
    renders it. And there is no total timeout on the resolve: a blackholed
    relay can hang `join` with nothing on screen.

    Fix (ported from upstream `24b6a03`):

    1. `dht::resolve_network_packet` becomes the single wrap: it names the
       discovery server actually used (`dht::effective_pkarr_url()`) and
       renders the full source chain with `{e:#}` — e.g. `failed to
       resolve network record via https://dns.iroh.link/pkarr: ...`.
    2. `create_join.rs`'s `resolve_and_fetch_blob` drops its redundant
       `.context("failed to resolve network record")`, so the error
       propagates wrapped exactly once. No other caller re-wraps it.
    3. `dht::resolve_network_packet` caps the resolve with
       `tokio::time::timeout(RESOLVE_TIMEOUT)` (15s), mapping the timeout
       to a message that names the server and the bound — a blackholed
       relay fails fast with a diagnosable error instead of hanging join.

    `dht.rs`'s existing tests (`effective_url_defaults_when_unset`,
    `network_record_roundtrip`, ...) are unaffected; the timeout/error-
    formatting path is verified by `cargo build`/`clippy` and the
    testsuite (the resolve itself needs a live client, same precedent as
    PATH-DIAG-001's log lines). Small, high-diagnosability, squarely the
    "one thing" (join) done well.

    Independent of INVITE-CHECKSUM-001, TUN-SENDERCACHE-001, and
    IPV4-MIN-IHL-001: disjoint files, no shared state. May land in any
    order.

    Found: 2026-08-05, upstream rayfish review `a56b4b9..b002168`
    (`DO-NOT-COMMIT/REVIEW_upstream-rayfish_2026-08-05.md`, item 2).
    """
    req_id = "DHT-ERRCAUSE-001"


# --------------------------------------------------------------------------
# CONVERGE-007: a kick-coded connection close never mutates the roster
# --------------------------------------------------------------------------

class CloseCodeNeverMutatesRoster(Requirement):
    """REQUIREMENT-ID: CONVERGE-007

    Found triaging the applicable slice of upstream rayfish commit 1c193b9
    (most of that commit — status device-grouping, `RequestUnpair` — is N/A
    for tetron, since device pairing was removed by MINIMAL-004) against
    tetron's current `forward.rs`/`coordinator.rs`. Confirmed present by direct
    inspection, not assumed from upstream.

    `DisconnectEvent.intentional` was computed `true` for *both* `LEAVE_CODE`
    and `KICK_CODE` (`forward.rs:314-319`), and `coordinator.rs`'s
    `spawn_peer_cleanup` treated `intentional == true` as authority to prune
    the canonical roster (`st.members.remove(&member_id)`). But
    `prune_departed_peers` (CONVERGE-005's territory) closes a connection with
    `KICK_CODE` on *every* node, coordinator or not, whenever its own local
    roster momentarily doesn't list the peer on the other end — including
    during an ordinary, still-resolving convergence race, not just a real
    kick. If that peer happens to be the coordinator's own link to a
    genuinely-still-valid member (a transient reconverge race, exactly the
    class CONVERGE-005 narrows but does not fully eliminate — the
    same-generation-tie window is explicitly left unresolved), the
    coordinator's cleanup handler saw the `KICK_CODE` close, computed
    `intentional = true`, and pruned that real member from its own roster and
    republished — a false eviction, driven by connection-close inference
    instead of the signed record. Worse, thanks to CONVERGE-003, the
    mistakenly-pruned member would now promptly and cleanly *leave* on
    receiving that wrongly-updated blob — CONVERGE-003 makes a bogus eviction
    complete faster and more silently than before that fix, since there is no
    longer a stuck "ghost" state to notice and investigate.

    tetron's actual, coordinator-authoritative kick path
    (`remove_member_roster_only` + `finalize_removal` in `coordinator.rs`) was
    never the problem — it already mutates the roster directly as a real
    decision, then closes the victim's connection with `KICK_CODE` as a
    consequence, not a cause. The bug was a second, redundant, and incorrect
    path to the same roster mutation, reachable from mere connection-close
    observation on *any* node running `prune_departed_peers` — not the actual
    kick command.

    Fix: replace `DisconnectEvent.intentional: bool` with a `CloseReason`
    enum (`Left` / `Kicked` / `Other`) and a `prunes_member()` helper that is
    `true` only for `Left`. `coordinator.rs`'s cleanup now prunes the roster
    only on `Left`; a `Kicked` (or `Other`) close just stamps `last_seen`,
    matching the existing non-intentional-drop branch. `join.rs`'s reconnect
    loop narrows its "peer left, not reconnecting" skip to `Left` only,
    letting a `Kicked` close fall through to the existing `pruned_peers` check
    immediately below it — which is *already* the correct, signed-roster-
    driven arbiter (populated only by `prune_departed_peers` after a verified
    reconverge, never by raw close-code inference) for whether to actually
    stop reconnecting. This ties every reconnect-suppression and every roster
    mutation to the signed record, never to a bare close code, continuing the
    "generation/signed record is the only source of truth" principle
    CONVERGE-005 established for publishing.

    The synthetic disconnect event `dial_reconnect` sends per member on a
    cold restore (no live connection yet, used only to force the reconnect
    loop's first dial attempt) maps to `CloseReason::Other` — it was never a
    leave or a kick, just a kick-start (pun unintended) for the dial loop.

    Found: 2026-07-16, triaging rayfish 1c193b9 for tetron applicability.
    """
    req_id = "CONVERGE-007"


# --------------------------------------------------------------------------
# CONVERGE-008: no unconditional "first publish" bypass -- always
# read-before-write, even on a coordinator's very first publish attempt
# --------------------------------------------------------------------------

class NoUnconditionalFirstPublish(Requirement):
    """REQUIREMENT-ID: CONVERGE-008

    `dht_read_before_write` (CONVERGE-005's generation-authoritative publish
    guard) had a bypass: `if last_published.is_none() { return true; }` --
    a caller's very first publish attempt (no locally-tracked prior publish
    yet) always proceeded unconditionally, skipping the DHT comparison
    entirely. This was meant for the genuinely-new-network case (nothing to
    compare against), which the guard's own `Err` arm (no DHT record found)
    already handles correctly on its own -- the bypass was never actually
    load-bearing for that case.

    What it was actually doing, unintentionally: `seal_and_publish` (shared
    by `create_network_inner` and `restore_coordinator_network`) calls
    `dht::publish_network` directly -- with no read-before-write guard at
    all -- immediately at restore time, before the periodic publisher loop
    (which does have the guard, but only after its own first-iteration
    bypass) even starts. `restore_member_roster` falls back to stale local
    config when the DHT/blob is unreachable at restart ("could not restore
    roster from DHT blob; falling back to config"). Combine the two: a
    coordinator restarting under flaky DHT connectivity restores a
    possibly-stale roster, then `seal_and_publish` unconditionally
    republishes it, potentially overwriting a concurrently-mutated (or even
    already-nuked, see NUKE-CONSENSUS) DHT record with old, wrong content.

    Fix: removed the bypass from `dht_read_before_write` entirely (now
    `pub(crate)`, no `last_published` parameter -- it always does the real
    generation/hash comparison, with the existing `Err` arm still covering
    "nothing published yet"). `seal_and_publish` now goes through the same
    guard before its `dht::publish_network` call, instead of calling it
    unconditionally; if the guard defers, the (already generation-authoritative)
    group poller picks up the real current state on its next tick. For a
    genuinely brand-new network this adds one harmless extra `resolve_network`
    round-trip (always `Err`, guard passes). `spawn_network_publisher`'s
    `last_published` local variable is now unused for gating (removed);
    `spawn_lazy_publisher` keeps its own `last_published` check as a
    separate, still-valid optimization (skip even attempting a DHT
    round-trip when the local hash hasn't changed) -- that check is
    independent of what the guard itself does internally.

    This does not touch the established pattern used by one-shot,
    fresh-local-mutation publishes (`invite_create`, `invite_revoke`, kick,
    `admin_add`'s `store_and_publish_group`) -- those correctly publish
    unconditionally because they *are* the authoritative new state (a local
    mutation just happened), unlike a restore, which may or may not reflect
    reality depending on whether the DHT fetch that fed it actually
    succeeded.

    Found: 2026-07-17, as a side effect of NUKE-CONSENSUS live testing --
    repeated manual daemon restarts (for redeploying binaries mid-test) on
    the original coordinator collided with this, resurrecting a stale
    record and getting the node stuck comparing against its own
    resurrected write rather than the real state. Deliberately deferred out
    of the NUKE-CONSENSUS commit (needed its own scoped fix + live
    validation, not a tail-end change to an already-large feature PR).

    Live validation needed before trusting this in production, same bar as
    CONVERGE-001/NUKE-CONSENSUS: restart a coordinator with the DHT/blob
    deliberately blocked (falls back to stale config), verify it does not
    clobber a concurrently-mutated (or nuked) record once connectivity
    returns.
    """
    req_id = "CONVERGE-008"


class PrunedPeersPeriodicGc(Requirement):
    """REQUIREMENT-ID: CONVERGE-009

    `pruned_peers` (the `Arc<DashSet<(String, EndpointId)>>` CONVERGE-007
    introduced as the signed-roster-driven arbiter for reconnect
    suppression) is populated by `prune_departed_peers`
    (`reconverge.rs:215`, one insert per peer no longer in the verified
    roster) and consumed exactly once, by the reconnect loop's disconnect
    handler (`join.rs:907`, `pruned_peers.remove(...)`) -- but only on the
    branch that actually reaches that line. Two earlier `continue`s in the
    same handler skip it entirely: `!removed` (a stale disconnect event for
    a connection already superseded by a fresh dial) and
    `event.reason.prunes_member()` (a deliberate `tetron leave`). An entry
    inserted for a peer whose disconnect event lands on either of those
    branches is never removed -- there is no periodic sweep anywhere in the
    daemon that revisits `pruned_peers` independent of that one consumer
    path.

    Found triaging the 2026-08-02 memory-leak audit (Finding #3,
    `DO-NOT-COMMIT/tetron_memleak.md`); confirmed present by direct
    inspection against `main` 2026-08-06 -- grepped for any interval/GC
    task touching `pruned_peers`, found none.

    In practice this is narrow (most disconnects do reach the removal
    line) and each leaked entry is small (one `(String, EndpointId)` per
    stuck tuple), but it is still unbounded over the lifetime of a
    long-running, churny mesh -- the same "should shrink, never does"
    shape as CACHE-002, just a different subsystem.

    Fix: a periodic sweep (piggybacked on the same cadence as an existing
    daemon-wide periodic task rather than a new dedicated interval) that
    drops any `pruned_peers` entry whose network name is no longer present
    in `self.networks` -- a peer pruned from a network the daemon has since
    left or torn down can never be consumed by that network's reconnect
    loop again, since the loop itself is gone. This does not touch the
    happy-path removal in `join.rs:907`, which stays the primary,
    immediate consumer; the sweep only catches what that path's two
    `continue` branches leave behind.

    Independent of CACHE-002 -- different subsystem (roster-prune
    suppression vs. peer address persistence), no shared state.
    """
    req_id = "CONVERGE-009"


class LeaveAcceptsNetworkKey(Requirement):
    """REQUIREMENT-ID: LEAVE-NETWORK-KEY-001

    `tetron leave` previously resolved its network argument only by
    exact match against the local display name (`self.networks.get`,
    a plain map lookup, no dedicated resolver) -- unlike `nuke`/`kick`,
    which both resolve by network key. A user who only has the invite
    key or room id handy (e.g. at uninstall time, having never noted
    the locally-assigned display name) had no way to `leave` at all.

    Fix: new `MeshManager::resolve_network_name_or_key` (`src/daemon/
    mod.rs`), tried at the top of `leave_network`
    (`src/daemon/mesh/runtime.rs`) before any of its existing logic --
    every downstream use of the network argument (the sole-coordinator
    check, connection teardown, config removal, response messages) now
    operates on the resolved local name either way, so behavior for the
    existing local-name path is unchanged byte-for-byte. Tries the exact
    local name first (preserves today's only path untouched); falls back
    to `resolve_network_short_id` (same >=10-char-minimum, ambiguous-
    prefix-rejected rules already used by `nuke`/`kick`) only if that
    fails. Deliberately **not** the same trust posture as `nuke`/`kick`'s
    key-only resolution: `leave` only ever tears down the caller's own
    participation, never mutates another node's roster, so there is no
    destructive-action argument for refusing a local-name match the way
    `resolve_network_short_id`'s own doc comment explains for those two.
    On failure, the combined error names both things that were tried
    (not a known local name, and not a valid/unambiguous key) rather
    than surfacing `resolve_network_short_id`'s raw wording, which
    assumes -- correctly for `nuke`/`kick`, not for `leave` -- that the
    caller was attempting key resolution in the first place.

    `--help`, `AGENTS.md`, `README.md`, and `docs/HOWTO.md` updated to
    document the fallback.

    Follow-up (`INVITE-ADMIN-NETWORK-KEY-001`, below): `invite`/`admin`
    gained the identical fallback via this same resolver.
    """
    req_id = "LEAVE-NETWORK-KEY-001"


class InviteAdminAcceptNetworkKey(Requirement):
    """REQUIREMENT-ID: INVITE-ADMIN-NETWORK-KEY-001

    `tetron invite <net> ...` (`create`/`list`/`revoke`) and `tetron admin
    <net> ...` (`add`/`list`) previously resolved their network argument
    only by exact match against the local display name (`self.networks.get`,
    the same plain map lookup `leave` used before `LEAVE-NETWORK-KEY-001`) --
    a user who only has the invite key or room id handy had no way to mint
    an invite or grant admin at all, unlike `leave`, which already gained
    the key-prefix fallback.

    Fix: all five handlers (`invite_create`/`invite_list`/`invite_revoke` in
    `src/daemon/mesh/invite_handler.rs`; `admin_add`/`admin_list` in
    `src/daemon/mesh/admin.rs`) now call the existing
    `MeshManager::resolve_network_name_or_key` at the very top, shadowing
    the `network` parameter with the resolved local name before any
    existing logic runs -- identical placement to `leave_network`
    (`LEAVE-NETWORK-KEY-001`) and `grant_admin_key`'s other caller
    (`leave_network`'s `STRANDED-COORDINATOR-WARN` auto-promotion, which
    already passed an already-resolved name, so it is unaffected). No new
    resolver was needed -- `resolve_network_name_or_key` already tries the
    exact local name first (today's only path, unchanged) and falls back to
    `resolve_network_short_id`'s `>=10`-char/ambiguous-prefix rules only on
    a miss, same non-destructive trust posture as `leave` (invite/admin
    grant capability, they never mutate anyone else's membership, so there
    is no destructive-action case for a key-only rule the way `nuke`/`kick`
    have).

    This makes `CLI-VOCAB-002`'s docstring note ("the same lookup
    `leave`/`invite`/`admin` still use") historical for `invite`/`admin` as
    of this requirement -- accurate as of 2026-07-17, no longer accurate as
    of this change. `nuke`/`kick` remain deliberately unchanged (key-only,
    no name fallback at all -- the destructive-action argument in
    `CLI-VOCAB-002` still applies to them specifically).

    `--help` text for `Invite.network`/`Admin.network` (`src/main.rs`)
    updated to match `Leave.network`'s wording, documenting the fallback.
    """
    req_id = "INVITE-ADMIN-NETWORK-KEY-001"


class LeaveRemovesStuckNetwork(Requirement):
    """REQUIREMENT-ID: LEAVE-STUCK-NETWORK-001

    Found 2026-07-20: a network whose restore fails outright (DHT/blob
    unreachable, no config fallback either) never gets a `self.networks`
    entry -- but both of `leave_network`'s resolution paths
    (`resolve_network_name_or_key`'s exact-name check and its
    `resolve_network_short_id` key-prefix fallback) only ever scan
    `self.networks`. So a network stuck in this state had no CLI path to
    remove it at all -- the only workaround was deleting
    `networks/<name>.toml` directly as root.

    Fix: `leave_network` (`src/daemon/mesh/runtime.rs`) falls back to an
    exact match against the persisted `NetworkConfig` (`config::
    load_network`) when the live-only resolver fails, before giving up.
    Deliberately exact-name only, no key-prefix fallback added here: with no
    live roster there's nothing to resolve a prefix against safely, and this
    path only ever acts on the caller's own already-broken config entry, so
    the exact local name (as shown by `tetron status`'s "saved networks"
    listing when the daemon can't reach it) is enough. Every step below the
    resolution point (`teardown_network_runtime`, the sole-coordinator
    stranding check) already degrades gracefully to a no-op when the network
    isn't in `self.networks` -- this was purely a resolution-layer gap, not
    a teardown-logic one, so no changes were needed there.
    """
    req_id = "LEAVE-STUCK-NETWORK-001"


class InviteListRevokedNotUsed(Requirement):
    """REQUIREMENT-ID: INVITE-STATUS-001

    Found live bug-hunting after `SUBNET-UNIQUE-001`: `tetron invite <net>
    revoke <id>` followed by `tetron invite <net> list` showed the just-
    revoked invite's status as `used` -- indistinguishable from one someone
    had actually redeemed. `InviteInfo.used` (`tetron-proto/src/ipc.rs`) was
    populated as `used: entry.revoked` (`src/daemon/mesh/invite_handler.rs`),
    with a comment claiming "revoked flag means consumed."

    That's not just misleadingly named -- `InviteEntry` has no field that
    could ever represent "actually redeemed" in the first place. An invite
    that's genuinely used is removed from the blob entirely on successful
    redemption (`src/daemon/mesh/accept.rs`'s "burn the invite" step), so
    it's never listed again at all once that happens. The only thing
    `InviteEntry.revoked` can ever mean, for any entry still present to
    list, is "an admin explicitly revoked this" -- calling it `used` claimed
    a distinction (redeemed vs. cancelled) the data model was never capable
    of drawing, and actively misled anyone auditing which invites were
    manually revoked vs. genuinely consumed by a joiner.

    **Fix:** renamed `InviteInfo.used` -> `revoked` (wire field), the
    daemon's construction site, `tetron invite list`'s `--json` key and text
    `status` column (now prints `revoked` instead of `used` for that case;
    `active`/`expired` unchanged), and `docs/HOWTO.md`'s `jq` example.
    """
    req_id = "INVITE-STATUS-001"
