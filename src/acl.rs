use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultPolicy {
    #[serde(rename = "deny-all")]
    DenyAll,
    #[serde(rename = "allow-same-network")]
    AllowSameNetwork,
    #[serde(rename = "allow-all")]
    AllowAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub port: Option<u16>,
    pub allow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclPolicy {
    pub default: DefaultPolicy,
    pub rules: Vec<AclRule>,
}

impl AclPolicy {
    pub fn allow_all() -> Self {
        Self {
            default: DefaultPolicy::AllowAll,
            rules: vec![],
        }
    }

    pub fn deny_all() -> Self {
        Self {
            default: DefaultPolicy::DenyAll,
            rules: vec![],
        }
    }

    pub fn check(&self, src: Ipv4Addr, dst: Ipv4Addr, dst_port: Option<u16>) -> bool {
        for rule in &self.rules {
            if rule.src == src
                && rule.dst == dst
                && (rule.port.is_none() || rule.port == dst_port)
            {
                return rule.allow;
            }
        }
        match self.default {
            DefaultPolicy::AllowAll | DefaultPolicy::AllowSameNetwork => true,
            DefaultPolicy::DenyAll => false,
        }
    }
}

fn dst_port_from_packet(packet: &[u8]) -> Option<u16> {
    if packet.len() < 20 {
        return None;
    }
    let ihl = (packet[0] & 0x0f) as usize * 4;
    let protocol = packet[9];
    // TCP (6) or UDP (17)
    if (protocol == 6 || protocol == 17) && packet.len() >= ihl + 4 {
        let port = u16::from_be_bytes([packet[ihl + 2], packet[ihl + 3]]);
        Some(port)
    } else {
        None
    }
}

pub fn packet_allowed(policy: &AclPolicy, packet: &[u8]) -> bool {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return false;
    }
    let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
    let port = dst_port_from_packet(packet);
    policy.check(src, dst, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all_policy() {
        let policy = AclPolicy::allow_all();
        let src = Ipv4Addr::new(100, 64, 0, 2);
        let dst = Ipv4Addr::new(100, 64, 0, 3);
        assert!(policy.check(src, dst, None));
    }

    #[test]
    fn test_deny_all_policy() {
        let policy = AclPolicy::deny_all();
        let src = Ipv4Addr::new(100, 64, 0, 2);
        let dst = Ipv4Addr::new(100, 64, 0, 3);
        assert!(!policy.check(src, dst, None));
    }

    #[test]
    fn test_explicit_allow_overrides_deny() {
        let mut policy = AclPolicy::deny_all();
        policy.rules.push(AclRule {
            src: Ipv4Addr::new(100, 64, 0, 2),
            dst: Ipv4Addr::new(100, 64, 0, 1),
            port: Some(25565),
            allow: true,
        });
        assert!(policy.check(
            Ipv4Addr::new(100, 64, 0, 2),
            Ipv4Addr::new(100, 64, 0, 1),
            Some(25565),
        ));
        assert!(!policy.check(
            Ipv4Addr::new(100, 64, 0, 2),
            Ipv4Addr::new(100, 64, 0, 1),
            Some(22),
        ));
    }

    #[test]
    fn test_explicit_deny_overrides_allow() {
        let mut policy = AclPolicy::allow_all();
        policy.rules.push(AclRule {
            src: Ipv4Addr::new(100, 64, 0, 3),
            dst: Ipv4Addr::new(100, 64, 0, 1),
            port: None,
            allow: false,
        });
        assert!(!policy.check(
            Ipv4Addr::new(100, 64, 0, 3),
            Ipv4Addr::new(100, 64, 0, 1),
            None,
        ));
        assert!(policy.check(
            Ipv4Addr::new(100, 64, 0, 2),
            Ipv4Addr::new(100, 64, 0, 1),
            None,
        ));
    }

    #[test]
    fn test_packet_allowed() {
        let policy = AclPolicy::allow_all();
        let mut packet = vec![0u8; 40];
        packet[0] = 0x45; // IPv4, IHL=5
        packet[9] = 6; // TCP
        packet[12..16].copy_from_slice(&[100, 64, 0, 2]);
        packet[16..20].copy_from_slice(&[100, 64, 0, 1]);
        packet[22] = 0x63; // dst port 25565
        packet[23] = 0xDD;
        assert!(packet_allowed(&policy, &packet));
    }

    #[test]
    fn test_packet_denied_too_short() {
        let policy = AclPolicy::allow_all();
        assert!(!packet_allowed(&policy, &[0x45; 10]));
    }

    #[test]
    fn test_policy_toml_roundtrip() {
        let policy = AclPolicy {
            default: DefaultPolicy::DenyAll,
            rules: vec![AclRule {
                src: Ipv4Addr::new(100, 64, 0, 2),
                dst: Ipv4Addr::new(100, 64, 0, 1),
                port: Some(25565),
                allow: true,
            }],
        };
        let toml_str = toml::to_string_pretty(&policy).unwrap();
        let parsed: AclPolicy = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.default, policy.default);
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.rules[0].port, Some(25565));
    }

    #[test]
    fn test_dst_port_tcp() {
        let mut packet = vec![0u8; 24];
        packet[0] = 0x45;
        packet[9] = 6; // TCP
        packet[22] = 0x00;
        packet[23] = 0x50; // port 80
        assert_eq!(dst_port_from_packet(&packet), Some(80));
    }

    #[test]
    fn test_dst_port_udp() {
        let mut packet = vec![0u8; 24];
        packet[0] = 0x45;
        packet[9] = 17; // UDP
        packet[22] = 0x01;
        packet[23] = 0xBB; // port 443
        assert_eq!(dst_port_from_packet(&packet), Some(443));
    }
}
