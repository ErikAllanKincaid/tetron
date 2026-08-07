//! Network membership management: member/approved rosters, the canonical
//! `GroupBlob` format, nuke consensus, and invite/reusable-key validation.
//!
//! Overlay IP derivation and the configurable subnet live in
//! [`crate::addressing`] (MODULARIZE-001); identity abstraction lives in
//! [`crate::identity`]; the invite/reusable-key *types* live in
//! [`crate::invite`] (alongside the invite-code encoding they're minted
//! into). All are re-exported below so existing `crate::membership::…`
//! paths keep working unchanged.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::net::Ipv4Addr;

use anyhow::{Result, bail};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};

/// Overlay addressing (Subnet, IP derivation, IPv6) — moved to
/// `crate::addressing` (MODULARIZE-001); re-exported so every existing
/// `crate::membership::Subnet`/`derive_ip`/… path keeps compiling.
pub use crate::addressing::{
    IPV6_NETWORK_PREFIX_LEN, Subnet, assign_ip, cidr_opt, default_subnet, derive_ip,
    derive_ip_with_index, derive_ipv6, ip_in_subnet, ipv6_in_network, ipv6_network_prefix,
    next_available_subnet, parse_cidr, resolve_subnet, subnet_change_warning, subnet_gateway,
    subnet_host_mask, subnet_netmask, subnets_overlap, validate_subnet_matches_roster,
};

/// Identity abstraction — moved to `crate::identity` (MODULARIZE-001);
/// re-exported so every existing `crate::membership::IdentityProvider`/
/// `IrohIdentityProvider` path keeps compiling.
pub use crate::identity::{IdentityProvider, IrohIdentityProvider};

/// Invite/reusable-key record types — moved to `crate::invite`
/// (MODULARIZE-001), consolidated with the invite-code encoding logic that
/// already lived there; re-exported so every existing
/// `crate::membership::InviteEntry`/`ReusableKey` path keeps compiling.
pub use crate::invite::{InviteEntry, ReusableKey};

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
/// Defined in `tetron-proto` (shared with GUI frontends); re-exported here so
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

/// Flag an existing member as a coordinator (idempotent; no-op if absent).
pub fn mark_coordinator(members: &mut MemberList, identity: &EndpointId) {
    if let Some(m) = members.get_mut(identity) {
        m.is_coordinator = true;
    }
}

// ---------------------------------------------------------------------------
// Canonical membership serialization + hashing
// ---------------------------------------------------------------------------

/// The single authoritative blob for a network, published by the coordinator.
/// Contains all state a joiner needs: members, the approved list, reusable
/// join keys, and single-use invite entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupBlob {
    /// Monotonic version, incremented on every local content mutation (admit,
    /// kick, invite create/revoke, admin grant, ...). Lets any node tell a
    /// genuinely newer blob from an objectively-stale one regardless of DHT
    /// write order — plain hash comparison cannot do this (CONVERGE-005).
    /// `#[serde(default)]` keeps pre-generation blobs decodable as generation 0.
    #[serde(default)]
    pub generation: u64,
    pub members: Vec<Member>,
    pub approved: Vec<ApprovedEntry>,
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
    /// Pending nuke proposals (NUKE-CONSENSUS), keyed by the proposing
    /// coordinator's full identity string (not the short id — a map key must be
    /// collision-free, and two coordinators' short ids could theoretically
    /// collide; short ids are used only for CLI display/matching). Value is the
    /// Unix-seconds timestamp of the proposal. `tetron nuke` on a network with
    /// exactly one coordinator nukes immediately (unchanged legacy behavior);
    /// with two or more, it adds an entry here instead, and any coordinator
    /// that observes two or more distinct, unexpired proposers executes the
    /// actual nuke. `BTreeMap` keeps the encoding canonical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nuke_proposals: BTreeMap<String, u64>,
    /// Minimum distinct, unexpired proposers required to execute a nuke once
    /// this network has 2+ coordinators (NUKE-CONSENSUS-THRESHOLD-001). Fixed
    /// at `tetron create --nuke-consensus <n>` (default 2), never mutated
    /// afterward -- no CLI path changes it post-creation, the same
    /// immutable-after-create treatment as `subnet`. Always persisted
    /// explicitly on every write from this code (SUBNET-DRIFT-001's lesson
    /// applied up front rather than repeated): `#[serde(default = ...)]`
    /// exists only so a blob predating this field decodes as the historical
    /// hardcoded value of 2, not because a fresh write should ever omit it.
    #[serde(default = "default_nuke_consensus_threshold")]
    pub nuke_consensus_threshold: u32,
}

