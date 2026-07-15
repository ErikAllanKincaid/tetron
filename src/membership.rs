//! Network membership management: identity, IP derivation, member/approved lists, and policies.
//!
//! Virtual IPs are deterministically derived from [`EndpointId`] via FNV-1a hashing
//! into the network's configured overlay subnet (the `GroupBlob.subnet` field),
//! defaulting to 10.88.0.0/24 when none is set (an uncommon 10.x slice that
//! avoids Tailscale's 100.64.0.0/10). The host-bit width follows the prefix.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Result, bail};
use iroh::EndpointId;
use tetron_proto::SuggestedFirewall;
use serde::{Deserialize, Serialize};

use crate::control::DeviceCert;

/// Current Unix time in whole seconds (0 if the clock predates the epoch).
/// Shared clock source for `Member::last_seen` stamping and the ephemeral pruner.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A peer that has been admitted to the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    pub is_coordinator: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_identity: Option<EndpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_cert: Option<DeviceCert>,
    /// Index used to resolve IPv4 collisions in the 22-bit CGNAT space.
    /// 0 for most peers; incremented only when `derive_ip_with_index(identity, 0)`
    /// collides with an already-assigned address.
    #[serde(default)]
    pub collision_index: u32,
    /// Unix seconds this peer was last observed going offline. `None` = never
    /// observed offline, so the ephemeral pruner never evicts it. Stamped on
    /// disconnect and seeded at admit; part of the hashed blob so it replicates
    /// to co-coordinators and survives a coordinator restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

/// Controls who can approve new members joining the network.
///
/// Defined in `ray-proto` (shared with GUI frontends); re-exported here so
/// existing `crate::membership::GroupMode` paths keep working.
pub use tetron_proto::GroupMode;

/// Two different identities hashed to the same virtual IP (extremely rare with 22-bit space).
#[derive(Debug)]
pub struct IpCollision {
    pub ip: Ipv4Addr,
    pub existing_identity: EndpointId,
    pub new_identity: EndpointId,
}

impl fmt::Display for IpCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IP collision: {} already assigned to {}, cannot assign to {}",
            self.ip,
            self.existing_identity.fmt_short(),
            self.new_identity.fmt_short()
        )
    }
}

impl std::error::Error for IpCollision {}

/// Active members of a network, keyed by [`EndpointId`]. Rejects additions
/// that would create an IP collision with an existing member.
#[derive(Debug, Clone)]
pub struct MemberList {
    members: HashMap<EndpointId, Member>,
}

impl Default for MemberList {
    fn default() -> Self {
        Self::new()
    }
}

impl MemberList {
    pub fn new() -> Self {
        Self {
            members: HashMap::new(),
        }
    }

    pub fn add(&mut self, member: Member) -> Result<(), IpCollision> {
        if let Some(existing) = self.get_by_ip(member.ip)
            && existing.identity != member.identity
        {
            return Err(IpCollision {
                ip: member.ip,
                existing_identity: existing.identity,
                new_identity: member.identity,
            });
        }
        self.members.insert(member.identity, member);
        Ok(())
    }

    pub fn remove(&mut self, identity: &EndpointId) -> Option<Member> {
        self.members.remove(identity)
    }

    pub fn get(&self, identity: &EndpointId) -> Option<&Member> {
        self.members.get(identity)
    }

    pub fn get_mut(&mut self, identity: &EndpointId) -> Option<&mut Member> {
        self.members.get_mut(identity)
    }

    pub fn get_by_ip(&self, ip: Ipv4Addr) -> Option<&Member> {
        self.members.values().find(|m| m.ip == ip)
    }

    pub fn is_member(&self, identity: &EndpointId) -> bool {
        self.members.contains_key(identity)
    }

    pub fn all(&self) -> Vec<&Member> {
        self.members.values().collect()
    }

    pub fn from_members(members: Vec<Member>) -> Self {
        let mut list = Self::new();
        for m in members {
            let _ = list.add(m);
        }
        list
    }
}

/// A peer that has been approved by the coordinator but hasn't connected yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_identity: Option<EndpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_cert: Option<DeviceCert>,
    /// Index used to resolve IPv4 collisions. Mirrors `Member.collision_index`
    /// for the same identity; defaults to 0 for backward-compatible decoding.
    #[serde(default)]
    pub collision_index: u32,
}

/// Pre-approved peers that the coordinator has broadcast but that haven't
/// connected yet. Any peer holding this list can welcome them.
#[derive(Debug, Clone)]
pub struct ApprovedList {
    entries: HashMap<EndpointId, ApprovedEntry>,
}

impl Default for ApprovedList {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovedList {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn approve(
        &mut self,
        entry: ApprovedEntry,
        members: &MemberList,
    ) -> Result<(), IpCollision> {
        if let Some(existing) = members.get_by_ip(entry.ip)
            && existing.identity != entry.identity
        {
            return Err(IpCollision {
                ip: entry.ip,
                existing_identity: existing.identity,
                new_identity: entry.identity,
            });
        }
        if let Some(existing) = self.get_by_ip(entry.ip)
            && existing.identity != entry.identity
        {
            return Err(IpCollision {
                ip: entry.ip,
                existing_identity: existing.identity,
                new_identity: entry.identity,
            });
        }
        self.entries.insert(entry.identity, entry);
        Ok(())
    }

    pub fn is_approved(&self, identity: &EndpointId) -> bool {
        self.entries.contains_key(identity)
    }

    pub fn remove(&mut self, identity: &EndpointId) -> Option<ApprovedEntry> {
        self.entries.remove(identity)
    }

    pub fn all(&self) -> Vec<&ApprovedEntry> {
        self.entries.values().collect()
    }

    pub fn get_by_ip(&self, ip: Ipv4Addr) -> Option<&ApprovedEntry> {
        self.entries.values().find(|e| e.ip == ip)
    }

