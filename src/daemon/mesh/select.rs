//! Pure decision helpers for the join/gossip paths: coordinator dial order,
//! gossip targets, dial-fallback outcome selection, persisted-roster fallback,
//! connection-path classification, and subnet-collision detection. No I/O, so
//! these are unit-tested directly (see the tests in `daemon/mod.rs`).

use super::super::*;

/// Compute the order in which a joiner should dial coordinators.
/// Returns the minter first (if present and not `me`), then every other
/// `is_coordinator` member except `me`, de-duplicated, preserving order.
/// Consumed by the join dial-fallback loop.
pub(crate) fn coordinator_dial_order(
    minter: EndpointId,
    members: &[Member],
    me: EndpointId,
) -> Vec<EndpointId> {
    let mut order = Vec::new();
    let is_coord = |id: EndpointId| members.iter().any(|m| m.identity == id && m.is_coordinator);
    if minter != me && is_coord(minter) {
        order.push(minter);
    }
    for m in members {
        if m.is_coordinator && m.identity != me && !order.contains(&m.identity) {
            order.push(m.identity);
        }
    }
    order
}

/// RESTORE-COORD-FALLTHROUGH-001: result of one coordinator attempt during a
/// boot-restore walk. `Joined` carries whatever the caller needs to finish
/// (the handshake's live state), so the walker stays free of I/O types.
pub(crate) enum RestoreAttempt<T> {
    /// This coordinator completed the handshake.
    Joined(T),
    /// This coordinator queued the request for operator approval: stop here,
    /// as on a fresh join.
    Pending,
    /// Connect failed or the handshake errored/timed out: try the next one.
    Failed,
}

/// RESTORE-COORD-FALLTHROUGH-001: how a restore walk ended.
pub(crate) enum RestoreWalk<T> {
    Joined(T),
    Pending,
    /// No coordinator completed (all failed, budget spent, or none to try).
    /// The caller registers the network from the verified blob.
    Exhausted,
}

/// RESTORE-COORD-FALLTHROUGH-001: walk `order` (see [`coordinator_dial_order`])
/// calling `attempt` for each coordinator until one joins. No new attempt is
/// started once `budget` has elapsed since the walk began; an attempt already in
/// flight finishes within its own bounds. `attempt` is expected to bound its own
/// connect and handshake (`DIAL_TIMEOUT`, `RESTORE_HANDSHAKE_TIMEOUT`).
pub(crate) async fn walk_restore_coordinators<T, F, Fut>(
    order: &[EndpointId],
    budget: Duration,
    mut attempt: F,
) -> RestoreWalk<T>
where
    F: FnMut(EndpointId) -> Fut,
    Fut: std::future::Future<Output = RestoreAttempt<T>>,
{
    let started = std::time::Instant::now();
    for (i, coordinator) in order.iter().enumerate() {
        if i > 0 && started.elapsed() >= budget {
            break;
        }
        match attempt(*coordinator).await {
            RestoreAttempt::Joined(t) => return RestoreWalk::Joined(t),
            RestoreAttempt::Pending => return RestoreWalk::Pending,
            RestoreAttempt::Failed => {}
        }
    }
    RestoreWalk::Exhausted
}

/// BLOB-FETCH-BOUND-001: try each of `items` in order, giving each at most
/// `per_item`, and return the first `Some`. A stalled or failing item is
/// skipped. No new item is started once `budget` has elapsed since the walk
/// began (an attempt already running still ends within `per_item`). Used by
/// `fetch_verified_blob` so one dead peer cannot block a roster fetch.
pub(crate) async fn first_some_bounded<I, T, F, Fut>(
    items: &[I],
    per_item: Duration,
    budget: Duration,
    mut f: F,
) -> Option<T>
where
    F: FnMut(&I) -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let started = std::time::Instant::now();
    for (i, item) in items.iter().enumerate() {
        if i > 0 && started.elapsed() >= budget {
            break;
        }
        if let Ok(Some(found)) = tokio::time::timeout(per_item, f(item)).await {
            return Some(found);
        }
    }
    None
}

/// GRANT-PUBLISH-001: what a grantor should do with the roster it just flagged,
/// given what the DHT currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrantPublish {
    /// Our record is newer than the DHT's (or the DHT has none): publish it.
    Publish,
    /// The DHT already holds exactly this record: nothing to write.
    AlreadyPublished,
    /// The DHT is ahead of us, or tied with different content: our flag would
    /// be ignored or overwritten. Adopt the DHT's roster, re-assert the flag on
    /// top of it, and go round again.
    Reconverge,
}

