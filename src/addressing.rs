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
