//! Overlay addressing: the configurable IPv4 subnet and deterministic
//! IPv4/IPv6 derivation from an [`EndpointId`]. Pure, stateless helpers —
//! see `spec/addressing.py` (SUBNET-*) for the requirement domain.
//!
//! Split out of `membership.rs` (MODULARIZE-001); `crate::membership::…`
//! paths to everything here keep working via re-export.

use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;
use iroh::EndpointId;

use crate::membership::{Member, MemberList};

/// The overlay subnet as `(base address, prefix length)`. `None` in a
/// [`crate::membership::GroupBlob`] / config means "use [`default_subnet`]".
pub type Subnet = (Ipv4Addr, u8);

/// The default overlay subnet when none is configured: 10.88.0.0/24, an
/// uncommon 10.x slice chosen so a no-flag `tetron create` does NOT collide
/// with Tailscale's 100.64.0.0/10 out of the box (SUBNET-011). Override with
/// `--subnet` / `tetron config set subnet`. A /24 gives 256 host addresses,
/// enough for personal/team meshes.
pub fn default_subnet() -> Subnet {
    (Ipv4Addr::new(10, 88, 0, 0), 24)
}

/// Resolve an optional configured subnet to a concrete one, falling back to
/// [`default_subnet`].
pub fn resolve_subnet(subnet: Option<Subnet>) -> Subnet {
    subnet.unwrap_or_else(default_subnet)
}

/// SUBNET-014: the node runs a single TUN whose subnet is fixed at daemon
/// startup, so a `create --subnet` / `join` that selects a different subnet only
/// takes effect after a restart. Returns a user-facing warning to that effect
/// when `chosen` differs from the node's `live` TUN subnet; `None` when they
/// match (nothing to announce).
pub fn subnet_change_warning(chosen: Subnet, live: Subnet) -> Option<String> {
    (chosen != live).then(|| {
        let (b, p) = chosen;
        format!(
            "subnet {b}/{p} takes effect after `sudo tetron restart` (this node's live overlay is still on the previous subnet)"
        )
    })
}

/// Host-bit mask for a prefix length: the low `32 - prefix` bits set.
pub fn subnet_host_mask(prefix: u8) -> u32 {
    if prefix >= 32 {
        0
    } else {
        (1u32 << (32 - prefix)) - 1
    }
}

/// Dotted-quad netmask for a prefix length (e.g. /10 -> 255.192.0.0).
pub fn subnet_netmask(prefix: u8) -> Ipv4Addr {
    Ipv4Addr::from(!subnet_host_mask(prefix))
}

/// True if `ip` falls within `subnet` (network bits match). The single
/// containment predicate shared by `ensure_in_cgnat_range` and the DNS
/// PTR/reverse-lookup range check, so the two can never diverge (SUBNET-008).
pub fn ip_in_subnet(ip: Ipv4Addr, subnet: Subnet) -> bool {
    let (base_addr, prefix) = subnet;
    let host_mask = subnet_host_mask(prefix);
    let base = u32::from(base_addr) & !host_mask;
    (u32::from(ip) & !host_mask) == base
}

/// Checks a resolved subnet against this identity's own already-recorded
/// roster IP (SUBNET-DRIFT-001). A daemon restart re-resolves a network's
/// subnet independently of the roster; if that resolution ever disagrees
/// with what the signed roster already says this identity's address is,
/// attaching a TUN in the newly (wrongly) resolved subnet silently breaks
/// data-plane routing to every peer -- the underlying transport connection
/// stays up (control-channel traffic doesn't care about TUN addressing),
/// so nothing looks obviously broken in `tetron status` even though no
/// application traffic can actually reach anyone. Returns an error instead
/// of a bool so callers can propagate a message that names both values.
/// A no-op (`Ok`) when the identity isn't in the roster yet -- nothing to
/// check against.
pub fn validate_subnet_matches_roster<'a>(
    subnet: Subnet,
    roster: impl IntoIterator<Item = &'a Member>,
    self_identity: &EndpointId,
) -> Result<(), String> {
    let Some(member) = roster.into_iter().find(|m| &m.identity == self_identity) else {
        return Ok(());
    };
    if ip_in_subnet(member.ip, subnet) {
        Ok(())
    } else {
        Err(format!(
            "this network's signed roster records this node's address as {} \
             (identity {}), but the resolved subnet is {}/{} -- refusing to \
             start with inconsistent state rather than silently break \
             data-plane routing to every peer",
            member.ip,
            self_identity.fmt_short(),
            subnet.0,
            subnet.1,
        ))
    }
}