/// How long a nuke proposal remains valid (NUKE-CONSENSUS). A proposal older
/// than this no longer counts toward consensus, so a stale entry from months
/// ago can't silently combine with a fresh one to trigger an unintended nuke.
/// Compiled default; overridable via `tetron config set nuke-proposal-ttl
/// <duration>` (CONFIG-AUDIT-002) — callers read `config::AppConfig::
/// nuke_proposal_ttl` and fall back to this constant, rather than the
/// functions below reading it internally, so the TTL used is a live config
/// snapshot rather than baked in at compile time.
pub const NUKE_PROPOSAL_TTL_SECS: u64 = 24 * 60 * 60;

/// The historical, hardcoded NUKE-CONSENSUS threshold, kept as the default for
/// `tetron create` when `--nuke-consensus` is omitted, and for decoding a blob
/// published before `nuke_consensus_threshold` existed.
pub fn default_nuke_consensus_threshold() -> u32 {
    2
}

/// Count coordinators in a roster (NUKE-CONSENSUS uses this to decide whether
/// a nuke needs consensus at all — a solo coordinator has no one to second).
pub fn coordinator_count(members: &[Member]) -> usize {
    members.iter().filter(|m| m.is_coordinator).count()
}

/// The proposer identities in `proposals` whose entry has not expired, as of
/// `now` (Unix seconds). `ttl_secs` is normally [`NUKE_PROPOSAL_TTL_SECS`] or
/// the `nuke-proposal-ttl` config override (CONFIG-AUDIT-002) — callers
/// resolve which before calling in, rather than this function reading config
/// itself, to keep it a pure function of its arguments.
pub fn active_nuke_proposers(proposals: &BTreeMap<String, u64>, now: u64, ttl_secs: u64) -> Vec<&String> {
    proposals
        .iter()
        .filter(|&(_, &proposed_at)| now.saturating_sub(proposed_at) < ttl_secs)
        .map(|(id, _)| id)
        .collect()
}

/// Whether `proposals` currently has enough distinct, unexpired proposers to
/// execute a nuke against `threshold` (NUKE-CONSENSUS-THRESHOLD-001;
/// previously a hardcoded 2).
pub fn nuke_consensus_reached(
    proposals: &BTreeMap<String, u64>,
    now: u64,
    threshold: u32,
    ttl_secs: u64,
) -> bool {
    active_nuke_proposers(proposals, now, ttl_secs).len() >= threshold as usize
}