    pub fn from_entries(entries: Vec<ApprovedEntry>) -> Self {
        let mut list = Self::new();
        for e in entries {
            list.entries.insert(e.identity, e);
        }
        list
    }
}

/// Determines whether a given member is allowed to approve new peers.
///
/// Superseded at runtime by the per-network access mode gate in the daemon
/// (open auto-admits; closed routes through the invite/approval flow); retained
/// for reference and unit coverage.
#[allow(dead_code)]
pub trait MembershipPolicy: Send + Sync {
    fn can_authorize(&self, acceptor: &Member) -> bool;
}

/// Any member can approve new peers.
#[allow(dead_code)]
pub struct OpenPolicy;

impl MembershipPolicy for OpenPolicy {
    fn can_authorize(&self, _acceptor: &Member) -> bool {
        true
    }
}

/// Only the coordinator can approve new peers.
#[allow(dead_code)]
pub struct RestrictedPolicy;

impl MembershipPolicy for RestrictedPolicy {
    fn can_authorize(&self, acceptor: &Member) -> bool {
        acceptor.is_coordinator
    }
}

#[allow(dead_code)]
pub fn policy_for_mode(mode: GroupMode) -> Box<dyn MembershipPolicy> {
    match mode {
        GroupMode::Open => Box::new(OpenPolicy),
        GroupMode::Restricted => Box::new(RestrictedPolicy),
    }
}

/// Flag an existing member as a coordinator (idempotent; no-op if absent).
pub fn mark_coordinator(members: &mut MemberList, identity: &EndpointId) {
    if let Some(m) = members.get_mut(identity) {
        m.is_coordinator = true;
    }
}

/// Abstracts identity and IP derivation so the membership system doesn't
/// depend directly on iroh types.
pub trait IdentityProvider: Send + Sync {
    fn local_ip(&self) -> Ipv4Addr;
    fn local_identity(&self) -> EndpointId;
    fn derive_ip(&self, peer_identity: &EndpointId) -> Ipv4Addr;
}

// ---------------------------------------------------------------------------
// Overlay subnet (configurable; see spec/design_spec.py SUBNET-*)
// ---------------------------------------------------------------------------

/// The overlay subnet as `(base address, prefix length)`. `None` in a
/// [`GroupBlob`] / config means "use [`default_subnet`]".
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
/// containment predicate shared by [`ensure_in_cgnat_range`] and the DNS
/// PTR/reverse-lookup range check, so the two can never diverge (SUBNET-008).
pub fn ip_in_subnet(ip: Ipv4Addr, subnet: Subnet) -> bool {
    let (base_addr, prefix) = subnet;
    let host_mask = subnet_host_mask(prefix);
    let base = u32::from(base_addr) & !host_mask;
    (u32::from(ip) & !host_mask) == base
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
            Some(cidr) => Ok(Some(super::parse_cidr(&cidr).map_err(serde::de::Error::custom)?)),
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

/// The reserved virtual IPv4 at a fixed host offset within `subnet`. Full
/// tetron runs its Magic DNS resolver here; tetron removed Magic DNS
/// (MINIMAL-012) but keeps the address reserved for **wire compatibility (D1)**:
/// a full-tetron node on a shared network routes this address to its own
/// resolver, so it must never be handed to a member. The `0x0064_6435` offset
/// reproduces the historical resolver address for the default subnet and yields
/// a stable, distinct in-subnet address for any /24-or-larger custom subnet.
pub fn magic_dns_v4(subnet: Subnet) -> Ipv4Addr {
    let (base_addr, prefix) = subnet;
    let host_mask = subnet_host_mask(prefix);
    let base = u32::from(base_addr) & !host_mask;
    let host = 0x0064_6435 & host_mask;
    Ipv4Addr::from(base | host)
}

/// True if `ip` is reserved and must never be assigned to a member
/// (currently the reserved resolver address, which is subnet-relative).
fn is_reserved_ipv4(ip: Ipv4Addr, subnet: Subnet) -> bool {
    ip == magic_dns_v4(subnet)
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
        if is_reserved_ipv4(ip, subnet) {
            index += 1;
            continue;
        }
        match members.get_by_ip(ip) {
            Some(existing) if existing.identity != *identity => index += 1,
            _ => return (ip, index),
        }
    }
}

/// Derives a stable IPv6 address from an [`EndpointId`] in the `200::/7` range.
/// Uses blake3 to hash the identity, takes 15 bytes, and prepends `0x02`.
/// The 120-bit address space makes collisions practically impossible.
pub fn derive_ipv6(identity: &EndpointId) -> Ipv6Addr {
    let hash = blake3::hash(identity.to_string().as_bytes());
    let bytes = hash.as_bytes();
    let octets: [u8; 16] = [
        0x02, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
    ];
    Ipv6Addr::from(octets)
}

/// [`IdentityProvider`] backed by an iroh [`EndpointId`].
#[derive(Clone)]
pub struct IrohIdentityProvider {
    endpoint_id: EndpointId,
    ip: Ipv4Addr,
    /// The node's operative overlay subnet (from `AppConfig.subnet` at
    /// bootstrap; [`default_subnet`] otherwise). Drives `local_ip` and every
    /// peer IP derived through this provider.
    subnet: Subnet,
}

impl IrohIdentityProvider {
    pub fn new(endpoint_id: EndpointId, collision_index: u32, subnet: Subnet) -> Self {
        let ip = derive_ip_with_index(&endpoint_id, collision_index, subnet);
        Self {
            endpoint_id,
            ip,
            subnet,
        }
    }

    /// The node's operative overlay subnet.
    pub fn subnet(&self) -> Subnet {
        self.subnet
    }
}

impl IdentityProvider for IrohIdentityProvider {
    fn local_ip(&self) -> Ipv4Addr {
        self.ip
    }

    fn local_identity(&self) -> EndpointId {
        self.endpoint_id
    }

    fn derive_ip(&self, peer_identity: &EndpointId) -> Ipv4Addr {
        derive_ip(peer_identity, self.subnet)
    }
}

// ---------------------------------------------------------------------------
// Canonical membership serialization + hashing
// ---------------------------------------------------------------------------

/// A reusable, expiring join key (Tailscale auth-key analog). Only the
/// `blake3(secret)` hash is published — the raw secret lives solely in the code
/// handed to a joiner. Because it rides the signed `GroupBlob`, *any* network-key
/// holder can verify-and-admit and revocation propagates to every admin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReusableKey {
    /// Short human id: the first 8 hex chars of the secret hash.
    pub id: String,
    /// Unix seconds when minted.
    pub created: u64,
    /// Unix seconds after which the key is no longer redeemable.
    pub expires: u64,
    /// Set by `tetron invite revoke`; a revoked key admits no one.
    pub revoked: bool,
}

/// A single-use invite entry carried in the signed `GroupBlob`.
///
/// Keyed in the blob by hex `blake3(secret)`, the same hash convention
/// [`ReusableKey`] uses. An `InviteEntry` is minted by any network-key holder
/// and validated by any network-key holder — no machine-local store required.
/// Once redeemed the entry is removed from the blob (the hash changes, the
/// updated blob propagates to every coordinator).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteEntry {
    /// Short human id: the first 8 hex chars of the secret hash.
    pub id: String,
    /// Unix seconds when minted.
    pub created: u64,
    /// Unix seconds after which the invite is no longer redeemable.
    /// `0` means permanent (never expires).
    pub expires: u64,
    /// Set by `tetron invite revoke`; a revoked invite admits no one.
    pub revoked: bool,
}