/// True if two IPv4 subnets share any address. Compares network bits at the
/// shorter (less specific) prefix, so it catches overlap in both directions —
/// a smaller range inside a larger one, or vice versa (SUBNET-012). Two ranges
/// that share no network bits (e.g. 10.88.0.0/16 vs Tailscale's 100.64.0.0/10)
/// do not overlap.
pub fn subnets_overlap(a: Subnet, b: Subnet) -> bool {
    let (a_addr, a_prefix) = a;
    let (b_addr, b_prefix) = b;
    let host_mask = subnet_host_mask(a_prefix.min(b_prefix));
    (u32::from(a_addr) & !host_mask) == (u32::from(b_addr) & !host_mask)
}

/// Finds a subnet that doesn't overlap any of `existing`, starting from
/// `candidate` and keeping its prefix length fixed -- every network on a
/// node must get a genuinely distinct subnet, not silently reuse whatever
/// the node's single default happens to be (found live-testing: two
/// networks created back to back without an explicit `--subnet` both got
/// `10.77.0.0/24`, giving the same node the identical address on both and
/// defeating the whole point of per-network subnets). Advances by one
/// block size (`2^(32-prefix)` addresses) per attempt so successive
/// candidates never overlap each other either. Capped at 4096 attempts --
/// astronomically more than any real node's network count -- after which
/// it gives up and returns `candidate` unchanged rather than looping
/// forever; a caller can still detect that case by checking for overlap
/// again if it cares.
pub fn next_available_subnet(candidate: Subnet, existing: impl Iterator<Item = Subnet>) -> Subnet {
    let existing: Vec<Subnet> = existing.collect();
    let (base, prefix) = candidate;
    let block_size = 1u32.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
    let mut attempt = (base, prefix);
    for _ in 0..4096 {
        if !existing.iter().any(|&e| subnets_overlap(e, attempt)) {
            return attempt;
        }
        if block_size == 0 {
            break;
        }
        let next_base = Ipv4Addr::from(u32::from(attempt.0).wrapping_add(block_size));
        attempt = (next_base, prefix);
    }
    attempt
}

/// The gateway address for a subnet: base + 1 (the `.1` the TUN takes).
pub fn subnet_gateway(subnet: Subnet) -> Ipv4Addr {
    let (base, _) = subnet;
    Ipv4Addr::from(u32::from(base).wrapping_add(1))
}

/// Parse a CIDR string (e.g. `"10.88.0.0/16"`) into a [`Subnet`].
pub fn parse_cidr(s: &str) -> Result<Subnet> {
    let (base, prefix) = s
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("subnet must be CIDR base/prefix, e.g. 10.88.0.0/16"))?;
    let base: Ipv4Addr = base
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("bad subnet base address: {e}"))?;
    let prefix: u8 = prefix
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("bad subnet prefix length: {e}"))?;
    anyhow::ensure!(prefix <= 32, "subnet prefix must be 0..=32");
    Ok((base, prefix))
}

/// Serde helper (de)serializing `Option<Subnet>` as an optional CIDR string
/// (e.g. `"10.88.0.0/16"`). Keeps the on-disk (TOML) and on-wire (msgpack)
/// forms uniform and human-readable, and sidesteps TOML's ban on heterogeneous
/// arrays that a raw `(Ipv4Addr, u8)` tuple would trip.
pub mod cidr_opt {
    use super::Subnet;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Subnet>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some((base, prefix)) => Some(format!("{base}/{prefix}")).serialize(s),
            None => Option::<String>::None.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Subnet>, D::Error> {
        let opt = Option::<String>::deserialize(d)?;
        match opt {
            None => Ok(None),
            Some(cidr) => Ok(Some(
                super::parse_cidr(&cidr).map_err(serde::de::Error::custom)?,
            )),
        }
    }
}