/// Resolve `tetron nuke --second <short-id>` against the currently active
/// (unexpired) proposers: exact match or unambiguous prefix, mirroring
/// [`revoke_invite`]/[`revoke_reusable`]'s id-matching convention. Proposal
/// keys are full identity strings (not short ids, to keep the map key
/// collision-free), so a short id is matched as a string prefix — the same
/// relationship `EndpointId::fmt_short()` has to the full identity string.
pub fn resolve_nuke_proposer(
    proposals: &BTreeMap<String, u64>,
    now: u64,
    ttl_secs: u64,
    short: &str,
) -> Result<String> {
    let matches: Vec<&String> = active_nuke_proposers(proposals, now, ttl_secs)
        .into_iter()
        .filter(|id| id.starts_with(short))
        .collect();
    match matches.as_slice() {
        [] => bail!("no active nuke proposal from a coordinator matching '{short}'"),
        [id] => Ok((*id).clone()),
        _ => bail!("ambiguous proposer id '{short}'"),
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
    invites
        .get_mut(&hash)
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
/// identical output regardless of HashMap iteration order.
#[allow(clippy::too_many_arguments)]
pub fn canonical_group_bytes(
    generation: u64,
    members: &MemberList,
    approved: &ApprovedList,
    name: Option<&str>,
    reusable_keys: &BTreeMap<String, ReusableKey>,
    subnet: Option<Subnet>,
    invites: &BTreeMap<String, InviteEntry>,
    nuke_proposals: &BTreeMap<String, u64>,
    nuke_consensus_threshold: u32,
) -> Vec<u8> {
    let mut sorted_members: Vec<Member> = members.all().into_iter().cloned().collect();
    sorted_members.sort_by_key(|m| m.identity.to_string());

    let mut sorted_approved: Vec<ApprovedEntry> = approved.all().into_iter().cloned().collect();
    sorted_approved.sort_by_key(|a| a.identity.to_string());

    let data = GroupBlob {
        generation,
        members: sorted_members,
        approved: sorted_approved,
        name: name.map(|s| s.to_string()),
        subnet,
        reusable_keys: reusable_keys.clone(),
        invites: invites.clone(),
        nuke_proposals: nuke_proposals.clone(),
        nuke_consensus_threshold,
    };
    rmp_serde::to_vec_named(&data).expect("msgpack serialize")
}

#[allow(clippy::too_many_arguments)]
pub fn group_blob_hash(
    generation: u64,
    members: &MemberList,
    approved: &ApprovedList,
    name: Option<&str>,
    reusable_keys: &BTreeMap<String, ReusableKey>,
    subnet: Option<Subnet>,
    invites: &BTreeMap<String, InviteEntry>,
    nuke_proposals: &BTreeMap<String, u64>,
    nuke_consensus_threshold: u32,
) -> blake3::Hash {
    let bytes = canonical_group_bytes(
        generation,
        members,
        approved,
        name,
        reusable_keys,
        subnet,
        invites,
        nuke_proposals,
        nuke_consensus_threshold,
    );
    blake3::hash(&bytes)
}

/// Reconstruct and verify a nuke tombstone locally, without fetching from a
/// peer. A tombstone's content is fully deterministic given just its
/// `generation` (empty members/approved/reusable_keys/invites/nuke_proposals,
/// no name, default subnet/firewall — see
/// `MeshManager::publish_nuke_tombstone`), so any node that resolves the
/// signed `(hash, generation)` pair can recompute the exact same bytes and
/// check them against `signed` itself.
///
/// This matters because the coordinator that publishes a tombstone calls
/// `leave_network` immediately after (closing its connections), so it is
/// typically the *only* node that ever held the blob bytes in its local
/// store, and is gone as a fetch source by the time anyone else's
/// reconverge/poller notices the generation bump. Found via live testing,
/// 2026-07-17: `resolve_network` correctly signaled the new generation to
/// remaining members, but every `fetch_verified_blob` attempt failed
/// ("could not fetch updated group blob from any peer or seed") since
/// nobody was left to serve it — `member_removed` (CONVERGE-003) never
/// fired, and remaining members polled a resolved-but-unfetchable tombstone
/// forever. Trying this local reconstruction first (before ever attempting
/// a peer fetch) sidesteps the distribution problem entirely for the one
/// case where content is knowable without fetching anything.
pub fn try_decode_tombstone(signed: blake3::Hash, generation: u64) -> Option<GroupBlob> {
    let bytes = canonical_group_bytes(
        generation,
        &MemberList::new(),
        &ApprovedList::new(),
        None,
        &BTreeMap::new(),
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
        // A tombstone carries no real network's configured value -- it's
        // reconstructed purely from `generation`, so this must be the same
        // fixed constant `MeshManager::publish_nuke_tombstone` used to
        // publish it (both must agree byte-for-byte for the hash to match).
        default_nuke_consensus_threshold(),
    );
    if blake3::hash(&bytes) != signed {
        return None;
    }
    rmp_serde::from_slice(&bytes).ok()
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
    anyhow::ensure!(ip_u != base, "ip {} is the reserved network address", ip,);
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
    use super::*;
    use std::collections::BTreeMap;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = iroh::SecretKey::from(key_bytes);
        key.public()
    }

    #[test]
    fn test_member_list_add_and_lookup() {
        let id = test_id(1);
        let mut list = MemberList::new();
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        list.add(member.clone()).unwrap();
        assert!(list.is_member(&id));
        assert!(!list.is_member(&test_id(2)));
        assert_eq!(list.get(&id).unwrap().ip, Ipv4Addr::new(10, 88, 10, 5));
    }

    #[test]
    fn test_member_list_lookup_by_ip() {
        let id = test_id(1);
        let mut list = MemberList::new();
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        list.add(member).unwrap();
        let found = list.get_by_ip(Ipv4Addr::new(10, 88, 10, 5)).unwrap();
        assert_eq!(found.identity, id);
        assert!(list.get_by_ip(Ipv4Addr::new(10, 88, 10, 6)).is_none());
    }

    #[test]
    fn test_member_list_ip_collision() {
        let mut list = MemberList::new();
        list.add(Member {
            identity: test_id(1),
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        let result = list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
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
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        list.add(Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: true,
            hostname: None,
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
            ip: Ipv4Addr::new(10, 88, 10, 5),
            is_coordinator: false,
            hostname: None,
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
            ip: Ipv4Addr::new(10, 88, 0, 2),
            is_coordinator: true,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(10, 88, 0, 3),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        assert_eq!(list.all().len(), 2);
    }

    #[test]
    fn test_approved_list_add_and_check() {
        let id = test_id(1);
        let mut list = ApprovedList::new();
        let entry = ApprovedEntry {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 5, 10),
            hostname: None,
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
                ip: Ipv4Addr::new(10, 88, 5, 10),
                is_coordinator: false,
                hostname: None,
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        let entry = ApprovedEntry {
            identity: test_id(2),
            ip: Ipv4Addr::new(10, 88, 5, 10),
            hostname: None,
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
                    ip: Ipv4Addr::new(10, 88, 5, 10),
                    hostname: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        let result = approved.approve(
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(10, 88, 5, 10),
                hostname: None,
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
                    ip: Ipv4Addr::new(10, 88, 5, 10),
                    hostname: None,
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();
        approved
            .approve(
                ApprovedEntry {
                    identity: id,
                    ip: Ipv4Addr::new(10, 88, 5, 10),
                    hostname: None,
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
                    ip: Ipv4Addr::new(10, 88, 5, 10),
                    hostname: None,
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
                ip: Ipv4Addr::new(10, 88, 0, 2),
                hostname: None,
                collision_index: 0,
            },
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(10, 88, 0, 3),
                hostname: None,
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
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let b = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
                0,
                &m1,
                &approved,
                None,
                &BTreeMap::new(),
                None,
                &BTreeMap::new(),
                &BTreeMap::new(),
                2,
            ),
            canonical_group_bytes(
                0,
                &m2,
                &approved,
                None,
                &BTreeMap::new(),
                None,
                &BTreeMap::new(),
                &BTreeMap::new(),
                2,
            ),
        );
    }

    #[test]
    fn test_group_blob_hash_changes_on_mutation() {
        let members = make_member_list(&[1, 2]);
        let approved = ApprovedList::new();
        let h1 = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let members2 = make_member_list(&[1, 2, 3]);
        let h2 = group_blob_hash(
            0,
            &members2,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
                    collision_index: 0,
                },
                &members,
            )
            .unwrap();

        let bytes = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let hash = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
                collision_index: 0,
                last_seen: Some(12345),
            })
            .unwrap();
        let approved = ApprovedList::new();
        let bytes = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let hash = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
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
                collision_index: 0,
                last_seen: None,
            })
            .unwrap();
        let approved = ApprovedList::new();
        let bytes = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("last_seen"));
        let hash = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let data = verify_group_blob(&bytes, &hash).unwrap();
        assert_eq!(data.members[0].last_seen, None);
    }

    #[test]
    fn test_old_blob_without_optional_fields_decodes() {
        // A blob serialized before any of the later optional fields existed
        // (reusable_keys, invites, nuke_proposals, subnet, the now-removed
        // suggested_firewall) must still decode, defaulting them all.
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
            0,
            &members,
            &approved,
            None,
            &keys,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
            0,
            &members,
            &approved,
            None,
            &empty,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );

        let secret = [3u8; 16];
        let (hash, key) = reusable_key_for(&secret, 9_999_999_999, false);
        let mut keys = BTreeMap::new();
        keys.insert(hash.clone(), key);
        let h1 = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &keys,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert_ne!(h0, h1, "adding a reusable key must change the signed hash");

        // Revoking is a content change → the hash must change again so peers reconverge.
        keys.get_mut(&hash).unwrap().revoked = true;
        let h2 = group_blob_hash(
            0,
            &members,
            &approved,
            None,
            &keys,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert_ne!(
            h1, h2,
            "revoking a reusable key must change the signed hash"
        );
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
                generation: 0,
                members: vec![],
                approved: vec![],
                name: None,
                subnet: None,
                reusable_keys: keys,
                invites: BTreeMap::new(),
                nuke_proposals: BTreeMap::new(),
                nuke_consensus_threshold: default_nuke_consensus_threshold(),
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
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&member, default_subnet()).is_ok());
    }

    #[test]
    fn validate_member_rejects_mismatched_ip() {
        // A peer/ coordinator must not be able to assign an arbitrary IP to an
        // identity. This is the invariant that prevents IP hijacking. Must be
        // an address that's actually *inside* default_subnet() -- otherwise
        // this and validate_member_rejects_out_of_range_ip below could both
        // pass for the same "out of range" reason, never actually exercising
        // the in-range-but-mismatched (hijack) check this test names.
        let id = test_id(7);
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 99), // in-subnet, but does NOT equal derive_ip(test_id(7))
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        let err = validate_member(&member, default_subnet())
            .unwrap_err()
            .to_string();
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
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&member, default_subnet()).is_err());
    }

    #[test]
    fn validate_member_rejects_reserved_addresses() {
        // .0 (network) and .1 (gateway) are reserved even if derive_ip avoids
        // them. Must be default_subnet()'s own .0/.1 -- 100.64.0.0/100.64.0.1
        // are outside 10.88.0.0/24 entirely, so this test previously passed
        // only because of the (unrelated) out-of-range check, never actually
        // exercising the reserved-address rule it's named for.
        let id = test_id(7);
        let net = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 0),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        let gw = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 1),
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        assert!(validate_member(&net, default_subnet()).is_err());
        assert!(validate_member(&gw, default_subnet()).is_err());
    }

    #[test]
    fn validate_approved_rejects_mismatched_ip() {
        // Must be in-subnet (same reasoning as validate_member_rejects_
        // mismatched_ip above) so this exercises the mismatch check itself,
        // not an incidental out-of-range rejection.
        let id = test_id(9);
        let entry = ApprovedEntry {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 99),
            hostname: None,
            collision_index: 0,
        };
        assert!(validate_approved(&entry, default_subnet()).is_err());
    }

    #[test]
    fn validate_member_accepts_all_derived_ips_in_range() {
        // Every derive_ip() output for a spread of identities must pass validation,
        // as long as the IP does not collide with the reserved network (.0) or
        // gateway (.1) address. With the default /24 there are 254 usable host
        // addresses; we skip seeds that land on one of the 2 reserved.
        let mut validated = 0u32;
        let mut seed: u8 = 0;
        loop {
            let id = test_id(seed);
            let ip = derive_ip(&id, default_subnet());
            if ip != Ipv4Addr::new(10, 88, 0, 0)  // network
                && ip != Ipv4Addr::new(10, 88, 0, 1)
            // gateway
            {
                let member = Member {
                    identity: id,
                    ip,
                    is_coordinator: false,
                    hostname: None,
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
        // Blob has subnet: None -> resolves to default_subnet() (10.88.0.0/24);
        // the member ip below must actually be inside that range, or this
        // passes for the wrong reason (out of range, not mismatched).
        let id = test_id(1);
        let bad_member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 99), // in-subnet, but not derive_ip(test_id(1))
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        let blob = GroupBlob {
            generation: 0,
            members: vec![bad_member],
            approved: vec![],
            name: None,
            subnet: None,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
            nuke_proposals: BTreeMap::new(),
            nuke_consensus_threshold: default_nuke_consensus_threshold(),
        };
        let bytes = rmp_serde::to_vec_named(&blob).unwrap();
        let err = decode_group_blob(&bytes).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn decode_group_blob_rejects_reserved_gateway_ip() {
        // Must be default_subnet()'s own gateway (10.88.0.1), not the
        // pre-fork subnet's -- otherwise this is rejected as merely
        // out-of-range, never actually reaching the reserved-gateway check.
        let id = test_id(2);
        let bad_member = Member {
            identity: id,
            ip: Ipv4Addr::new(10, 88, 0, 1), // TUN gateway
            is_coordinator: false,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        };
        let blob = GroupBlob {
            generation: 0,
            members: vec![bad_member],
            approved: vec![],
            name: None,
            subnet: None,
            reusable_keys: BTreeMap::new(),
            invites: BTreeMap::new(),
            nuke_proposals: BTreeMap::new(),
            nuke_consensus_threshold: default_nuke_consensus_threshold(),
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
            collision_index: 0,
            last_seen: None,
        })
        .unwrap();
        mark_coordinator(&mut list, &id);
        assert!(list.get(&id).unwrap().is_coordinator);
    }

    #[test]
    fn validate_member_accepts_declared_index_rejects_mismatch() {
        let id = test_id(5);
        let good = Member {
            identity: id,
            ip: derive_ip_with_index(&id, 2, default_subnet()),
            is_coordinator: false,
            hostname: None,
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
            collision_index: 0,
            last_seen: None,
        };
        let dup = derive_ip(&a, default_subnet());
        assert!(validate_no_duplicate_ips(&[m(a, dup), m(test_id(2), dup)]).is_err());
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

    // --- configurable subnet (SUBNET-003/004/005/007) ------------------------

    // A custom subnet distinct from BOTH the default (10.88.0.0/24) and the
    // legacy CGNAT range, so "custom != default" assertions stay meaningful.
    const CUSTOM: Subnet = (Ipv4Addr::new(10, 99, 0, 0), 16);

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
    fn group_blob_subnet_survives_roundtrip() {
        let members = make_member_list_in(&[1, 2], CUSTOM);
        let bytes = canonical_group_bytes(
            0,
            &members,
            &ApprovedList::new(),
            None,
            &BTreeMap::new(),
            Some(CUSTOM),
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
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
                collision_index: idx,
                last_seen: None,
            })
            .unwrap();
        }
        list
    }

    // -- NUKE-CONSENSUS --------------------------------------------------------

    fn make_member(seed: u8, is_coordinator: bool) -> Member {
        let id = test_id(seed);
        Member {
            identity: id,
            ip: derive_ip(&id, default_subnet()),
            is_coordinator,
            hostname: None,
            collision_index: 0,
            last_seen: None,
        }
    }

    #[test]
    fn coordinator_count_counts_only_coordinators() {
        let members = vec![
            make_member(1, true),
            make_member(2, false),
            make_member(3, true),
        ];
        assert_eq!(coordinator_count(&members), 2);
        assert_eq!(coordinator_count(&[]), 0);
    }

    /// KICK-COORDINATOR-001: removing a coordinator's `Member` entry from
    /// the roster (what `kick_member` now does for a coordinator target,
    /// same as it always did for an ordinary one) must actually drop them
    /// out of `coordinator_count()` -- this is the load-bearing mechanism
    /// the whole fix relies on to also repair `leave_network`'s stranding
    /// check and `nuke_network`'s consensus gate, both of which read this
    /// same count. Mirrors `MemberList::remove`'s real call site
    /// (`remove_member_roster_only`, `daemon/mesh/coordinator.rs`) and
    /// `roster()`'s real snapshot pattern (`self.members.all().into_iter()
    /// .cloned().collect()`, `daemon/mod.rs:252`) rather than reaching into
    /// `coordinator_count` with a hand-built `Vec` that a real roster
    /// mutation could never actually produce.
    #[test]
    fn kicking_a_coordinator_drops_them_from_coordinator_count() {
        let mut list = MemberList::new();
        let zombie = make_member(1, true);
        let zombie_id = zombie.identity;
        list.add(zombie).unwrap();
        list.add(make_member(2, true)).unwrap();
        list.add(make_member(3, false)).unwrap();

        let roster_before: Vec<Member> = list.all().into_iter().cloned().collect();
        assert_eq!(coordinator_count(&roster_before), 2);

        // What kick_member's post-refusal-removal path actually does.
        list.remove(&zombie_id);

        let roster_after: Vec<Member> = list.all().into_iter().cloned().collect();
        assert_eq!(coordinator_count(&roster_after), 1);
    }

    #[test]
    fn active_nuke_proposers_excludes_expired() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("alice".to_string(), now - 10); // fresh
        proposals.insert("bob".to_string(), now - NUKE_PROPOSAL_TTL_SECS - 1); // expired
        let active = active_nuke_proposers(&proposals, now, NUKE_PROPOSAL_TTL_SECS);
        assert_eq!(active, vec!["alice"]);
    }

    #[test]
    fn active_nuke_proposers_boundary_is_expired() {
        // Exactly at the TTL boundary counts as expired (strict `<`).
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("alice".to_string(), now - NUKE_PROPOSAL_TTL_SECS);
        assert!(active_nuke_proposers(&proposals, now, NUKE_PROPOSAL_TTL_SECS).is_empty());
    }

    #[test]
    fn nuke_consensus_requires_two_distinct_active_proposers() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        assert!(!nuke_consensus_reached(&proposals, now, 2, NUKE_PROPOSAL_TTL_SECS));
        proposals.insert("alice".to_string(), now);
        assert!(!nuke_consensus_reached(&proposals, now, 2, NUKE_PROPOSAL_TTL_SECS));
        proposals.insert("bob".to_string(), now);
        assert!(nuke_consensus_reached(&proposals, now, 2, NUKE_PROPOSAL_TTL_SECS));
    }

    #[test]
    fn nuke_consensus_not_reached_with_one_fresh_one_expired() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("alice".to_string(), now);
        proposals.insert("bob".to_string(), now - NUKE_PROPOSAL_TTL_SECS - 1);
        assert!(!nuke_consensus_reached(&proposals, now, 2, NUKE_PROPOSAL_TTL_SECS));
    }

    /// NUKE-CONSENSUS-THRESHOLD-001: a configured threshold other than the
    /// historical hardcoded 2 must actually be honored, not silently ignored.
    #[test]
    fn nuke_consensus_honors_configured_threshold() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("alice".to_string(), now);
        // threshold=1: a single proposer is already enough.
        assert!(nuke_consensus_reached(&proposals, now, 1, NUKE_PROPOSAL_TTL_SECS));
        // threshold=3: two proposers are not enough on a stricter network.
        proposals.insert("bob".to_string(), now);
        assert!(!nuke_consensus_reached(&proposals, now, 3, NUKE_PROPOSAL_TTL_SECS));
        proposals.insert("carol".to_string(), now);
        assert!(nuke_consensus_reached(&proposals, now, 3, NUKE_PROPOSAL_TTL_SECS));
    }

    #[test]
    fn default_nuke_consensus_threshold_is_two() {
        assert_eq!(default_nuke_consensus_threshold(), 2);
    }

    #[test]
    fn resolve_nuke_proposer_matches_unambiguous_prefix() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("aaaa1111".to_string(), now);
        proposals.insert("bbbb2222".to_string(), now);
        assert_eq!(
            resolve_nuke_proposer(&proposals, now, NUKE_PROPOSAL_TTL_SECS, "aaaa").unwrap(),
            "aaaa1111"
        );
    }

    #[test]
    fn resolve_nuke_proposer_rejects_no_match() {
        let now = 1_000_000u64;
        let proposals = BTreeMap::new();
        assert!(resolve_nuke_proposer(&proposals, now, NUKE_PROPOSAL_TTL_SECS, "aaaa").is_err());
    }

    #[test]
    fn resolve_nuke_proposer_rejects_ambiguous_match() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("aaaa1111".to_string(), now);
        proposals.insert("aaaa2222".to_string(), now);
        assert!(resolve_nuke_proposer(&proposals, now, NUKE_PROPOSAL_TTL_SECS, "aaaa").is_err());
    }

    #[test]
    fn resolve_nuke_proposer_ignores_expired() {
        let now = 1_000_000u64;
        let mut proposals = BTreeMap::new();
        proposals.insert("aaaa1111".to_string(), now - NUKE_PROPOSAL_TTL_SECS - 1);
        assert!(resolve_nuke_proposer(&proposals, now, NUKE_PROPOSAL_TTL_SECS, "aaaa").is_err());
    }

    #[test]
    fn nuke_proposals_field_roundtrips_and_defaults_empty() {
        // Old blobs without the field must still decode (back-compat with
        // pre-NUKE-CONSENSUS blobs, not D1), and a blob with no proposals
        // must not serialize the (empty) field at all —
        // matching `reusable_keys`/`invites`'s `skip_serializing_if` convention.
        let members = make_member_list(&[1]);
        let approved = ApprovedList::new();
        let empty_bytes = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert!(!String::from_utf8_lossy(&empty_bytes).contains("nuke_proposals"));

        let mut proposals = BTreeMap::new();
        proposals.insert(test_id(9).to_string(), 42u64);
        let bytes = canonical_group_bytes(
            0,
            &members,
            &approved,
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &proposals,
            2,
        );
        let decoded: GroupBlob = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.nuke_proposals, proposals);
    }

    #[test]
    fn try_decode_tombstone_matches_deterministic_hash() {
        // A tombstone's hash depends only on generation -- this must decode
        // locally without any fetch, and round-trip back to an empty blob.
        let hash = group_blob_hash(
            7,
            &MemberList::new(),
            &ApprovedList::new(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        let decoded = try_decode_tombstone(hash, 7).expect("must decode as a tombstone");
        assert_eq!(decoded.generation, 7);
        assert!(decoded.members.is_empty());
        assert!(decoded.approved.is_empty());
    }

    #[test]
    fn try_decode_tombstone_rejects_wrong_generation() {
        // The hash is generation-specific -- claiming a different generation
        // for the same hash must not verify (would let a stale tombstone be
        // replayed at a higher generation).
        let hash = group_blob_hash(
            7,
            &MemberList::new(),
            &ApprovedList::new(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert!(try_decode_tombstone(hash, 8).is_none());
    }

    #[test]
    fn try_decode_tombstone_rejects_non_tombstone_hash() {
        // A real (non-empty) blob's hash must not be mistaken for a tombstone.
        let members = make_member_list(&[1, 2]);
        let hash = group_blob_hash(
            7,
            &members,
            &ApprovedList::new(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            2,
        );
        assert!(try_decode_tombstone(hash, 7).is_none());
    }
}