/// The single authoritative blob for a network, published by the coordinator.
/// Contains all state a joiner needs: members, the approved list, the
/// (wire-compat) suggested-firewall field, reusable join keys, and single-use
/// invite entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupBlob {
    pub members: Vec<Member>,
    pub approved: Vec<ApprovedEntry>,
    /// Coordinator-suggested firewall rules, keyed by subject hostname. Retained
    /// for wire compatibility with full tetron (D1): tetron removed the
    /// userspace firewall (MINIMAL-010), so it carries this field through the
    /// blob verbatim on republish but never acts on it. A full-tetron
    /// coordinator can still populate it. `BTreeMap` keys keep the encoding canonical.
    #[serde(default, skip_serializing_if = "SuggestedFirewall::is_empty")]
    pub suggested_firewall: SuggestedFirewall,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The network-wide overlay IPv4 subnet as `(base, prefix)`, serialized as a
    /// CIDR string. `None` means the [`default_subnet`] (10.88.0.0/24), keeping
    /// default-subnet networks byte-identical. This is the signed, network-wide
    /// source of truth every peer derives and validates addresses against.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "cidr_opt")]
    pub subnet: Option<Subnet>,
    /// Reusable join keys, keyed by hex `blake3(secret)`. `BTreeMap` keeps the
    /// encoding canonical; the secret hash commits to the signed hash, so adding
    /// or revoking a key changes the blob hash and triggers reconvergence.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reusable_keys: BTreeMap<String, ReusableKey>,
    /// Single-use invite entries, keyed by hex `blake3(secret)`. Validated by
    /// any network-key holder; redeemed entries are removed on republish.
    /// `BTreeMap` keeps the encoding canonical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub invites: BTreeMap<String, InviteEntry>,
}

impl ReusableKey {
    /// Build a reusable key from a freshly generated secret. Returns the map key
    /// (hex `blake3(secret)`) and the entry. `created`/`ttl_secs` are Unix seconds;
    /// the raw secret is the caller's to encode into the join code and discard.
    pub fn from_secret(secret: &[u8], created: u64, ttl_secs: u64) -> (String, ReusableKey) {
        let hash = blake3::hash(secret).to_hex().to_string();
        let id = hash[..8].to_string();
        (
            hash,
            ReusableKey {
                id,
                created,
                expires: created.saturating_add(ttl_secs),
                revoked: false,
            },
        )
    }
}

/// Revoke a reusable key by id (exact match, or unambiguous prefix), setting its
/// `revoked` flag. A revoked key stays in the blob (so the revocation is part of
/// the signed content and propagates) but admits no one.
pub fn revoke_reusable(keys: &mut BTreeMap<String, ReusableKey>, id: &str) -> Result<()> {
    let matches: Vec<String> = keys
        .iter()
        .filter(|(_, k)| k.id == id || k.id.starts_with(id))
        .map(|(hash, _)| hash.clone())
        .collect();
    let hash = match matches.as_slice() {
        [] => bail!("no reusable key matching '{id}'"),
        [h] => h.clone(),
        _ => bail!("ambiguous reusable key id '{id}'"),
    };
    keys.get_mut(&hash)
        .expect("hash came from this map")
        .revoked = true;
    Ok(())
}

/// Verify a presented reusable-key secret against a key set. Returns the key iff
/// it is present, not revoked, and not expired (`now` is Unix seconds). This is
/// the (pure) admission decision for a reusable join — usable by any network-key
/// holder, since the key set comes from the network-key-signed blob.
pub fn validate_reusable_key<'a>(
    keys: &'a BTreeMap<String, ReusableKey>,
    secret: &[u8],
    now: u64,
) -> Option<&'a ReusableKey> {
    let hash = blake3::hash(secret).to_hex().to_string();
    let key = keys.get(&hash)?;
    if key.revoked || now >= key.expires {
        return None;
    }
    Some(key)
}

impl InviteEntry {
    /// Build an invite entry from a freshly generated secret. Returns the map key
    /// (hex `blake3(secret)`) and the entry. `created`/`ttl_secs` are Unix seconds;
    /// `ttl_secs=0` means permanent (never expires). The raw secret is the caller's
    /// to encode into the invite code and discard.
    pub fn from_secret(secret: &[u8], created: u64, ttl_secs: u64) -> (String, InviteEntry) {
        let hash = blake3::hash(secret).to_hex().to_string();
        let id = hash[..8].to_string();
        (
            hash,
            InviteEntry {
                id,
                created,
                expires: if ttl_secs == 0 { 0 } else { created.saturating_add(ttl_secs) },
                revoked: false,
            },
        )
    }
}

/// Revoke an invite by id (exact match, or unambiguous prefix), setting its
/// `revoked` flag. A revoked invite stays in the blob (so the revocation
/// propagates) but admits no one.
pub fn revoke_invite(invites: &mut BTreeMap<String, InviteEntry>, id: &str) -> Result<()> {
    let matches: Vec<String> = invites
        .iter()
        .filter(|(_, k)| k.id == id || k.id.starts_with(id))
        .map(|(hash, _)| hash.clone())
        .collect();
    let hash = match matches.as_slice() {
        [] => bail!("no invite matching '{id}'"),
        [h] => h.clone(),
        _ => bail!("ambiguous invite id '{id}'"),
    };
    invites.get_mut(&hash)
        .expect("hash came from this map")
        .revoked = true;
    Ok(())
}

/// Validate a presented invite secret against an invite map. Returns the entry
/// iff it is present, not revoked, and not expired (`now` is Unix seconds;
/// `expires == 0` means permanent).
pub fn validate_invite<'a>(
    invites: &'a BTreeMap<String, InviteEntry>,
    secret: &[u8],
    now: u64,
) -> Option<&'a InviteEntry> {
    let hash = blake3::hash(secret).to_hex().to_string();
    let entry = invites.get(&hash)?;
    if entry.revoked || (entry.expires > 0 && now >= entry.expires) {
        return None;
    }
    Some(entry)
}

impl GroupBlob {
    /// Convenience wrapper over [`validate_reusable_key`] for a decoded blob.
    #[allow(dead_code)] // used in tests; the daemon calls the free function on NetworkState
    pub fn validate_reusable(&self, secret: &[u8], now: u64) -> Option<&ReusableKey> {
        validate_reusable_key(&self.reusable_keys, secret, now)
    }

    /// Convenience wrapper over [`validate_invite`] for a decoded blob.
    #[allow(dead_code)]
    pub fn validate_invite(&self, secret: &[u8], now: u64) -> Option<&InviteEntry> {
        validate_invite(&self.invites, secret, now)
    }
}

/// Produces a deterministic msgpack encoding of a group blob.
/// Members and approved entries are sorted by identity string to ensure
/// identical output regardless of HashMap iteration order; the suggested
/// firewall is a `BTreeMap`, so it is already canonically ordered.
pub fn canonical_group_bytes(
    members: &MemberList,
    approved: &ApprovedList,
    suggested_firewall: &SuggestedFirewall,
    name: Option<&str>,
    reusable_keys: &BTreeMap<String, ReusableKey>,
    subnet: Option<Subnet>,
    invites: &BTreeMap<String, InviteEntry>,
) -> Vec<u8> {
    let mut sorted_members: Vec<Member> = members.all().into_iter().cloned().collect();
    sorted_members.sort_by_key(|m| m.identity.to_string());

    let mut sorted_approved: Vec<ApprovedEntry> = approved.all().into_iter().cloned().collect();
    sorted_approved.sort_by_key(|a| a.identity.to_string());

    let data = GroupBlob {
        members: sorted_members,
        approved: sorted_approved,
        suggested_firewall: suggested_firewall.clone(),
        name: name.map(|s| s.to_string()),
        subnet,
        reusable_keys: reusable_keys.clone(),
        invites: invites.clone(),
    };
    rmp_serde::to_vec_named(&data).expect("msgpack serialize")
}

