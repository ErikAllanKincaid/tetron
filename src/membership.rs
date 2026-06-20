use std::collections::HashMap;
use std::fmt;
use std::net::Ipv4Addr;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    pub is_coordinator: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupMode {
    Open,
    #[default]
    Restricted,
}

impl fmt::Display for GroupMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GroupMode::Open => write!(f, "open"),
            GroupMode::Restricted => write!(f, "restricted"),
        }
    }
}

impl std::str::FromStr for GroupMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(GroupMode::Open),
            "restricted" => Ok(GroupMode::Restricted),
            other => Err(format!("unknown group mode: {other}")),
        }
    }
}

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

#[derive(Debug, Clone)]
pub struct MemberList {
    members: HashMap<EndpointId, Member>,
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

    #[cfg(test)]
    pub fn remove(&mut self, identity: &EndpointId) -> Option<Member> {
        self.members.remove(identity)
    }

    pub fn get(&self, identity: &EndpointId) -> Option<&Member> {
        self.members.get(identity)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
}

#[derive(Debug, Clone)]
pub struct ApprovedList {
    entries: HashMap<EndpointId, ApprovedEntry>,
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

pub trait MembershipPolicy: Send + Sync {
    fn can_authorize(&self, acceptor: &Member) -> bool;
}

pub struct OpenPolicy;

impl MembershipPolicy for OpenPolicy {
    fn can_authorize(&self, _acceptor: &Member) -> bool {
        true
    }
}

pub struct RestrictedPolicy;

impl MembershipPolicy for RestrictedPolicy {
    fn can_authorize(&self, acceptor: &Member) -> bool {
        acceptor.is_coordinator
    }
}

pub fn policy_for_mode(mode: GroupMode) -> Box<dyn MembershipPolicy> {
    match mode {
        GroupMode::Open => Box::new(OpenPolicy),
        GroupMode::Restricted => Box::new(RestrictedPolicy),
    }
}

pub trait IdentityProvider: Send + Sync {
    fn local_ip(&self) -> Ipv4Addr;
    fn local_identity(&self) -> EndpointId;
    fn derive_ip(&self, peer_identity: &EndpointId) -> Ipv4Addr;
}

pub fn derive_ip(identity: &EndpointId) -> Ipv4Addr {
    let id_str = identity.to_string();
    let mut hash: u32 = 2_166_136_261; // FNV-1a offset basis
    for &b in id_str.as_bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16_777_619); // FNV-1a prime
    }

    let base: u32 = 0x6440_0000; // 100.64.0.0
    let host_bits = hash & 0x003F_FFFF; // lower 22 bits
    // Reserve 0 (network) and 1 (TUN gateway)
    let host_bits = if host_bits <= 1 {
        host_bits + 2
    } else {
        host_bits
    };
    Ipv4Addr::from(base | host_bits)
}

#[derive(Clone)]
pub struct IrohIdentityProvider {
    endpoint_id: EndpointId,
    ip: Ipv4Addr,
}

impl IrohIdentityProvider {
    pub fn new(endpoint_id: EndpointId) -> Self {
        let ip = derive_ip(&endpoint_id);
        Self { endpoint_id, ip }
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
        derive_ip(peer_identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let key = iroh::SecretKey::from(key_bytes);
        key.public()
    }

    #[test]
    fn test_derive_ip_deterministic() {
        let id = test_id(1);
        let ip1 = derive_ip(&id);
        let ip2 = derive_ip(&id);
        assert_eq!(ip1, ip2);
    }

    #[test]
    fn test_derive_ip_in_cgnat_range() {
        let id = test_id(1);
        let ip = derive_ip(&id);
        let octets = ip.octets();
        assert_eq!(octets[0], 100);
        assert!(octets[1] >= 64 && octets[1] <= 127);
    }

    #[test]
    fn test_derive_ip_different_identities_differ() {
        let ip1 = derive_ip(&test_id(1));
        let ip2 = derive_ip(&test_id(2));
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn test_derive_ip_avoids_reserved() {
        let reserved1 = Ipv4Addr::new(100, 64, 0, 0);
        let reserved2 = Ipv4Addr::new(100, 64, 0, 1);
        for i in 0..=255u8 {
            let ip = derive_ip(&test_id(i));
            assert_ne!(ip, reserved1);
            assert_ne!(ip, reserved2);
        }
    }

    #[test]
    fn test_iroh_identity_provider() {
        let key = iroh::SecretKey::generate();
        let endpoint_id = key.public();
        let provider = IrohIdentityProvider::new(endpoint_id);

        let ip = provider.local_ip();
        let octets = ip.octets();
        assert_eq!(octets[0], 100);
        assert!(octets[1] >= 64 && octets[1] <= 127);

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
        };
        list.add(member.clone()).unwrap();
        assert!(list.is_member(&id));
        assert!(!list.is_member(&test_id(2)));
        assert_eq!(
            list.get(&id).unwrap().ip,
            Ipv4Addr::new(100, 64, 10, 5)
        );
    }

    #[test]
    fn test_member_list_lookup_by_ip() {
        let id = test_id(1);
        let mut list = MemberList::new();
        let member = Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
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
        })
        .unwrap();
        let result = list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: false,
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
        })
        .unwrap();
        list.add(Member {
            identity: id,
            ip: Ipv4Addr::new(100, 64, 10, 5),
            is_coordinator: true,
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
        })
        .unwrap();
        list.add(Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 0, 3),
            is_coordinator: false,
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
        };
        let regular = Member {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 0, 3),
            is_coordinator: false,
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
            })
            .unwrap();
        let entry = ApprovedEntry {
            identity: test_id(2),
            ip: Ipv4Addr::new(100, 64, 5, 10),
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
                },
                &members,
            )
            .unwrap();
        let result = approved.approve(
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(100, 64, 5, 10),
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
                },
                &members,
            )
            .unwrap();
        approved
            .approve(
                ApprovedEntry {
                    identity: id,
                    ip: Ipv4Addr::new(100, 64, 5, 10),
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
            },
            ApprovedEntry {
                identity: test_id(2),
                ip: Ipv4Addr::new(100, 64, 0, 3),
            },
        ];
        let list = ApprovedList::from_entries(entries);
        assert!(list.is_approved(&test_id(1)));
        assert!(list.is_approved(&test_id(2)));
        assert_eq!(list.all().len(), 2);
    }
}
