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
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                    last_seen: None,
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
    if addr.is_custom() {
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
///    (Direct > Relay > Tor), *regardless* of `is_selected()` -- corroborated
///    real traffic outranks a bare selected label or classification alone.
/// 3. Nothing has proven itself yet: fall back to plain classification among
///    all trustworthy candidates, so a genuinely new, still-validating path
///    is still reported rather than hidden as `?` just for being new.
pub(crate) fn choose_path_index(classes: &[(ipc::ConnType, bool, bool, bool)]) -> Option<usize> {
    let trustworthy_count = classes.iter().filter(|(_, _, in_subnet, _)| *in_subnet).count();

    if let Some(i) = classes.iter().position(|(_, selected, in_subnet, has_activity)| {
        *selected && *in_subnet && (*has_activity || trustworthy_count == 1)
    }) {
        return Some(i);
    }

    for want in [
        ipc::ConnType::Direct,
        ipc::ConnType::Relay,
        ipc::ConnType::Tor,
    ] {
        if let Some(i) = classes
            .iter()
            .position(|(ct, _, in_subnet, has_activity)| {
                *ct == want && *in_subnet && *has_activity
            })
        {
            return Some(i);
        }
    }

    for want in [
        ipc::ConnType::Direct,
        ipc::ConnType::Relay,
        ipc::ConnType::Tor,
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