pub fn group_blob_hash(
    members: &MemberList,
    approved: &ApprovedList,
    suggested_firewall: &SuggestedFirewall,
    name: Option<&str>,
    reusable_keys: &BTreeMap<String, ReusableKey>,
    subnet: Option<Subnet>,
    invites: &BTreeMap<String, InviteEntry>,
) -> blake3::Hash {
    let bytes = canonical_group_bytes(
        members,
        approved,
        suggested_firewall,
        name,
        reusable_keys,
        subnet,
        invites,
    );
    blake3::hash(&bytes)
}

/// Validates that a [`Member`]'s virtual IP is consistent with its identity and
/// lies in the CGNAT range, excluding the reserved network (`.0`) and gateway
/// (`.1`) addresses.
///
/// This is the invariant the network *should* enforce at every trust boundary
/// (GroupBlob decode, `Welcome`/`MemberSync` application, `MeshHello.ip`). Today
/// the daemon trusts the `ip` field carried in those messages, which permits IP
/// hijacking — see the security audit. This helper exists so enforcement can be
/// added at the data layer without changing the on-wire format.
pub fn validate_member(member: &Member, subnet: Subnet) -> Result<()> {
    let expected = derive_ip_with_index(&member.identity, member.collision_index, subnet);
    anyhow::ensure!(
        member.ip == expected,
        "member ip {} does not match identity-derived ip {}",
        member.ip,
        expected,
    );
    anyhow::ensure!(
        !is_reserved_ipv4(member.ip, subnet),
        "member IP {} is the reserved Magic DNS address",
        member.ip
    );
    ensure_in_cgnat_range(member.ip, subnet)
}

/// Like [`validate_member`] but for [`ApprovedEntry`].
pub fn validate_approved(entry: &ApprovedEntry, subnet: Subnet) -> Result<()> {
    let expected = derive_ip_with_index(&entry.identity, entry.collision_index, subnet);
    anyhow::ensure!(
        entry.ip == expected,
        "approved entry ip {} does not match identity-derived ip {}",
        entry.ip,
        expected,
    );
    ensure_in_cgnat_range(entry.ip, subnet)
}

/// Returns `Err` if any two members share the same IPv4 address.
///
/// This enforces the roster invariant that every member has a unique IP.
/// Call this at any trust boundary where a freshly-decoded roster is applied.
pub fn validate_no_duplicate_ips(members: &[Member]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for m in members {
        anyhow::ensure!(seen.insert(m.ip), "duplicate IP {} in roster", m.ip);
    }
    Ok(())
}

/// Resolve duplicate-IP rosters deterministically: for each clashing IP the
/// lowest identity keeps it; others re-roll to their next free index.
///
/// Two coordinators can independently admit a fresh joiner at the same collision
/// index, so a reconverged roster may carry duplicate IPs. Sorting by identity
/// bytes and re-seating every member through [`assign_ip`] makes the resolution
/// order independent of where the roster was assembled, so every node converges
/// on the same address map.
pub fn resolve_ip_tiebreak(mut members: Vec<Member>, subnet: Subnet) -> Vec<Member> {
    members.sort_by_key(|m| m.identity.as_bytes().to_owned());
    let mut list = MemberList::new();
    for mut m in members {
        let (ip, idx) = assign_ip(&list, &m.identity, subnet);
        m.ip = ip;
        m.collision_index = idx;
        let _ = list.add(m);
    }
    list.all().into_iter().cloned().collect()
}

/// Validates that `ip` lies inside `subnet`, excluding the reserved network
/// (`base`) and gateway (`base + 1`) addresses.
fn ensure_in_cgnat_range(ip: Ipv4Addr, subnet: Subnet) -> Result<()> {
    let (base_addr, prefix) = subnet;
    let host_mask = subnet_host_mask(prefix);
    let base = u32::from(base_addr) & !host_mask;
    let ip_u = u32::from(ip);
    anyhow::ensure!(
        ip_in_subnet(ip, subnet),
        "ip {} is outside the configured overlay subnet {}/{}",
        ip,
        base_addr,
        prefix,
    );
    anyhow::ensure!(
        ip_u != base,
        "ip {} is the reserved network address",
        ip,
    );
    anyhow::ensure!(
        ip_u != base + 1,
        "ip {} is the reserved TUN gateway address",
        ip,
    );
    Ok(())
}

pub fn decode_group_blob(bytes: &[u8]) -> Result<GroupBlob> {
    let blob: GroupBlob =
        rmp_serde::from_slice(bytes).map_err(|e| anyhow::anyhow!("invalid group blob: {e}"))?;
    // Enforce the identity<->IP binding at the decode boundary against the
    // network's own configured subnet. Any blob that survives this check has
    // self-consistent members/approved entries, so a malicious or buggy
    // publisher cannot inject a spoofed or reserved IP.
    let subnet = resolve_subnet(blob.subnet);
    for m in &blob.members {
        validate_member(m, subnet)?;
    }
    for a in &blob.approved {
        validate_approved(a, subnet)?;
    }
    Ok(blob)
}

pub fn verify_group_blob(bytes: &[u8], expected_hash: &blake3::Hash) -> Result<GroupBlob> {
    let actual = blake3::hash(bytes);
    if actual != *expected_hash {
        bail!("group blob hash mismatch: expected {expected_hash}, got {actual}");
    }
    decode_group_blob(bytes)
}