/// GRANT-PUBLISH-001: decide from our `(generation, hash)` and the DHT's
/// `(generation, hash)` (`None` when it has no record). Mirrors
/// `dht_read_before_write`, but keeps "already identical" apart from "ahead of
/// us" so a grant can reconverge and retry instead of giving up.
pub(crate) fn grant_publish_action(
    local_generation: u64,
    local_hash: blake3::Hash,
    dht: Option<(u64, blake3::Hash)>,
) -> GrantPublish {
    match dht {
        None => GrantPublish::Publish,
        Some((dht_generation, dht_hash)) => match local_generation.cmp(&dht_generation) {
            std::cmp::Ordering::Greater => GrantPublish::Publish,
            std::cmp::Ordering::Equal if dht_hash == local_hash => GrantPublish::AlreadyPublished,
            _ => GrantPublish::Reconverge,
        },
    }
}

/// Outcome of a single coordinator dial attempt during the join fallback loop.
/// Used as a unit-testable specification of the loop termination policy.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(dead_code)]
pub(crate) enum DialOutcome {
    Welcomed,
    Denied,
    Unreachable,
}

/// Returns `(index_of_last_tried, welcomed)`.
/// Iterates `outcomes` left-to-right and stops at the first `Welcomed`.
/// If none is found, returns the index of the last element and `false`.
#[allow(dead_code)]
pub(crate) fn pick_first_welcome(outcomes: &[DialOutcome]) -> (usize, bool) {
    for (i, o) in outcomes.iter().enumerate() {
        if *o == DialOutcome::Welcomed {
            return (i, true);
        }
    }
    (outcomes.len().saturating_sub(1), false)
}