/// Derives a deterministic virtual IP from an [`EndpointId`] using FNV-1a.
/// Always produces an address inside `subnet`, avoiding .0 and .1
/// (network address and TUN gateway).
pub fn derive_ip(identity: &EndpointId, subnet: Subnet) -> Ipv4Addr {
    derive_ip_with_index(identity, 0, subnet)
}

/// Derives a virtual IPv4 with a collision index. Index 0 produces the same
/// result as [`derive_ip`]. Higher indices rotate the address to resolve
/// collisions in the 22-bit space. The index is local state — each node
/// resolves collisions independently.
pub fn derive_ip_with_index(identity: &EndpointId, index: u32, subnet: Subnet) -> Ipv4Addr {
    let input = if index == 0 {
        identity.to_string()
    } else {
        format!("{identity}{index}")
    };
    let mut hash: u32 = 2_166_136_261; // FNV-1a offset basis
    for &b in input.as_bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16_777_619); // FNV-1a prime
    }

    let (base_addr, prefix) = subnet;
    // Mask the base to its network bits so a caller passing e.g. 10.88.0.5/16
    // still anchors on the network address.
    let host_mask = subnet_host_mask(prefix);
    let base: u32 = u32::from(base_addr) & !host_mask;
    let host_bits = hash & host_mask; // low (32 - prefix) bits
    // Reserve 0 (network) and 1 (TUN gateway)
    let host_bits = if host_bits <= 1 {
        host_bits + 2
    } else {
        host_bits
    };
    Ipv4Addr::from(base | host_bits)
}

/// Finds the lowest collision index whose derived IPv4 is free in `members`.
///
/// An IP is considered free if no *different* identity holds it — a re-add of
/// the same identity at its existing index is always accepted. Returns the
/// `(ip, index)` pair that should be stored in `Member.ip` / `Member.collision_index`.
pub fn assign_ip(members: &MemberList, identity: &EndpointId, subnet: Subnet) -> (Ipv4Addr, u32) {
    let mut index = 0u32;
    loop {
        let ip = derive_ip_with_index(identity, index, subnet);
        match members.get_by_ip(ip) {
            Some(existing) if existing.identity != *identity => index += 1,
            _ => return (ip, index),
        }
    }
}

/// Derives a stable, per-network IPv6 address from an [`EndpointId`] in the
/// `200::/7` range (IPV6-001). Structural split: fixed `0x02` (byte 0) +
/// 48-bit network-prefix (bytes 1-6, a function of `network` alone, so every
/// member of a network shares the same routable `/56` block — see
/// [`ipv6_network_prefix`]) + 72-bit peer-part (bytes 7-15, a function of
/// both `identity` and `network`, so the same identity gets an unrelated
/// address in each network it joins — closes a cross-network grinding-reuse
/// loophole against [`derive_ipv6`]'s admission-time collision check).
/// No collision-index: at 72 bits, a 1% collision probability needs ~3.1
/// billion members of one network, so accidental collision is not a
/// practical concern (deliberate grinding is handled separately, at
/// admission).
pub fn derive_ipv6(identity: &EndpointId, network: &EndpointId) -> Ipv6Addr {
    let prefix = ipv6_network_prefix(network);
    let peer_hash = blake3::hash(format!("{identity}:{network}").as_bytes());
    let peer_bytes = peer_hash.as_bytes();
    let prefix_octets = prefix.octets();
    let mut octets = [0u8; 16];
    octets[..7].copy_from_slice(&prefix_octets[..7]);
    octets[7..16].copy_from_slice(&peer_bytes[..9]);
    Ipv6Addr::from(octets)
}

/// Bit width of the fixed `0x02` tag plus [`derive_ipv6`]'s network-prefix —
/// the CIDR block shared by every member of one network (IPV6-003).
pub const IPV6_NETWORK_PREFIX_LEN: u8 = 56;

/// The `/56` network-prefix address for `network` (IPV6-001/003): fixed
/// `0x02` + 48 bits of `blake3(network)`, peer-part bits zeroed. Used both as
/// the base for [`derive_ipv6`] and as the route destination each network's
/// TUN device gets (`crate::tun::route_peer_range`).
pub fn ipv6_network_prefix(network: &EndpointId) -> Ipv6Addr {
    let hash = blake3::hash(network.to_string().as_bytes());
    let bytes = hash.as_bytes();
    let octets: [u8; 16] = [
        0x02, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], 0, 0, 0, 0, 0, 0, 0, 0,
        0,
    ];
    Ipv6Addr::from(octets)
}