/// Decides whether to reconverge the local group state, and to which hash.
///
/// The network-key-signed pkarr record is the *sole* authority: `signed` is the
/// hash it commits to. Peer control messages (`MemberSync`, `BlobUpdated`) are
/// payload-free triggers — they carry no hash — so there is never any
/// peer-supplied value that could be fetched or applied. Returns `Some(signed)`
/// when it differs from what we already hold (`current`), else `None`.
pub fn trusted_reconverge_hash(
    current: Option<blake3::Hash>,
    signed: blake3::Hash,
) -> Option<blake3::Hash> {
    if current == Some(signed) {
        None
    } else {
        Some(signed)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn subnet_change_warning_fires_only_on_mismatch() {
        use super::{default_subnet, subnet_change_warning};
        let live = default_subnet();
        let other = ("10.99.0.0".parse::<std::net::Ipv4Addr>().unwrap(), 16u8);
        assert!(subnet_change_warning(live, live).is_none());
        let w = subnet_change_warning(other, live).expect("mismatch must warn");
        assert!(w.contains("sudo tetron restart"), "{w}");
    }

    use super::*;
    use std::collections::BTreeMap;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = iroh::SecretKey::from(key_bytes);
        key.public()
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
        let reserved1 = Ipv4Addr::new(100, 64, 0, 0);
        let reserved2 = Ipv4Addr::new(100, 64, 0, 1);
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
            assert_eq!(derive_ip(&id, default_subnet()), derive_ip_with_index(&id, 0, default_subnet()));
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
        assert_eq!(derive_ipv6(&id), derive_ipv6(&id));
    }

    #[test]
    fn test_derive_ipv6_in_200_range() {
        for i in 0..=255u8 {
            let ipv6 = derive_ipv6(&test_id(i));
            let octets = ipv6.octets();
            assert_eq!(octets[0], 0x02, "first byte must be 0x02 for 200::/7");
        }
    }

    #[test]
    fn test_derive_ipv6_different_identities_differ() {
        let a = derive_ipv6(&test_id(1));
        let b = derive_ipv6(&test_id(2));
        assert_ne!(a, b);
    }

    #[test]
    fn test_iroh_identity_provider() {
        let key = iroh::SecretKey::generate();
        let endpoint_id = key.public();
        let provider = IrohIdentityProvider::new(endpoint_id, 0, default_subnet());

        let ip = provider.local_ip();
        assert!(ip_in_subnet(ip, default_subnet()));

        let id = provider.local_identity();
        assert_eq!(provider.derive_ip(&id), ip);
    }

    #[test]
    fn test_member_list_add_and_lookup() {
        let id = test_id(1);
        let mut list = MemberList::new();
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        list.add(member.clone()).unwrap();
        assert!(list.is_member(&id));
        assert!(!list.is_member(&test_id(2)));
        assert_eq!(list.get(&id).unwrap().ip, Ipv4Addr::new(100, 64, 10, 5));
    }

    #[test]
    fn test_member_list_lookup_by_ip() {
        let id = test_id(1);
        let mut list = MemberList::new();
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        list.add(member).unwrap();
        let found = list.get_by_ip(Ipv4Addr::new(100, 64, 10, 5)).unwrap();
        assert_eq!(found.identity, id);
        assert!(list.get_by_ip(Ipv4Addr::new(100, 64, 10, 6)).is_none());
    }

    #[test]
    fn test_member_list_ip_collision() {
        let mut list = MemberList::new();
        list.add(Member {
            identity: test_id(1),
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        let result = list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_member_list_same_identity_updates() {
        let id = test_id(1);
        let mut list = MemberList::new();
        list.add(Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        list.add(Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: true,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        assert!(list.get(&id).unwrap().is_coordinator);
    }

    #[test]
    fn test_member_list_remove() {
        let id = test_id(1);
        let mut list = MemberList::new();
        list.add(Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        let removed = list.remove(&id);
        assert!(removed.is_some());
        assert!(!list.is_member(&id));
        assert!(list.remove(&id).is_none());
    }

    #[test]
    fn test_member_list_all() {
        let mut list = MemberList::new();
        list.add(Member {
            identity: test_id(1),
            ip: Ipv4Addr::new(100, 64, 0, 2),
            is_coordinator: true,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 0, 3),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        assert_eq!(list.all().len(), 2);
    }

    #[test]
    fn test_open_policy_anyone_can_authorize() {
        let policy = OpenPolicy;
        let member = Member {
            identity: test_id(1),
            ip: Ipv4Addr::new(100, 64, 0, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(policy.can_authorize(&member));
    }

    #[test]
    fn test_restricted_policy_only_coordinators() {
        let policy = RestrictedPolicy;
        let coordinator = Member {
            identity: test_id(1),
            ip: Ipv4Addr::new(100, 64, 0, 2),
            is_coordinator: true,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let regular = Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 0, 3),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(policy.can_authorize(&coordinator));
        assert!(!policy.can_authorize(&regular));
    }

    #[test]
    fn test_approved_list_add_and_check() {
        let id = test_id(1);
        let mut list = ApprovedList::new();
        let entry = ApprovedEntry {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 5, 10),
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
        };
        let members = MemberList::new();
        list.approve(entry, &members).unwrap();
        assert!(list.is_approved(&id));
        assert!(!list.is_approved(&test_id(2)));
    }

    #[test]
    fn test_approved_list_collision_with_member() {
        let mut approved = ApprovedList::new();
        let mut members = MemberList::new();
        members
            .add(Member {
                identity: test_id(1),
                ip: Ipv4Addr::new(100, 64, 5, 10),
                is_coordinator: false,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        let entry = ApprovedEntry {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 5, 10),
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
        };
        assert!(approved.approve(entry, &members).is_err());
    }

    #[test]
    fn test_approved_list_collision_within_approved() {
        let mut approved = ApprovedList::new();
        let members = MemberList::new();
        approved
            .approve(
                ApprovedEntry {
                    identity: test_id(1),
                    ip: Ipv4Addr::new(100, 64, 5, 10),
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        let result = approved.approve(
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(100, 64, 5, 10),
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
            },
            &members,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_approved_list_same_identity_is_idempotent() {
        let id = test_id(1);
        let mut approved = ApprovedList::new();
        let members = MemberList::new();
        approved
            .approve(
                ApprovedEntry {
                    identity: id,
                    ip: Ipv4Addr::new(100, 64, 5, 10),
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        approved
            .approve(
                ApprovedEntry {
                    identity: id,
                    ip: Ipv4Addr::new(100, 64, 5, 10),
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        assert_eq!(approved.all().len(), 1);
    }

    #[test]
    fn test_approved_list_remove() {
        let id = test_id(1);
        let mut approved = ApprovedList::new();
        let members = MemberList::new();
        approved
            .approve(
                ApprovedEntry {
                    identity: id,
                    ip: Ipv4Addr::new(100, 64, 5, 10),
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        let removed = approved.remove(&id);
        assert!(removed.is_some());
        assert!(!approved.is_approved(&id));
    }

    #[test]
    fn test_approved_list_from_entries() {
        let entries = vec![
            ApprovedEntry {
                identity: test_id(1),
                ip: Ipv4Addr::new(100, 64, 0, 2),
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
            },
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(100, 64, 0, 3),
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
            },
        ];
        let list = ApprovedList::from_entries(entries);
        assert!(list.is_approved(&test_id(1)));
        assert!(list.is_approved(&test_id(2)));
        assert_eq!(list.all().len(), 2);
    }

    // -- Canonical serialization + hashing ------------------------------------

    fn make_member_list(seeds: &[u8]) -> MemberList {
        let mut list = MemberList::new();
        for &seed in seeds {
            let id = test_id(seed);
            let _ = list.add(Member {
                identity: id,
                ip: derive_ip(&id, default_subnet()),
                is_coordinator: false,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            });
        }
        list
    }

    #[test]
    fn test_canonical_bytes_deterministic() {
        let members = make_member_list(&[1, 2, 3]);
        let approved = ApprovedList::new();
        let a = canonical_group_bytes(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let b = canonical_group_bytes(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn test_canonical_bytes_order_independent() {
        let m1 = make_member_list(&[1, 2, 3]);
        let m2 = make_member_list(&[3, 1, 2]);
        let approved = ApprovedList::new();
        assert_eq!(
            canonical_group_bytes(
                &m1,
                &approved,
                &tetron_proto::SuggestedFirewall::default(),
                None,
                &BTreeMap::new(), None,
                &BTreeMap::new(),
            ),
            canonical_group_bytes(
                &m2,
                &approved,
                &tetron_proto::SuggestedFirewall::default(),
                None,
                &BTreeMap::new(), None,
                &BTreeMap::new(),
            ),
        );
    }

    #[test]
    fn test_group_blob_hash_changes_on_mutation() {
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let h1 = group_blob_hash(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let members2 = make_member_list(&[1, 2, 3]);
        let h2 = group_blob_hash(
            &members2,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_group_blob_roundtrip() {
        let members = make_member_list(&[1, 2]);
        let mut approved = ApprovedList::new();
        let id3 = test_id(3);
        approved
            .approve(
                ApprovedEntry {
                    identity: id3,
                    ip: derive_ip(&id3, default_subnet()),
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();

        let bytes = canonical_group_bytes(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let data = decode_group_blob(&bytes).unwrap();
        assert_eq!(data.members.len(), 2);
        assert_eq!(data.approved.len(), 1);
    }

    #[test]
    fn test_verify_group_blob_ok() {
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let bytes = canonical_group_bytes(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let hash = group_blob_hash(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let data = verify_group_blob(&bytes, &hash).unwrap();
        assert_eq!(data.members.len(), 2);
    }

    #[test]
    fn no_reconverge_when_already_on_signed_hash() {
        // We already hold the authoritative (signed) blob — no work to do.
        let signed = blake3::hash(b"authoritative blob");
        assert_eq!(trusted_reconverge_hash(Some(signed), signed), None);
    }

    #[test]
    fn reconverge_targets_signed_hash_on_change() {
        // The signed record changed. We reconverge to the SIGNED hash.
        let current = blake3::hash(b"old blob");
        let signed = blake3::hash(b"new authoritative blob");
        assert_eq!(trusted_reconverge_hash(Some(current), signed), Some(signed));
    }

    #[test]
    fn reconverge_applies_signed_hash_when_no_current() {
        let signed = blake3::hash(b"authoritative blob");
        assert_eq!(trusted_reconverge_hash(None, signed), Some(signed));
    }

    #[test]
    fn test_verify_group_blob_bad_hash() {
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let bytes = canonical_group_bytes(
            &members,
            &approved,
            &tetron_proto::SuggestedFirewall::default(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let bad_hash = blake3::hash(b"wrong data");
        let result = verify_group_blob(&bytes, &bad_hash);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash mismatch"));
    }

    #[test]
    fn last_seen_survives_blob_roundtrip() {
        let id = test_id(7);
        let mut members = MemberList::new();
        members
            .add(Member {
                identity: id,
                ip: derive_ip(&id, default_subnet()),
                is_coordinator: false,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: Some(12345),
            })
            .unwrap();
        let approved = ApprovedList::new();
        let sf = tetron_proto::SuggestedFirewall::default();
        let bytes = canonical_group_bytes(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        let hash = group_blob_hash(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        let data = verify_group_blob(&bytes, &hash).unwrap();
        assert_eq!(data.members[0].last_seen, Some(12345));
    }

    #[test]
    fn last_seen_absent_decodes_to_none() {
        // A member with no last_seen serializes WITHOUT the field
        // (skip_serializing_if), exactly like a blob published before the field
        // existed; it must decode to None with no mass eviction on upgrade.
        let id = test_id(8);
        let mut members = MemberList::new();
        members
            .add(Member {
                identity: id,
                ip: derive_ip(&id, default_subnet()),
                is_coordinator: false,
                hostname: None,
                user_identity: None,
                device_cert: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        let approved = ApprovedList::new();
        let sf = tetron_proto::SuggestedFirewall::default();
        let bytes = canonical_group_bytes(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        assert!(!String::from_utf8_lossy(&bytes).contains("last_seen"));
        let hash = group_blob_hash(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        let data = verify_group_blob(&bytes, &hash).unwrap();
        assert_eq!(data.members[0].last_seen, None);
    }

    #[test]
    fn test_suggested_firewall_canonical_and_hashed() {
        use tetron_proto::{HostSuggestions, SuggestedFirewall};
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let mut sf = SuggestedFirewall::new();
        let mut hs = HostSuggestions::default();
        hs.allows
            .insert("peer-a".to_string(), "9000,8123".to_string());
        sf.insert("subject".to_string(), hs);

        // Deterministic: BTreeMap keys canonicalize regardless of insert order.
        let a = canonical_group_bytes(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        let b = canonical_group_bytes(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        assert_eq!(a, b);

        // Suggestions are part of the signed content, so they change the hash.
        let h_empty = group_blob_hash(
            &members,
            &approved,
            &SuggestedFirewall::new(),
            None,
            &BTreeMap::new(), None,
            &BTreeMap::new(),
        );
        let h_sf = group_blob_hash(&members, &approved, &sf, None, &BTreeMap::new(), None, &BTreeMap::new());
        assert_ne!(h_empty, h_sf);
    }

    #[test]
    fn test_old_blob_without_suggested_firewall_decodes() {
        // A blob serialized before suggested firewall existed (no
        // `suggested_firewall` key) must still decode, defaulting it empty.
        #[derive(Serialize)]
        struct OldBlob {
            members: Vec<Member>,
            approved: Vec<ApprovedEntry>,
            name: Option<String>,
        }
        let members = make_member_list(&[1, 2]);
        let old = OldBlob {
            members: members.all().into_iter().cloned().collect(),
            approved: vec![],
            name: Some("net".to_string()),
        };
        let bytes = rmp_serde::to_vec_named(&old).unwrap();
        let blob = decode_group_blob(&bytes).unwrap();
        assert_eq!(blob.members.len(), 2);
        assert!(blob.suggested_firewall.is_empty());
        // A pre-reusable-keys blob decodes with an empty reusable_keys map.
        assert!(blob.reusable_keys.is_empty());
    }

    // -- reusable keys --------------------------------------------------------

    fn reusable_key_for(secret: &[u8], expires: u64, revoked: bool) -> (String, ReusableKey) {
        let hash = blake3::hash(secret).to_hex().to_string();
        let id = hash[..8].to_string();
        (
            hash,
            ReusableKey {
                id,
                created: 0,
                expires,
                revoked,
            },
        )
    }

    #[test]
    fn reusable_key_blob_roundtrips() {
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let secret = [7u8; 16];
        let (hash, key) = reusable_key_for(&secret, 9_999_999_999, false);
        let mut keys = BTreeMap::new();
        keys.insert(hash, key);

        let bytes = canonical_group_bytes(
            &members,
            &approved,
            &SuggestedFirewall::default(),
            None,
            &keys, None,
            &BTreeMap::new(),
        );
        let blob = decode_group_blob(&bytes).unwrap();
        assert_eq!(blob.reusable_keys.len(), 1);
        // The decoded blob validates the secret it was built with.
        assert!(blob.validate_reusable(&secret, 1000).is_some());
    }

    #[test]
    fn reusable_key_changes_hash_when_added_or_revoked() {
        let members = make_member_list(&[1]);
        let approved = ApprovedList::new();
        let empty = BTreeMap::new();
        let h0 = group_blob_hash(
            &members,
            &approved,
            &SuggestedFirewall::default(),
            None,
            &empty, None,
            &BTreeMap::new(),
        );

        let secret = [3u8; 16];
        let (hash, key) = reusable_key_for(&secret, 9_999_999_999, false);
        let mut keys = BTreeMap::new();
        keys.insert(hash.clone(), key);
        let h1 = group_blob_hash(
            &members,
            &approved,
            &SuggestedFirewall::default(),
            None,
            &keys, None,
            &BTreeMap::new(),
        );
        assert_ne!(h0, h1, "adding a reusable key must change the signed hash");

        // Revoking is a content change → the hash must change again so peers reconverge.
        keys.get_mut(&hash).unwrap().revoked = true;
        let h2 = group_blob_hash(
            &members,
            &approved,
            &SuggestedFirewall::default(),
            None,
            &keys, None,
            &BTreeMap::new(),
        );
        assert_ne!(
            h1, h2,
            "revoking a reusable key must change the signed hash"
        );
    }

    #[test]
    fn reusable_key_from_secret_sets_id_and_expiry() {
        let secret = [5u8; 16];
        let (hash, key) = ReusableKey::from_secret(&secret, 100, 50);
        assert_eq!(hash, blake3::hash(&secret).to_hex().to_string());
        assert_eq!(key.id, hash[..8]);
        assert_eq!(key.created, 100);
        assert_eq!(key.expires, 150);
        assert!(!key.revoked);
    }

    #[test]
    fn revoke_reusable_by_full_id_and_prefix() {
        let secret = [6u8; 16];
        let (hash, key) = ReusableKey::from_secret(&secret, 0, 100);
        let mut keys = BTreeMap::new();
        keys.insert(hash.clone(), key.clone());
        // Full id.
        revoke_reusable(&mut keys, &key.id).unwrap();
        assert!(keys[&hash].revoked);
        // Unambiguous prefix.
        keys.get_mut(&hash).unwrap().revoked = false;
        revoke_reusable(&mut keys, &key.id[..4]).unwrap();
        assert!(keys[&hash].revoked);
    }

    #[test]
    fn revoke_reusable_unknown_and_ambiguous_error() {
        let mut empty: BTreeMap<String, ReusableKey> = BTreeMap::new();
        assert!(revoke_reusable(&mut empty, "deadbeef").is_err());

        let mut keys = BTreeMap::new();
        keys.insert(
            "h1".to_string(),
            ReusableKey {
                id: "abcd0000".to_string(),
                created: 0,
                expires: 100,
                revoked: false,
            },
        );
        keys.insert(
            "h2".to_string(),
            ReusableKey {
                id: "abcd1111".to_string(),
                created: 0,
                expires: 100,
                revoked: false,
            },
        );
        assert!(
            revoke_reusable(&mut keys, "abcd").is_err(),
            "prefix matching two ids is ambiguous"
        );
    }

    #[test]
    fn validate_reusable_accepts_live_rejects_expired_revoked_unknown() {
        let secret = [9u8; 16];
        let mk = |expires, revoked| {
            let (hash, key) = reusable_key_for(&secret, expires, revoked);
            let mut keys = BTreeMap::new();
            keys.insert(hash, key);
            GroupBlob {
                members: vec![],
                approved: vec![],
                suggested_firewall: SuggestedFirewall::default(),
                name: None,
                subnet: None,
                reusable_keys: keys,
                invites: BTreeMap::new(),
            }
        };
        // Live key: present, not revoked, now < expires.
        assert!(mk(100, false).validate_reusable(&secret, 50).is_some());
        // Expired: now >= expires.
        assert!(mk(100, false).validate_reusable(&secret, 100).is_none());
        // Revoked.
        assert!(mk(100, true).validate_reusable(&secret, 50).is_none());
        // Unknown secret.
        assert!(mk(100, false).validate_reusable(&[0u8; 16], 50).is_none());
    }

    // -- validate_member / validate_approved ---------------------------------

    #[test]
    fn validate_member_accepts_consistent_ip() {
        let id = test_id(7);
        let member = Member {
            identity: id,
            ip: derive_ip(&id, default_subnet()),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&member, default_subnet()).is_ok());
    }

    #[test]
    fn validate_member_rejects_mismatched_ip() {
        // A peer/ coordinator must not be able to assign an arbitrary IP to an
        // identity. This is the invariant that prevents IP hijacking.
        let id = test_id(7);
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5), // does NOT equal derive_ip(test_id(7))
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let err = validate_member(&member, default_subnet()).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn validate_member_rejects_out_of_range_ip() {
        let id = test_id(7);
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 0, 0, 5),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&member, default_subnet()).is_err());
    }

    #[test]
    fn validate_member_rejects_reserved_addresses() {
        // .0 (network) and .1 (gateway) are reserved even if derive_ip avoids them.
        let id = test_id(7);
        let net = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 0, 0),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let gw = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 0, 1),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&net, default_subnet()).is_err());
        assert!(validate_member(&gw, default_subnet()).is_err());
    }

    #[test]
    fn validate_approved_rejects_mismatched_ip() {
        let id = test_id(9);
        let entry = ApprovedEntry {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 99, 99),
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
        };
        assert!(validate_approved(&entry, default_subnet()).is_err());
    }

    #[test]
    fn validate_member_accepts_all_derived_ips_in_range() {
        // Every derive_ip() output for a spread of identities must pass validation,
        // as long as the IP does not collide with a reserved address (.0, .1, or
        // the subnet-relative magic-DNS address). With the default /24 there are 253
        // usable host addresses; we skip seeds that land on one of the 3 reserved.
        let mut validated = 0u32;
        let mut seed: u8 = 0;
        loop {
            let id = test_id(seed);
            let ip = derive_ip(&id, default_subnet());
            if !is_reserved_ipv4(ip, default_subnet())
                && ip != Ipv4Addr::new(10, 88, 0, 0)  // network
                && ip != Ipv4Addr::new(10, 88, 0, 1)  // gateway
            {
                let member = Member {
                    identity: id,
                    ip,
                    is_coordinator: false,
                    hostname: None,
                    user_identity: None,
                    device_cert: None,
                    collision_index: 0,
                    last_seen: None,
                };
                assert!(
                    validate_member(&member, default_subnet()).is_ok(),
                    "seed {seed} -> {}",
                    member.ip
                );
                validated += 1;
                if validated >= 200 {
                    break;
                }
            }
            seed = seed.wrapping_add(1);
        }
    }

    #[test]
    fn decode_group_blob_rejects_mismatched_member_ip() {
        // A tampered blob carrying a member whose IP doesn't match its identity
        // must be rejected at the decode boundary, even if the bytes are
        // otherwise valid msgpack.
        let id = test_id(1);
        let bad_member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5), // not derive_ip(test_id(1))
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let blob = GroupBlob {
            members: vec![bad_member],
            approved: vec![],
            suggested_firewall: Default::default(),
            name: None,
            subnet: None,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
        };
        let bytes = rmp_serde::to_vec_named(&blob).unwrap();
        let err = decode_group_blob(&bytes).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn decode_group_blob_rejects_reserved_gateway_ip() {
        let id = test_id(2);
        let bad_member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 0, 1), // TUN gateway
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let blob = GroupBlob {
            members: vec![bad_member],
            approved: vec![],
            suggested_firewall: Default::default(),
            name: None,
            subnet: None,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
        };
        let bytes = rmp_serde::to_vec_named(&blob).unwrap();
        assert!(decode_group_blob(&bytes).is_err());
    }

    #[test]
    fn mark_coordinator_sets_flag_for_target() {
        let id = test_id(7);
        let mut list = MemberList::new();
        list.add(Member {
            identity: id,
            ip: derive_ip(&id, default_subnet()),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        mark_coordinator(&mut list, &id);
        assert!(list.get(&id).unwrap().is_coordinator);
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
    fn validate_member_accepts_declared_index_rejects_mismatch() {
        let id = test_id(5);
        let good = Member {
            identity: id,
            ip: derive_ip_with_index(&id, 2, default_subnet()),
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 2,
            last_seen: None,
        };
        assert!(validate_member(&good, default_subnet()).is_ok());
        let bad = Member {
            collision_index: 1,
            last_seen: None,
            ..good.clone()
        }; // ip is for index 2, claims 1
        assert!(validate_member(&bad, default_subnet()).is_err());
    }

    #[test]
    fn validate_no_duplicate_ips_rejects_clash() {
        let a = test_id(1);
        let m = |id, ip| Member {
            identity: id,
            ip,
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let dup = derive_ip(&a, default_subnet());
        assert!(validate_no_duplicate_ips(&[m(a, dup), m(test_id(2), dup)]).is_err());
    }

    #[test]
    fn assign_ip_rotates_on_collision() {
        let (a, b) = find_colliding_pair()
            .expect("birthday bound: should find a collision within 200k identities");
        // Sanity: a and b both map to the same index-0 IP.
        assert_eq!(derive_ip(&a, default_subnet()), derive_ip(&b, default_subnet()));
        let ip0 = derive_ip(&a, default_subnet());

        // Add `a` to the list at its index-0 IP.
        let mut list = MemberList::new();
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

    #[test]
    fn tiebreak_keeps_lower_identity_rerolls_other() {
        // Order two distinct identities by their canonical byte order so the
        // assertion ("lower identity keeps the shared ip") is deterministic
        // regardless of how the seeds map onto public keys.
        let (lo, hi) = {
            let (a, b) = (test_id(1), test_id(9));
            if a.as_bytes() <= b.as_bytes() {
                (a, b)
            } else {
                (b, a)
            }
        };
        let ip = derive_ip(&lo, default_subnet()); // both initially claim this ip at index 0
        let mk = |id| Member {
            identity: id,
            ip,
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            collision_index: 0,
            last_seen: None,
        };
        let resolved = resolve_ip_tiebreak(vec![mk(hi), mk(lo)], default_subnet());
        // lower identity keeps `ip`; higher re-rolls to a free index.
        let lo_m = resolved.iter().find(|m| m.identity == lo).unwrap();
        let hi_m = resolved.iter().find(|m| m.identity == hi).unwrap();
        assert_eq!(lo_m.ip, ip);
        assert_ne!(hi_m.ip, ip);
        assert!(validate_no_duplicate_ips(&resolved).is_ok());
    }

    #[test]
    fn is_reserved_ipv4_covers_magic_dns() {
        // The predicate test isolates the guard: it fails if anyone removes the
        // magic DNS IP from the reserved set, independent of IP-derivation.
        assert!(is_reserved_ipv4(magic_dns_v4(default_subnet()), default_subnet()));
        assert!(!is_reserved_ipv4(Ipv4Addr::new(100, 64, 0, 7), default_subnet()));
    }

    #[test]
    fn validate_member_rejects_magic_dns_ip() {
        // Behavioral guard; the predicate test above is the one that isolates it.
        let mut kb = [0u8; 32];
        kb[0] = 9;
        let id = iroh::SecretKey::from(kb).public();
        let m = Member {
            identity: id,
            ip: magic_dns_v4(default_subnet()),
            collision_index: 0,
            is_coordinator: false,
            hostname: None,
            user_identity: None,
            device_cert: None,
            last_seen: None,
        };
        assert!(validate_member(&m, default_subnet()).is_err());
    }

    // --- configurable subnet (SUBNET-003/004/005/007) ------------------------

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
        assert_eq!(subnet_gateway(default_subnet()), Ipv4Addr::new(10, 88, 0, 1));
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
        assert!(!subnets_overlap((Ipv4Addr::new(192, 168, 1, 5), 24), overlay));
        // Crucially, Tailscale's 100.64.0.0/10 does NOT overlap 10.88.0.0/24 —
        // this is the whole point of the safe default.
        assert!(!subnets_overlap((Ipv4Addr::new(100, 64, 0, 1), 10), overlay));
    }

    #[test]
    fn ensure_in_range_respects_custom_subnet() {
        // In-subnet host is accepted.
        assert!(ensure_in_cgnat_range(Ipv4Addr::new(10, 99, 5, 9), CUSTOM).is_ok());
        // Network + gateway are rejected.
        assert!(ensure_in_cgnat_range(Ipv4Addr::new(10, 99, 0, 0), CUSTOM).is_err());
        assert!(ensure_in_cgnat_range(Ipv4Addr::new(10, 99, 0, 1), CUSTOM).is_err());
        // A 100.64.0.0/10 (Tailscale/legacy) address is outside the custom subnet.
        assert!(ensure_in_cgnat_range(Ipv4Addr::new(100, 64, 0, 5), CUSTOM).is_err());
    }

    #[test]
    fn magic_dns_is_subnet_relative_and_back_compatible() {
        // For 100.64.0.0/10 the offset reproduces the historical 100.100.100.53
        // resolver address — a property of that range, not of the default.
        assert_eq!(
            magic_dns_v4((Ipv4Addr::new(100, 64, 0, 0), 10)),
            Ipv4Addr::new(100, 100, 100, 53)
        );
        // For any subnet it stays inside the range.
        assert!(ip_in_subnet(magic_dns_v4(CUSTOM), CUSTOM));
    }

    #[test]
    fn parse_cidr_roundtrips_and_rejects_garbage() {
        assert_eq!(parse_cidr("10.99.0.0/16").unwrap(), CUSTOM);
        assert!(parse_cidr("10.99.0.0").is_err());
        assert!(parse_cidr("not-an-ip/16").is_err());
        assert!(parse_cidr("10.0.0.0/33").is_err());
    }

    #[test]
    fn group_blob_subnet_survives_roundtrip() {
        let members = make_member_list_in(&[1, 2], CUSTOM);
        let bytes = canonical_group_bytes(
            &members,
            &ApprovedList::new(),
            &SuggestedFirewall::default(),
            None,
            &BTreeMap::new(),
            Some(CUSTOM),
            &BTreeMap::new(),
        );
        let blob = decode_group_blob(&bytes).unwrap();
        assert_eq!(resolve_subnet(blob.subnet), CUSTOM);
    }

    /// Like [`make_member_list`] but derives IPs in `subnet`.
    fn make_member_list_in(seeds: &[u8], subnet: Subnet) -> MemberList {
        let mut list = MemberList::new();
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
}