/// Last-known roster from persisted config. Used only as a fallback when the
/// signed pkarr record is briefly unreachable during a reconnect — never trusts
/// peer-supplied membership.
pub(crate) fn persisted_roster(network_name: &str) -> Vec<Member> {
    config::load()
        .ok()
        .and_then(|c| c.networks.into_iter().find(|n| n.name == network_name))
        .map(|n| {
            n.members
                .into_iter()
                .map(|m| Member {
                    identity: m.identity,
                    ip: m.ip,
                    is_coordinator: m.is_coordinator,
                    hostname: m.hostname,
                    collision_index: 0,
                    last_seen: None,
                    // `config::schema`'s persisted member entry doesn't carry
                    // this (unlike the live signed-blob `Member`) -- this
                    // fallback path is a degraded reconstruction already
                    // (collision_index/last_seen are lost too).
                    veilid_node_id: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Rebuild a network's DNS entries from its member roster (the single source of
/// truth) and persist our own — possibly coordinator-corrected — hostname. Called
/// whenever a roster update arrives so renames, joins, and departures all reflect
/// in `*.ray` resolution immediately.
/// Classifies one candidate path's address into its `ConnType` and whether
/// it is trustworthy (PATHBLEED-STATUS-003, corrected). `Relay`/`Tor`
/// addresses are not scoped to any tetron network (relay/discovery config
/// is daemon-wide) -- always trustworthy. A `Direct`/IP candidate's address
/// is trustworthy when it falls inside **none** of `managed_subnets` (v4)/
/// `managed_network_keys`' derived prefixes (v6) -- every overlay
/// subnet/network this *daemon* manages, not only the network currently
/// being queried. A genuine real transport address (a LAN or public IP)
/// will never coincidentally match any of them. A candidate that *does*
/// match one is a self-captured/bled overlay address: iroh's own
/// local-interface enumeration can pick up this daemon's TUN device and
/// offer a peer's overlay IP on a *different* one of its shared networks as
/// a "direct" candidate (`SELFCAPTURE-ROUTE-001` mitigates iroh's own
/// outbound use of this; it does not remove the candidate from
/// `Connection::paths()`) -- exactly the address shape
/// `DO-NOT-COMMIT/RESULTS_PathBleed_DataLossTest.md`'s live reproduction
/// recorded (`10.88.1.147`, a peer's own overlay address on its *other*
/// network). Checking against every managed network, not just the one
/// being queried, is what catches this -- checking only the current
/// network's own subnet (the original `PATHBLEED-STATUS-001` design)
/// rejects genuine candidates almost universally instead (they are never
/// inside *any* overlay subnet, so restricting the check to one specific
/// subnet fails for the same reason unconditional `true` overcorrected:
/// see `PATHBLEED-STATUS-003`'s own docstring for the full account of both
/// mistakes).
pub(crate) fn classify_candidate_addr(
    addr: &iroh::TransportAddr,
    managed_subnets: &[crate::membership::Subnet],
    managed_network_keys: &[EndpointId],
) -> (ipc::ConnType, bool) {
    if addr.is_relay() {
        return (ipc::ConnType::Relay, true);
    }
    #[cfg_attr(not(feature = "veilid"), allow(unused_variables))]
    if let iroh::TransportAddr::Custom(custom) = addr {
        // VEILID-007 follow-up: this predates VEILID-002 and defaulted
        // every custom transport to Tor regardless of which one it
        // actually was -- harmless while Tor was the only one, wrong
        // once Veilid shipped a second. `tetron status` was live-verified
        // reporting a real, active Veilid path as `conn_type: "Tor"`
        // (`remote_addr` gave it away: a hex-encoded `veilid_...` custom
        // address id). Falls back to Tor for anything else, matching the
        // original behavior for an actual Tor address (and any future
        // custom transport not yet given its own arm here).
        #[cfg(feature = "veilid")]
        if custom.id() == veilid_transport::VEILID_TRANSPORT_ID {
            return (ipc::ConnType::Veilid, true);
        }
        return (ipc::ConnType::Tor, true);
    }
    let looks_self_captured_or_bled = match addr {
        iroh::TransportAddr::Ip(socket_addr) => match socket_addr.ip() {
            std::net::IpAddr::V4(v4) => managed_subnets
                .iter()
                .any(|&subnet| crate::membership::ip_in_subnet(v4, subnet)),
            std::net::IpAddr::V6(v6) => managed_network_keys
                .iter()
                .any(|key| crate::membership::ipv6_in_network(v6, key)),
        },
        _ => false,
    };
    (ipc::ConnType::Direct, !looks_self_captured_or_bled)
}

/// Pick which connection path to report in `tetron status`. Prefers the path
/// iroh has selected; otherwise falls back to the best concrete path so a
/// live connection never renders as `Unknown` (`?`). Priority Direct > Relay
/// > Tor.
///
/// Returns the index into `classes`, or `None` when there are no paths, or
/// (PATHBLEED-STATUS-001/`-003`) when every path is untrustworthy.
///
/// Each candidate is `(conn_type, is_selected, in_subnet, has_activity)`.
/// `in_subnet` comes from `classify_candidate_addr` above. A candidate with
/// `in_subnet == false` is excluded from both the `is_selected()`-preference
/// check and the fallback scan below -- not just stripped of its selected
/// flag, since the fallback loop matches by classification alone and would
/// otherwise re-surface the same wrong address a moment later.
///
/// `has_activity` (PATHBLEED-STATUS-002, a hardening layer on top of the
/// subnet check) is whether the path's own `stats()` show real *received*
/// traffic (`PATHBLEED-STATUS-003` corrected this from transmitted to
/// received -- a path's own attempted-but-unvalidated `PATH_CHALLENGE`
/// probe already counts as transmitted activity, so only receipt actually
/// proves the path works) -- a freshly-opened, never-actually-validated
/// candidate reads as zero even while `is_selected()` claims it and even
/// while it's in-subnet. The residual case this actually still catches,
/// now that `in_subnet` (`PATHBLEED-STATUS-003`) checks every managed
/// subnet directly rather than only the currently-queried network's own: a
/// bled candidate whose address is the peer's own overlay address on a
/// network *they* belong to but *this daemon* does not -- it never appears
/// in `managed_subnets` at all, so the subnet check can't exclude it, and
/// `has_activity` is the only remaining defense (documented as a residual
/// gap in `PATHBLEED-STATUS-003`'s own docstring). Three tiers, each
/// restricted to `in_subnet` candidates only:
/// 1. Selected *and* (active or the sole trustworthy candidate) -- the
///    strongest signal, trusted outright.
/// 2. No tier-1 winner: prefer any candidate with real activity, by class
///    (Direct > Relay > Tor > Veilid), *regardless* of `is_selected()` -- corroborated
///    real traffic outranks a bare selected label or classification alone.
/// 3. Nothing has proven itself yet: fall back to plain classification among
///    all trustworthy candidates, so a genuinely new, still-validating path
///    is still reported rather than hidden as `?` just for being new.
pub(crate) fn choose_path_index(classes: &[(ipc::ConnType, bool, bool, bool)]) -> Option<usize> {
    let trustworthy_count = classes
        .iter()
        .filter(|(_, _, in_subnet, _)| *in_subnet)
        .count();

    if let Some(i) = classes
        .iter()
        .position(|(_, selected, in_subnet, has_activity)| {
            *selected && *in_subnet && (*has_activity || trustworthy_count == 1)
        })
    {
        return Some(i);
    }

    for want in [
        ipc::ConnType::Direct,
        ipc::ConnType::Relay,
        ipc::ConnType::Tor,
        ipc::ConnType::Veilid,
    ] {
        if let Some(i) = classes
            .iter()
            .position(|(ct, _, in_subnet, has_activity)| *ct == want && *in_subnet && *has_activity)
        {
            return Some(i);
        }
    }

    for want in [
        ipc::ConnType::Direct,
        ipc::ConnType::Relay,
        ipc::ConnType::Tor,
        ipc::ConnType::Veilid,
    ] {
        if let Some(i) = classes
            .iter()
            .position(|(ct, _, in_subnet, _)| *ct == want && *in_subnet)
        {
            return Some(i);
        }
    }

    // A path with no IP/relay/custom classification (none today), or the
    // only remaining trustworthy one isn't among the three known classes
    // (shouldn't happen), or -- deliberately -- every candidate is
    // untrustworthy: report unknown rather than a definitely-wrong address.
    classes.iter().position(|(_, _, in_subnet, _)| *in_subnet)
}

/// PATH-DIAG-004: explains *why* `winner_conn_type` isn't `ipc::ConnType::Direct`,
/// derived from the same `classes` data `choose_path_index` above consumes.
/// `None` when the winner *is* Direct -- nothing to explain.
///
/// Corrected 2026-08-02 while implementing this (see `PATH-DIAG-004`'s own
/// spec docstring for the full story): an in-subnet, active Direct
/// candidate always wins by `choose_path_index`'s own tier 2 regardless of
/// `is_selected`, so "selected? / not yet selected?" is never actually the
/// distinguishing factor here. The two real reasons are whether a Direct
/// candidate exists in-subnet at all (just unvalidated, PATHBLEED-STATUS-002)
/// versus existing only out-of-subnet (bled, PATHBLEED-STATUS-001) versus
/// not existing at all.
pub(crate) fn classify_via_detail(
    classes: &[(ipc::ConnType, bool, bool, bool)],
    winner_conn_type: &ipc::ConnType,
) -> Option<ipc::ViaDetail> {
    if *winner_conn_type == ipc::ConnType::Direct {
        return None;
    }
    let has_in_subnet_direct = classes
        .iter()
        .any(|(ct, _, in_subnet, _)| *ct == ipc::ConnType::Direct && *in_subnet);
    if has_in_subnet_direct {
        return Some(ipc::ViaDetail::DirectUnvalidated);
    }
    let has_bled_direct = classes
        .iter()
        .any(|(ct, _, in_subnet, _)| *ct == ipc::ConnType::Direct && !*in_subnet);
    if has_bled_direct {
        return Some(ipc::ViaDetail::DirectBled);
    }
    Some(ipc::ViaDetail::NoDirectCandidate)
}

/// SUBNET-COLLISION-001: returns the first subnet in `existing` that overlaps
/// `candidate`, or `None` if there is no collision. Shared by `create`'s
/// explicit-`--subnet` check and `join`'s network-subnet check -- both build
/// their own context-specific error message and `--force` handling around
/// this, since the wording differs (creating vs. joining) but the underlying
/// question does not.
pub(crate) fn find_subnet_collision(
    candidate: crate::membership::Subnet,
    existing: &[crate::membership::Subnet],
) -> Option<crate::membership::Subnet> {
    existing
        .iter()
        .copied()
        .find(|&e| crate::membership::subnets_overlap(e, candidate))
}

/// Outcome of `tetron nuke`'s solo-coordinator branch (NUKE-CONSENSUS,
/// PURE-LOGIC-001): a network with only one coordinator has no one to
/// second, so it either nukes immediately or is refused outright -- there
/// is no propose/consensus path. Pure specification of that branch's three
/// outcomes, extracted from `runtime.rs::nuke_network`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SoloNukeOutcome {
    /// `--cancel` or `--second` was passed, but there is no consensus in
    /// play with a single coordinator -- nothing to cancel or second.
    NothingToCancelOrSecond,
    /// Other members exist and `--force` was not passed; refuse rather
    /// than strand them.
    WouldStrandMembers,
    /// Nuke immediately.
    Proceed,
}

pub(crate) fn solo_coordinator_nuke_outcome(
    cancel: bool,
    second_present: bool,
    has_other_members: bool,
    force: bool,
) -> SoloNukeOutcome {
    if cancel || second_present {
        return SoloNukeOutcome::NothingToCancelOrSecond;
    }
    if has_other_members && !force {
        return SoloNukeOutcome::WouldStrandMembers;
    }
    SoloNukeOutcome::Proceed
}

/// Whether a just-received `Welcome` roster already assigns `my_ip` to a
/// different identity than `my_identity` (PURE-LOGIC-001) -- an IP-hijack/
/// collision that must abort the join rather than proceed with a
/// conflicting address. Returns the colliding member's identity, if any.
/// Extracted from `join.rs::perform_join_handshake`.
pub(crate) fn welcome_ip_collision(
    members: &[Member],
    my_ip: Ipv4Addr,
    my_identity: EndpointId,
) -> Option<EndpointId> {
    members
        .iter()
        .find(|m| m.ip == my_ip && m.identity != my_identity)
        .map(|m| m.identity)
}

/// Resolve the authoritative `my_ip` from a just-received `Welcome` roster
/// (MULTISEG-009, PURE-LOGIC-001). The coordinator's admission path
/// (`accept.rs::validate_admission`) already calls `assign_ip` and ignores
/// the joiner's own pre-dial guess, so `members` already carries the
/// joiner's real, collision-resolved IP under its own identity -- prefer
/// that over the guess instead of comparing the guess against the roster
/// and bailing on a mismatch. Falls back to [`welcome_ip_collision`]'s
/// guess-vs-roster check only when the roster has no entry for
/// `my_identity` at all (should not happen: `Welcome` is only sent after
/// the coordinator has already added the joiner to its own roster).
/// `Err` carries the still-colliding identity, for the caller's error
/// message.
pub(crate) fn resolve_welcome_self_ip(
    members: &[Member],
    my_ip: Ipv4Addr,
    my_identity: EndpointId,
) -> Result<Ipv4Addr, EndpointId> {
    if let Some(mine) = members.iter().find(|m| m.identity == my_identity) {
        return Ok(mine.ip);
    }
    match welcome_ip_collision(members, my_ip, my_identity) {
        Some(existing) => Err(existing),
        None => Ok(my_ip),
    }
}

/// Doubles `current` for the next reconnect attempt, capped at `max`
/// (exponential backoff, PURE-LOGIC-001). Extracted from
/// `join.rs::spawn_reconnect_loop`'s per-peer retry loop for direct unit
/// testing, independent of that loop's own tokio/async machinery.
pub(crate) fn next_backoff(
    current: std::time::Duration,
    max: std::time::Duration,
) -> std::time::Duration {
    (current * 2).min(max)
}

/// The backoff cap in effect for the next reconnect attempt (CONVERGE-011,
/// CONVERGE-013, PURE-LOGIC-001): the warm cap (`BACKOFF_MAX`, 30s) until
/// `cold_threshold` consecutive attempts have failed, the cold cap after
/// that, and the frozen cap once `frozen_threshold` is reached (a third,
/// even slower floor -- CONVERGE-013 -- for a peer that has been retried at
/// the cold cadence for a long time, roughly a day by the compiled
/// defaults). `next_backoff`'s doubling then climbs naturally toward
/// whichever cap is in effect, so escalation has no cliff at any tier. A
/// still-listed but long-offline roster member ("zombie") is retried
/// forever either way -- this bounds the outbound dial churn against it,
/// not membership: the frozen tier is a floor, deliberately not a full
/// stop, since a hard stop on one side risks a mutual deadlock against a
/// peer that stays running but network-partitioned longer than any cutoff
/// (see CONVERGE-013's docstring).
///
/// Checked from most- to least-escalated: a degenerate config where
/// `frozen_threshold <= cold_threshold` reaches the frozen cap directly
/// once `frozen_threshold` attempts have failed, without ever observing
/// the cold cap in between -- there is no meaningful "correct" behavior to
/// preserve for that misconfiguration, so the simplest check order wins.
pub(crate) fn backoff_cap(
    failed_attempts: u32,
    cold_threshold: u32,
    frozen_threshold: u32,
    warm_max: std::time::Duration,
    cold_max: std::time::Duration,
    frozen_max: std::time::Duration,
) -> std::time::Duration {
    if failed_attempts >= frozen_threshold {
        frozen_max
    } else if failed_attempts >= cold_threshold {
        cold_max
    } else {
        warm_max
    }
}

/// Outcome of one disconnect event in the per-peer reconnect loop
/// (PURE-LOGIC-001): whether to redial the peer, or one of three reasons
/// not to. Extracted from `join.rs::spawn_reconnect_loop`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReconnectDecision {
    /// Redial the peer.
    Reconnect,
    /// The stored connection was already replaced by a fresher one; this
    /// disconnect event is for the old, now-irrelevant connection.
    IgnoreStaleDisconnect,
    /// A deliberate `tetron leave` -- the peer is gone for good.
    PeerLeftDeliberately,
    /// We ourselves just pruned this peer from the roster (kicked or
    /// departed); the close that woke this loop was our own doing.
    PeerRemovedFromRoster,
}

/// `removed` is whether the peer's route was actually still the one that
/// died (`false` means a stale event for an already-superseded connection).
/// `prunes_member` is `CloseReason::prunes_member()` for the event's close
/// code. `was_pruned_locally` is whether this `(network, peer)` pair was
/// present in the one-shot `pruned_peers` suppression set (and has already
/// been removed from it by the caller).
pub(crate) fn reconnect_decision(
    removed: bool,
    prunes_member: bool,
    was_pruned_locally: bool,
) -> ReconnectDecision {
    if !removed {
        return ReconnectDecision::IgnoreStaleDisconnect;
    }
    if prunes_member {
        return ReconnectDecision::PeerLeftDeliberately;
    }
    if was_pruned_locally {
        return ReconnectDecision::PeerRemovedFromRoster;
    }
    ReconnectDecision::Reconnect
}

/// Outcome of the pre-attempt roster re-check in the per-peer dial task
/// (CONVERGE-010, PURE-LOGIC-001). `reconnect_decision` above only runs when
/// a new disconnect event arrives, which an offline peer never produces --
/// this is the check the already-spinning retry loop runs itself, before
/// every dial, so a roster mutation can stop it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DialRetryDecision {
    /// Peer is still in the roster: dial it (and keep retrying on the
    /// existing backoff -- an unreachable but still-listed member must keep
    /// being retried).
    KeepDialing,
    /// Peer is no longer roster-authorized (kicked or departed while this
    /// task was spinning): stop dialing for good.
    AbandonPeerGone,
}

/// `in_roster` is whether the peer is currently in this network's verified
/// roster (`live_state`, kept current by reconverge). `was_pruned_locally`
/// is whether a one-shot `pruned_peers` entry for `(network, peer)` existed
/// (and has already been consumed by the caller) -- it can land before the
/// roster copy reflects the removal, so either signal alone abandons.
pub(crate) fn dial_retry_decision(in_roster: bool, was_pruned_locally: bool) -> DialRetryDecision {
    if !in_roster || was_pruned_locally {
        return DialRetryDecision::AbandonPeerGone;
    }
    DialRetryDecision::KeepDialing
}

/// PEER-IP-BIND-002: the overlay IPv4 a reconnect re-registers a peer under.
/// The roster member's address wins; `stored` (the address recorded when the
/// dropped connection was first registered) is only a fallback for a peer no
/// longer listed, which the CONVERGE-010 recheck abandons before any re-add.
/// Re-resolving every attempt means a route registered under a wrong address
/// does not survive a reconnect, and a roster address change is picked up.
pub(crate) fn reconnect_route_ip(roster: Option<&Member>, stored: Ipv4Addr) -> Ipv4Addr {
    crate::membership::resolve_hello_ip(roster, None).unwrap_or(stored)
}

/// Classifies the connection that just dropped as a flap or a healthy drop,
/// and returns the starting state for the next reconnect task against this
/// peer (RECONNECT-STORM-001, PURE-LOGIC-001).
///
/// CONVERGE-011/013 only escalates dials that *never connect* (counted by a
/// task's own `failed_attempts`); a peer that connects then drops the
/// connection almost immediately resets a fresh task back to the warm floor
/// every cycle, so it is redialed at the 1s floor forever. This closes that
/// gap: a connection that lived less than `min_uptime` counts as a failure.
///
/// `uptime` is how long the dropped connection lived (`None` for the
/// synthetic cold-restore seed, which no live connection backs).
/// `prior_streak` is this peer's persisted short-lived-drop streak (see
/// `MeshCtx::reconnect_health`). Returns `(is_flap, new_streak,
/// start_backoff)`:
/// - `uptime >= min_uptime`: a healthy connection; streak resets to 0 and
///   the next task starts at the warm floor (`initial`).
/// - `uptime < min_uptime`: a flap; the streak increments and `start_backoff`
///   is the warm floor climbed as if `new_streak` attempts had failed,
///   capped by the same `backoff_cap` the offline-peer path uses, so a
///   chronic flapper escalates toward the cold then frozen caps on the same
///   schedule a chronically-offline peer does (the caller also seeds the new
///   task's `failed_attempts` with `new_streak`, unifying the two paths).
/// - `None`: a seed, not a flap; the streak is preserved and the next task
///   starts warm.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconnect_storm_decision(
    uptime: Option<std::time::Duration>,
    prior_streak: u32,
    min_uptime: std::time::Duration,
    initial: std::time::Duration,
    cold_threshold: u32,
    frozen_threshold: u32,
    warm_max: std::time::Duration,
    cold_max: std::time::Duration,
    frozen_max: std::time::Duration,
) -> (bool, u32, std::time::Duration) {
    match uptime {
        None => (false, prior_streak, initial),
        Some(lived) if lived >= min_uptime => (false, 0, initial),
        Some(_) => {
            let new_streak = prior_streak.saturating_add(1);
            let cap = backoff_cap(
                new_streak,
                cold_threshold,
                frozen_threshold,
                warm_max,
                cold_max,
                frozen_max,
            );
            // Climb from the warm floor as if `new_streak` attempts had
            // failed: initial * 2^new_streak, clamped by the cap in effect.
            // `min(20)` on the exponent keeps the shift well clear of
            // overflow (2^20 * 1s already dwarfs any cap); `checked_mul`
            // falls back to the cap on the off chance it still overflows.
            let climbed = initial
                .checked_mul(1u32 << new_streak.min(20))
                .unwrap_or(cap);
            (true, new_streak, climbed.min(cap))
        }
    }
}

/// Lengthens a reconnect backoff by anti-stampede jitter (RECONNECT-STORM-003,
/// PURE-LOGIC-001). `frac` is the maximum fractional increase (e.g. 0.2 for up
/// to +20%), `sample` a uniform draw in `[0, 1)` from the caller's rng. The
/// result is in `[backoff, backoff * (1 + frac))`: jitter only ever *adds*
/// delay, never shortening the floor, so a fleet of nodes all reconnecting to
/// the same peer spread out instead of re-dialing in lockstep. A non-positive
/// or non-finite `frac`/`sample` degrades to no jitter.
pub(crate) fn jitter_backoff(
    backoff: std::time::Duration,
    frac: f64,
    sample: f64,
) -> std::time::Duration {
    if !(frac.is_finite() && frac > 0.0 && sample.is_finite() && sample >= 0.0) {
        return backoff;
    }
    let extra = backoff.mul_f64((frac * sample).clamp(0.0, frac));
    backoff.saturating_add(extra)
}

#[cfg(test)]
mod restore_coordinator_walk_tests {
    //! RESTORE-COORD-FALLTHROUGH-001: the restore dial walks every flagged
    //! coordinator instead of giving up after the first.
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    fn id(n: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    const BUDGET: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn first_coordinator_joins_and_the_rest_are_never_tried() {
        let tried = RefCell::new(Vec::new());
        let order = [id(1), id(2), id(3)];
        let res = walk_restore_coordinators(&order, BUDGET, |c| {
            tried.borrow_mut().push(c);
            async move { RestoreAttempt::Joined(c) }
        })
        .await;
        assert!(matches!(res, RestoreWalk::Joined(c) if c == id(1)));
        assert_eq!(*tried.borrow(), vec![id(1)]);
    }

    #[tokio::test]
    async fn a_silent_first_coordinator_falls_through_to_the_next() {
        // The 2026-10-10 failure: the first flagged coordinator never greets.
        let tried = RefCell::new(Vec::new());
        let order = [id(1), id(2), id(3)];
        let res = walk_restore_coordinators(&order, BUDGET, |c| {
            tried.borrow_mut().push(c);
            async move {
                if c == id(1) {
                    RestoreAttempt::Failed
                } else {
                    RestoreAttempt::Joined(c)
                }
            }
        })
        .await;
        assert!(matches!(res, RestoreWalk::Joined(c) if c == id(2)));
        assert_eq!(*tried.borrow(), vec![id(1), id(2)]);
    }

    #[tokio::test]
    async fn every_coordinator_failing_is_exhausted_so_the_caller_falls_back() {
        let tried = RefCell::new(0usize);
        let order = [id(1), id(2), id(3)];
        let res: RestoreWalk<()> = walk_restore_coordinators(&order, BUDGET, |_| {
            *tried.borrow_mut() += 1;
            async { RestoreAttempt::Failed }
        })
        .await;
        assert!(matches!(res, RestoreWalk::Exhausted));
        assert_eq!(*tried.borrow(), 3, "every coordinator is tried once");
    }

    #[tokio::test]
    async fn no_coordinators_is_exhausted_without_any_attempt() {
        let tried = RefCell::new(0usize);
        let res: RestoreWalk<()> = walk_restore_coordinators(&[], BUDGET, |_| {
            *tried.borrow_mut() += 1;
            async { RestoreAttempt::Failed }
        })
        .await;
        assert!(matches!(res, RestoreWalk::Exhausted));
        assert_eq!(*tried.borrow(), 0);
    }

    #[tokio::test]
    async fn pending_verdict_stops_the_walk() {
        let tried = RefCell::new(Vec::new());
        let order = [id(1), id(2)];
        let res: RestoreWalk<()> = walk_restore_coordinators(&order, BUDGET, |c| {
            tried.borrow_mut().push(c);
            async { RestoreAttempt::Pending }
        })
        .await;
        assert!(matches!(res, RestoreWalk::Pending));
        assert_eq!(
            *tried.borrow(),
            vec![id(1)],
            "a queued request is not retried elsewhere"
        );
    }

    #[tokio::test]
    async fn spent_budget_stops_starting_new_attempts() {
        let tried = RefCell::new(0usize);
        let order = [id(1), id(2), id(3)];
        // The first attempt alone outlasts the budget, so none follow it.
        let res: RestoreWalk<()> =
            walk_restore_coordinators(&order, Duration::from_millis(20), |_| {
                *tried.borrow_mut() += 1;
                async {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    RestoreAttempt::Failed
                }
            })
            .await;
        assert!(matches!(res, RestoreWalk::Exhausted));
        assert_eq!(*tried.borrow(), 1);
    }

    #[test]
    fn restore_order_skips_self_and_keeps_every_other_flagged_coordinator() {
        let me = id(9);
        let mk = |n: u8, coord: bool| Member {
            identity: id(n),
            ip: std::net::Ipv4Addr::new(10, 0, 0, n),
            is_coordinator: coord,
            hostname: None,
            collision_index: 0,
            last_seen: None,
            veilid_node_id: None,
        };
        let members = [mk(1, false), mk(2, true), mk(9, true), mk(3, true)];
        assert_eq!(coordinator_dial_order(me, &members, me), vec![id(2), id(3)]);
    }
}

#[cfg(test)]
mod first_some_bounded_tests {
    //! BLOB-FETCH-BOUND-001: a stalled or dead peer must not block a roster
    //! fetch from the live peers behind it.
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    #[tokio::test]
    async fn a_stalled_first_peer_is_skipped_for_the_next() {
        let tried = RefCell::new(Vec::new());
        let started = std::time::Instant::now();
        let got = first_some_bounded(
            &[1u8, 2, 3],
            Duration::from_millis(50),
            Duration::from_secs(5),
            |n| {
                tried.borrow_mut().push(*n);
                let n = *n;
                async move {
                    if n == 1 {
                        // The dead peer: never completes.
                        std::future::pending::<Option<u8>>().await
                    } else {
                        Some(n)
                    }
                }
            },
        )
        .await;
        assert_eq!(got, Some(2));
        assert_eq!(*tried.borrow(), vec![1, 2], "the third peer is never tried");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "must not wait on the dead peer"
        );
    }

    #[tokio::test]
    async fn a_failing_peer_is_skipped_for_the_next() {
        let got = first_some_bounded(
            &[1u8, 2],
            Duration::from_secs(5),
            Duration::from_secs(5),
            |n| {
                let n = *n;
                async move { (n == 2).then_some(n) }
            },
        )
        .await;
        assert_eq!(got, Some(2));
    }

    #[tokio::test]
    async fn first_success_ends_the_walk() {
        let tried = RefCell::new(0usize);
        let got = first_some_bounded(
            &[1u8, 2, 3],
            Duration::from_secs(5),
            Duration::from_secs(5),
            |n| {
                *tried.borrow_mut() += 1;
                let n = *n;
                async move { Some(n) }
            },
        )
        .await;
        assert_eq!(got, Some(1));
        assert_eq!(*tried.borrow(), 1);
    }

    #[tokio::test]
    async fn all_peers_stalled_returns_none_within_the_budget() {
        let tried = RefCell::new(0usize);
        let started = std::time::Instant::now();
        let got: Option<u8> = first_some_bounded(
            &[1u8, 2, 3, 4, 5, 6, 7, 8],
            Duration::from_millis(40),
            Duration::from_millis(100),
            |_| {
                *tried.borrow_mut() += 1;
                std::future::pending::<Option<u8>>()
            },
        )
        .await;
        assert_eq!(got, None);
        assert!(
            *tried.borrow() < 8,
            "the budget stops new attempts: tried {}",
            tried.borrow()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn no_peers_is_none_without_any_attempt() {
        let tried = RefCell::new(0usize);
        let got: Option<u8> = first_some_bounded(
            &[] as &[u8],
            Duration::from_secs(1),
            Duration::from_secs(1),
            |_| {
                *tried.borrow_mut() += 1;
                async { None }
            },
        )
        .await;
        assert_eq!(got, None);
        assert_eq!(*tried.borrow(), 0);
    }
}

#[cfg(test)]
mod grant_publish_action_tests {
    //! GRANT-PUBLISH-001: a grant never publishes a roster the DHT is ahead of.
    use super::*;

    fn h(n: u8) -> blake3::Hash {
        blake3::hash(&[n])
    }

    #[test]
    fn no_dht_record_means_publish() {
        assert_eq!(grant_publish_action(5, h(1), None), GrantPublish::Publish);
    }

    #[test]
    fn higher_local_generation_publishes() {
        assert_eq!(
            grant_publish_action(6, h(1), Some((5, h(2)))),
            GrantPublish::Publish
        );
    }

    #[test]
    fn identical_record_is_already_published() {
        assert_eq!(
            grant_publish_action(5, h(1), Some((5, h(1)))),
            GrantPublish::AlreadyPublished
        );
    }

    #[test]
    fn a_dht_that_is_ahead_forces_a_reconverge_not_a_publish() {
        // The case that loses a grant: our bumped generation is still behind.
        assert_eq!(
            grant_publish_action(5, h(1), Some((9, h(2)))),
            GrantPublish::Reconverge
        );
    }

    #[test]
    fn a_tie_with_different_content_forces_a_reconverge() {
        // Two coordinators mutated from the same base: ours is not the record.
        assert_eq!(
            grant_publish_action(5, h(1), Some((5, h(2)))),
            GrantPublish::Reconverge
        );
    }
}
