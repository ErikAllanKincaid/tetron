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
/// Pick which connection path to report in `tetron status`. Prefers the path iroh
/// has selected; otherwise falls back to the best concrete path so a live
/// connection never renders as `Unknown` (`?`). Priority Direct > Relay > Tor.
/// Returns the index into `classes`, or `None` when there are no paths, or
/// (PATHBLEED-STATUS-001) when every path is untrustworthy.
///
/// Each candidate is `(conn_type, is_selected, in_subnet)`. `in_subnet` is
/// `gather_conn_info`'s PATHBLEED-STATUS-001 check: whether this path's own
/// address belongs to *this connection's own network* (always `true` for
/// Relay/Tor, which are not network-scoped). A candidate with
/// `in_subnet == false` is excluded from both the `is_selected()`-preference
/// check and the fallback scan below -- not just stripped of its selected
/// flag, since the fallback loop matches by classification alone and would
/// otherwise re-surface the same wrong address a moment later. This is
/// exactly the shape iroh's `RemoteStateActor` broadcasting one peer-wide
/// `selected_path` across every one of a shared peer's connections can
/// produce: a bled candidate from a different tetron network, marked
/// `is_selected()` on this network's connection too.
pub(crate) fn choose_path_index(classes: &[(ipc::ConnType, bool, bool)]) -> Option<usize> {
    if let Some(i) = classes
        .iter()
        .position(|(_, selected, in_subnet)| *selected && *in_subnet)
    {
        return Some(i);
    }
    for want in [
        ipc::ConnType::Direct,
        ipc::ConnType::Relay,
        ipc::ConnType::Tor,
    ] {
        if let Some(i) = classes
            .iter()
            .position(|(ct, _, in_subnet)| *ct == want && *in_subnet)
        {
            return Some(i);
        }
    }
    // A path with no IP/relay/custom classification (none today), or the
    // only remaining trustworthy one isn't among the three known classes
    // (shouldn't happen), or -- deliberately -- every candidate is
    // untrustworthy: report unknown rather than a definitely-wrong address.
    classes.iter().position(|(_, _, in_subnet)| *in_subnet)
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