/// Whether `addr` falls within `network`'s own `/56` (PATHBLEED-STATUS-001):
/// the v6 sibling of [`ip_in_subnet`], checking against
/// [`ipv6_network_prefix`] instead of a configurable v4 [`Subnet`] since a
/// network's v6 prefix is derived, not chosen. `IPV6_NETWORK_PREFIX_LEN` (56)
/// is a whole number of bytes, so comparing the first 7 octets is exact.
pub fn ipv6_in_network(addr: Ipv6Addr, network: &EndpointId) -> bool {
    let prefix_bytes = (IPV6_NETWORK_PREFIX_LEN / 8) as usize;
    addr.octets()[..prefix_bytes] == ipv6_network_prefix(network).octets()[..prefix_bytes]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership::Member;
    use std::collections::HashMap;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = iroh::SecretKey::from(key_bytes);
        key.public()
    }

    #[test]
    fn subnet_change_warning_fires_only_on_mismatch() {
        let live = default_subnet();
        let other = ("10.99.0.0".parse::<std::net::Ipv4Addr>().unwrap(), 16u8);
        assert!(subnet_change_warning(live, live).is_none());
        let w = subnet_change_warning(other, live).expect("mismatch must warn");
        assert!(w.contains("sudo tetron restart"), "{w}");
    }

    #[test]
    fn test_derive_ip_deterministic() {
        let id = test_id(1);
        let ip1 = derive_ip(&id, default_subnet());
        let ip2 = derive_ip(&id, default_subnet());
        assert_eq!(ip1, ip2);
    }

    #[test]
    fn test_derive_ip_in_default_subnet() {
        let id = test_id(1);
        let ip = derive_ip(&id, default_subnet());
        assert!(ip_in_subnet(ip, default_subnet()));
    }

    #[test]
    fn test_derive_ip_different_identities_differ() {
        let ip1 = derive_ip(&test_id(1), default_subnet());
        let ip2 = derive_ip(&test_id(2), default_subnet());
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn test_derive_ip_avoids_reserved() {
        // Was previously checked against 100.64.0.0/100.64.0.1 -- reserved
        // addresses for the pre-fork default subnet, not this one. Since
        // `default_subnet()` never derives an address outside 10.88.0.0/24,
        // that comparison was vacuously true for every `i`, silently testing
        // nothing. Must be this subnet's own reserved addresses.
        let (base, _) = default_subnet();
        let reserved1 = base;
        let reserved2 = Ipv4Addr::from(u32::from(base) + 1);
        for i in 0..=255u8 {
            let ip = derive_ip(&test_id(i), default_subnet());
            assert_ne!(ip, reserved1);
            assert_ne!(ip, reserved2);
        }
    }

    #[test]
    fn test_derive_ip_with_index_zero_matches_derive_ip() {
        for i in 0..=255u8 {
            let id = test_id(i);
            assert_eq!(
                derive_ip(&id, default_subnet()),
                derive_ip_with_index(&id, 0, default_subnet())
            );
        }
    }

    #[test]
    fn test_derive_ip_with_index_rotates() {
        let id = test_id(1);
        let ip0 = derive_ip_with_index(&id, 0, default_subnet());
        let ip1 = derive_ip_with_index(&id, 1, default_subnet());
        let ip2 = derive_ip_with_index(&id, 2, default_subnet());
        assert_ne!(ip0, ip1);
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn test_derive_ipv6_deterministic() {
        let id = test_id(1);
        let net = test_id(100);
        assert_eq!(derive_ipv6(&id, &net), derive_ipv6(&id, &net));
    }

    #[test]
    fn test_derive_ipv6_in_200_range() {
        let net = test_id(100);
        for i in 0..=255u8 {
            let ipv6 = derive_ipv6(&test_id(i), &net);
            let octets = ipv6.octets();
            assert_eq!(octets[0], 0x02, "first byte must be 0x02 for 200::/7");
        }
    }

    #[test]
    fn test_derive_ipv6_different_identities_differ() {
        let net = test_id(100);
        let a = derive_ipv6(&test_id(1), &net);
        let b = derive_ipv6(&test_id(2), &net);
        assert_ne!(a, b);
    }

    #[test]
    fn test_derive_ipv6_same_identity_different_networks_differ() {
        let id = test_id(1);
        let net_a = test_id(100);
        let net_b = test_id(101);
        assert_ne!(derive_ipv6(&id, &net_a), derive_ipv6(&id, &net_b));
    }

    #[test]
    fn test_derive_ipv6_shares_network_prefix() {
        let net = test_id(100);
        let a = derive_ipv6(&test_id(1), &net);
        let b = derive_ipv6(&test_id(2), &net);
        assert_eq!(a.octets()[..7], b.octets()[..7], "same network -> same /56 prefix");
        let prefix = ipv6_network_prefix(&net);
        assert_eq!(a.octets()[..7], prefix.octets()[..7]);
    }

    #[test]
    fn test_ipv6_network_prefix_deterministic_and_distinct() {
        let net_a = test_id(100);
        let net_b = test_id(101);
        assert_eq!(ipv6_network_prefix(&net_a), ipv6_network_prefix(&net_a));
        assert_ne!(ipv6_network_prefix(&net_a), ipv6_network_prefix(&net_b));
        assert_eq!(ipv6_network_prefix(&net_a).octets()[0], 0x02);
        assert_eq!(
            &ipv6_network_prefix(&net_a).octets()[7..],
            &[0u8; 9],
            "peer-part bits must be zeroed in the prefix address"
        );
    }

    #[test]
    fn ipv6_in_network_matches_own_prefix_only() {
        let net_a = test_id(100);
        let net_b = test_id(101);
        let prefix_a = ipv6_network_prefix(&net_a);
        // The network's own prefix address always matches itself.
        assert!(ipv6_in_network(prefix_a, &net_a));
        // A derived peer address (same /56, distinct low bits) still matches.
        let peer_addr = derive_ipv6(&test_id(200), &net_a);
        assert!(ipv6_in_network(peer_addr, &net_a));
        // The identical address does not match a different network's prefix.
        assert!(!ipv6_in_network(peer_addr, &net_b));
    }

    /// Brute-force (birthday approach) to find two distinct identities whose
    /// index-0 IPv4 collides. The 22-bit space makes this likely within ~a few
    /// thousand iterations. Bounded at 200_000 to avoid a runaway test.
    fn find_colliding_pair() -> Option<(EndpointId, EndpointId)> {
        let mut seen: HashMap<Ipv4Addr, EndpointId> = HashMap::new();
        for i in 0u32..200_000 {
            // Vary bytes across the whole 32-byte key to get good hash dispersion.
            let mut key_bytes = [0u8; 32];
            let b = i.to_le_bytes();
            key_bytes[0] = b[0];
            key_bytes[1] = b[1];
            key_bytes[2] = b[2];
            key_bytes[3] = b[3];
            let id = iroh::SecretKey::from(key_bytes).public();
            let ip = derive_ip(&id, default_subnet());
            if let Some(existing) = seen.get(&ip) {
                if *existing != id {
                    return Some((*existing, id));
                }
            } else {
                seen.insert(ip, id);
            }
        }
        None
    }

    #[test]
    fn assign_ip_rotates_on_collision() {
        let (a, b) = find_colliding_pair()
            .expect("birthday bound: should find a collision within 200k identities");
        // Sanity: a and b both map to the same index-0 IP.
        assert_eq!(
            derive_ip(&a, default_subnet()),
            derive_ip(&b, default_subnet())
        );
        let ip0 = derive_ip(&a, default_subnet());

        // Add `a` to the list at its index-0 IP.
        let mut list = crate::membership::MemberList::new();
        let (assigned_a, idx_a) = assign_ip(&list, &a, default_subnet());
        assert_eq!(idx_a, 0, "first peer always gets index 0");
        assert_eq!(assigned_a, ip0);
        list.add(Member {
            identity: a,
            ip: assigned_a,
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: idx_a,
            last_seen: None,
        })
        .unwrap();

        // Now assign_ip for `b` must rotate to index >= 1.
        let (ip_b, idx_b) = assign_ip(&list, &b, default_subnet());
        assert!(idx_b >= 1, "colliding identity must rotate to index >= 1");
        assert_ne!(ip_b, ip0, "rotated IP must differ from the occupied slot");
        assert_eq!(
            ip_b,
            derive_ip_with_index(&b, idx_b, default_subnet()),
            "assigned IP must equal derive_ip_with_index at that index"
        );
    }

    // A custom subnet distinct from BOTH the default (10.88.0.0/24) and the
    // legacy CGNAT range, so "custom != default" assertions stay meaningful.
    const CUSTOM: Subnet = (Ipv4Addr::new(10, 99, 0, 0), 16);

    #[test]
    fn derive_ip_lands_in_custom_subnet_and_avoids_reserved() {
        for seed in 1u8..40 {
            let ip = derive_ip(&test_id(seed), CUSTOM);
            assert!(ip_in_subnet(ip, CUSTOM), "{ip} not in {CUSTOM:?}");
            let host = u32::from(ip) & subnet_host_mask(16);
            assert!(host >= 2, "{ip} must avoid network(.0)/gateway(.1)");
            // A custom-subnet address must NOT be a default-range CGNAT address.
            assert!(!ip_in_subnet(ip, default_subnet()));
        }
    }

    #[test]
    fn derive_ip_is_deterministic_per_subnet() {
        let id = test_id(7);
        assert_eq!(derive_ip(&id, CUSTOM), derive_ip(&id, CUSTOM));
        // Different subnets generally yield different addresses.
        assert_ne!(
            u32::from(derive_ip(&id, CUSTOM)) & !subnet_host_mask(16),
            u32::from(derive_ip(&id, default_subnet())) & !subnet_host_mask(10),
        );
    }

    #[test]
    fn netmask_and_gateway_track_prefix() {
        assert_eq!(subnet_netmask(10), Ipv4Addr::new(255, 192, 0, 0));
        assert_eq!(subnet_netmask(16), Ipv4Addr::new(255, 255, 0, 0));
        assert_eq!(subnet_netmask(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(subnet_gateway(CUSTOM), Ipv4Addr::new(10, 99, 0, 1));
        assert_eq!(
            subnet_gateway(default_subnet()),
            Ipv4Addr::new(10, 88, 0, 1)
        );
    }

    #[test]
    fn subnets_overlap_detects_both_directions_but_not_disjoint() {
        let overlay = (Ipv4Addr::new(10, 88, 0, 0), 16);
        // A LAN address inside our range overlaps.
        assert!(subnets_overlap((Ipv4Addr::new(10, 88, 5, 2), 24), overlay));
        // A broad host route (10/8) that CONTAINS our range overlaps.
        assert!(subnets_overlap((Ipv4Addr::new(10, 0, 0, 5), 8), overlay));
        // Identical range overlaps itself.
        assert!(subnets_overlap(overlay, overlay));
        // A disjoint home LAN does not.
        assert!(!subnets_overlap(
            (Ipv4Addr::new(192, 168, 1, 5), 24),
            overlay
        ));
        // Crucially, Tailscale's 100.64.0.0/10 does NOT overlap 10.88.0.0/24 —
        // this is the whole point of the safe default.
        assert!(!subnets_overlap(
            (Ipv4Addr::new(100, 64, 0, 1), 10),
            overlay
        ));
    }

    #[test]
    fn next_available_subnet_returns_candidate_when_free() {
        let candidate = (Ipv4Addr::new(10, 88, 0, 0), 24);
        let existing = vec![(Ipv4Addr::new(10, 77, 0, 0), 24)];
        assert_eq!(
            next_available_subnet(candidate, existing.into_iter()),
            candidate
        );
    }

    #[test]
    fn next_available_subnet_advances_past_one_collision() {
        // Exactly the live-testing scenario: a second network created with no
        // explicit --subnet must not silently reuse the first one's.
        let candidate = (Ipv4Addr::new(10, 77, 0, 0), 24);
        let existing = vec![(Ipv4Addr::new(10, 77, 0, 0), 24)];
        let picked = next_available_subnet(candidate, existing.clone().into_iter());
        assert_ne!(picked, candidate);
        assert!(!existing.iter().any(|&e| subnets_overlap(e, picked)));
    }

    #[test]
    fn next_available_subnet_advances_past_several_collisions_in_order() {
        let candidate = (Ipv4Addr::new(10, 77, 0, 0), 24);
        // Three networks already occupy the first three /24 blocks in order.
        let existing = vec![
            (Ipv4Addr::new(10, 77, 0, 0), 24),
            (Ipv4Addr::new(10, 77, 1, 0), 24),
            (Ipv4Addr::new(10, 77, 2, 0), 24),
        ];
        let picked = next_available_subnet(candidate, existing.clone().into_iter());
        assert_eq!(picked, (Ipv4Addr::new(10, 77, 3, 0), 24));
        assert!(!existing.iter().any(|&e| subnets_overlap(e, picked)));
    }

    #[test]
    fn next_available_subnet_keeps_prefix_length() {
        let candidate = (Ipv4Addr::new(10, 88, 0, 0), 16);
        let existing = vec![(Ipv4Addr::new(10, 88, 0, 0), 16)];
        let picked = next_available_subnet(candidate, existing.into_iter());
        assert_eq!(picked.1, 16);
    }

    #[test]
    fn parse_cidr_roundtrips_and_rejects_garbage() {
        assert_eq!(parse_cidr("10.99.0.0/16").unwrap(), CUSTOM);
        assert!(parse_cidr("10.99.0.0").is_err());
        assert!(parse_cidr("not-an-ip/16").is_err());
        assert!(parse_cidr("10.0.0.0/33").is_err());
    }

    /// Like [`make_member_list_in`] in `membership.rs`'s own test module, but
    /// local to this one so neither module needs to expose test-only helpers
    /// to the other.
    fn make_member_list_in(seeds: &[u8], subnet: Subnet) -> crate::membership::MemberList {
        let mut list = crate::membership::MemberList::new();
        for &s in seeds {
            let id = test_id(s);
            let (ip, idx) = assign_ip(&list, &id, subnet);
            list.add(Member {
                identity: id,
                ip,
                is_coordinator: false,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: idx,
                last_seen: None,
            })
            .unwrap();
        }
        list
    }

    #[test]
    fn validate_subnet_matches_roster_ok_when_consistent() {
        // The roster's own recorded IP for this identity actually falls
        // within the subnet being validated -- no mismatch, no error.
        let subnet = default_subnet();
        let members = make_member_list_in(&[1, 2], subnet);
        let me = test_id(1);
        assert!(validate_subnet_matches_roster(subnet, members.all(), &me).is_ok());
    }

    #[test]
    fn validate_subnet_matches_roster_rejects_mismatch() {
        // SUBNET-DRIFT-001: this is the exact shape of the live bug -- a
        // roster whose own recorded IP for this identity was assigned in one
        // subnet, checked against a *different*, wrongly re-resolved one.
        const OTHER: Subnet = (Ipv4Addr::new(10, 99, 0, 0), 24);
        let subnet = default_subnet();
        let members = make_member_list_in(&[1, 2], subnet);
        let me = test_id(1);
        let recorded_ip = members
            .all()
            .into_iter()
            .find(|m| m.identity == me)
            .unwrap()
            .ip
            .to_string();
        let err = validate_subnet_matches_roster(OTHER, members.all(), &me)
            .expect_err("mismatched subnet must be rejected, not silently accepted");
        // Names both the roster's recorded address and the wrongly-resolved
        // subnet, so an operator (or a bug report) has enough to act on.
        assert!(err.contains(&recorded_ip));
        assert!(err.contains("10.99.0.0"));
    }

    #[test]
    fn validate_subnet_matches_roster_ok_when_identity_absent() {
        // A fresh join: the roster doesn't contain this identity yet (not
        // admitted), so there is nothing to check against -- must not error.
        let subnet = default_subnet();
        let members = make_member_list_in(&[1, 2], subnet);
        let not_yet_a_member = test_id(99);
        const OTHER: Subnet = (Ipv4Addr::new(10, 99, 0, 0), 24);
        assert!(validate_subnet_matches_roster(OTHER, members.all(), &not_yet_a_member).is_ok());
    }
}
