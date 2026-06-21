use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerFilter {
    Any,
    Identity(EndpointId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

impl PortRange {
    pub fn contains(&self, port: u16) -> bool {
        port >= self.start && port <= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallRule {
    pub direction: Direction,
    pub action: Action,
    pub protocol: Protocol,
    pub port: Option<PortRange>,
    pub peer: PeerFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallConfig {
    pub default_action: Action,
    pub rules: Vec<FirewallRule>,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            default_action: Action::Allow,
            rules: vec![],
        }
    }
}

#[derive(Clone)]
pub struct SharedFirewall {
    inner: Arc<RwLock<FirewallConfig>>,
}

impl SharedFirewall {
    pub fn new(config: FirewallConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(config)),
        }
    }

    pub fn evaluate(&self, direction: Direction, protocol: u8, dst_port: u16, peer: &EndpointId) -> Action {
        let config = self.inner.read().unwrap();
        for rule in &config.rules {
            if rule.direction != direction {
                continue;
            }
            if !protocol_matches(rule.protocol, protocol) {
                continue;
            }
            if let Some(ref range) = rule.port
                && !range.contains(dst_port) {
                    continue;
            }
            match &rule.peer {
                PeerFilter::Any => {}
                PeerFilter::Identity(id) => {
                    if id != peer {
                        continue;
                    }
                }
            }
            return rule.action;
        }
        config.default_action
    }

    pub fn update(&self, config: FirewallConfig) {
        *self.inner.write().unwrap() = config;
    }

    pub fn get_config(&self) -> FirewallConfig {
        self.inner.read().unwrap().clone()
    }
}

fn protocol_matches(filter: Protocol, ip_proto: u8) -> bool {
    match filter {
        Protocol::Any => true,
        Protocol::Tcp => ip_proto == 6,
        Protocol::Udp => ip_proto == 17,
        Protocol::Icmp => ip_proto == 1,
    }
}

pub struct PacketInfo {
    pub src_ip: Ipv4Addr,
    pub dst_ip: Ipv4Addr,
    pub protocol: u8,
    pub src_port: u16,
    pub dst_port: u16,
}

pub fn parse_packet_info(packet: &[u8]) -> Option<PacketInfo> {
    if packet.len() < 20 {
        return None;
    }
    if packet[0] >> 4 != 4 {
        return None;
    }
    let ihl = (packet[0] & 0x0F) as usize;
    let header_len = ihl * 4;
    if packet.len() < header_len {
        return None;
    }

    let protocol = packet[9];
    let src_ip = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let dst_ip = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);

    let (src_port, dst_port) = if (protocol == 6 || protocol == 17) && packet.len() >= header_len + 4 {
        (
            u16::from_be_bytes([packet[header_len], packet[header_len + 1]]),
            u16::from_be_bytes([packet[header_len + 2], packet[header_len + 3]]),
        )
    } else {
        (0, 0)
    };

    Some(PacketInfo { src_ip, dst_ip, protocol, src_port, dst_port })
}

pub fn firewall_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine config directory")?
        .join("pitopi");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("firewall.toml"))
}

pub fn load_firewall() -> Result<FirewallConfig> {
    let path = firewall_path()?;
    if !path.exists() {
        return Ok(FirewallConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parse {}", path.display()))
}

pub fn save_firewall(config: &FirewallConfig) -> Result<()> {
    let path = firewall_path()?;
    let content = toml::to_string_pretty(config).context("serialize firewall config")?;
    std::fs::write(&path, content).with_context(|| format!("write {}", path.display()))
}

pub fn parse_direction(s: &str) -> Result<Direction> {
    match s {
        "in" => Ok(Direction::In),
        "out" => Ok(Direction::Out),
        _ => bail!("invalid direction '{}' (expected 'in' or 'out')", s),
    }
}

pub fn parse_action(s: &str) -> Result<Action> {
    match s {
        "allow" => Ok(Action::Allow),
        "deny" => Ok(Action::Deny),
        _ => bail!("invalid action '{}' (expected 'allow' or 'deny')", s),
    }
}

pub fn parse_protocol(s: &str) -> Result<Protocol> {
    match s {
        "tcp" => Ok(Protocol::Tcp),
        "udp" => Ok(Protocol::Udp),
        "icmp" => Ok(Protocol::Icmp),
        "any" => Ok(Protocol::Any),
        _ => bail!("invalid protocol '{}' (expected 'tcp', 'udp', 'icmp', or 'any')", s),
    }
}

pub fn parse_port_range(s: &str) -> Result<PortRange> {
    if let Some((start, end)) = s.split_once('-') {
        let start: u16 = start.parse().context("invalid start port")?;
        let end: u16 = end.parse().context("invalid end port")?;
        if start > end {
            bail!("start port ({start}) must be <= end port ({end})");
        }
        Ok(PortRange { start, end })
    } else {
        let port: u16 = s.parse().context("invalid port number")?;
        Ok(PortRange { start: port, end: port })
    }
}

fn format_protocol(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
        Protocol::Icmp => "icmp",
        Protocol::Any => "any",
    }
}

fn format_direction(d: Direction) -> &'static str {
    match d {
        Direction::In => "in",
        Direction::Out => "out",
    }
}

fn format_action(a: Action) -> &'static str {
    match a {
        Action::Allow => "allow",
        Action::Deny => "deny",
    }
}

pub fn format_firewall_show(config: &FirewallConfig, short_id: &dyn Fn(&EndpointId) -> String) -> String {
    let mut out = format!("Default: {}\n", format_action(config.default_action));

    if config.rules.is_empty() {
        out.push_str("No rules.\n");
        return out;
    }

    out.push_str("Rules:\n");
    for (i, rule) in config.rules.iter().enumerate() {
        let peer_str = match &rule.peer {
            PeerFilter::Any => "any".to_string(),
            PeerFilter::Identity(id) => short_id(id),
        };
        let port_str = match &rule.port {
            None => "*".to_string(),
            Some(r) if r.start == r.end => r.start.to_string(),
            Some(r) => format!("{}-{}", r.start, r.end),
        };
        out.push_str(&format!(
            "  [{}] {} {} proto={} port={} peer={}\n",
            i,
            format_direction(rule.direction),
            format_action(rule.action),
            format_protocol(rule.protocol),
            port_str,
            peer_str,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        iroh::SecretKey::from(key_bytes).public()
    }

    #[test]
    fn parse_valid_ipv4_tcp() {
        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x45; // IPv4, IHL=5
        pkt[9] = 6; // TCP
        pkt[12..16].copy_from_slice(&[10, 0, 0, 1]);
        pkt[16..20].copy_from_slice(&[10, 0, 0, 2]);
        pkt[20] = 0x1F; // src port 8080
        pkt[21] = 0x90;
        pkt[22] = 0x01; // dst port 443
        pkt[23] = 0xBB;

        let info = parse_packet_info(&pkt).unwrap();
        assert_eq!(info.src_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(info.dst_ip, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(info.protocol, 6);
        assert_eq!(info.src_port, 8080);
        assert_eq!(info.dst_port, 443);
    }

    #[test]
    fn parse_udp_packet() {
        let mut pkt = vec![0u8; 28];
        pkt[0] = 0x45;
        pkt[9] = 17; // UDP
        pkt[20] = 0x00;
        pkt[21] = 53; // src port 53
        pkt[22] = 0x04;
        pkt[23] = 0xD2; // dst port 1234

        let info = parse_packet_info(&pkt).unwrap();
        assert_eq!(info.protocol, 17);
        assert_eq!(info.src_port, 53);
        assert_eq!(info.dst_port, 1234);
    }

    #[test]
    fn parse_icmp_no_ports() {
        let mut pkt = vec![0u8; 28];
        pkt[0] = 0x45;
        pkt[9] = 1; // ICMP

        let info = parse_packet_info(&pkt).unwrap();
        assert_eq!(info.protocol, 1);
        assert_eq!(info.src_port, 0);
        assert_eq!(info.dst_port, 0);
    }

    #[test]
    fn parse_too_short() {
        assert!(parse_packet_info(&[0x45; 10]).is_none());
    }

    #[test]
    fn parse_not_ipv4() {
        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x60; // IPv6
        assert!(parse_packet_info(&pkt).is_none());
    }

    #[test]
    fn evaluate_default_allow() {
        let fw = SharedFirewall::new(FirewallConfig::default());
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Allow);
    }

    #[test]
    fn evaluate_default_deny() {
        let fw = SharedFirewall::new(FirewallConfig {
            default_action: Action::Deny,
            rules: vec![],
        });
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Deny);
    }

    #[test]
    fn evaluate_deny_specific_port() {
        let fw = SharedFirewall::new(FirewallConfig {
            default_action: Action::Allow,
            rules: vec![FirewallRule {
                direction: Direction::In,
                action: Action::Deny,
                protocol: Protocol::Tcp,
                port: Some(PortRange { start: 22, end: 22 }),
                peer: PeerFilter::Any,
            }],
        });
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Deny);
        assert_eq!(fw.evaluate(Direction::In, 6, 80, &test_id(1)), Action::Allow);
        assert_eq!(fw.evaluate(Direction::Out, 6, 22, &test_id(1)), Action::Allow);
    }

    #[test]
    fn evaluate_port_range() {
        let fw = SharedFirewall::new(FirewallConfig {
            default_action: Action::Deny,
            rules: vec![FirewallRule {
                direction: Direction::In,
                action: Action::Allow,
                protocol: Protocol::Any,
                port: Some(PortRange { start: 80, end: 443 }),
                peer: PeerFilter::Any,
            }],
        });
        assert_eq!(fw.evaluate(Direction::In, 6, 80, &test_id(1)), Action::Allow);
        assert_eq!(fw.evaluate(Direction::In, 17, 443, &test_id(1)), Action::Allow);
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Deny);
    }

    #[test]
    fn evaluate_peer_filter() {
        let fw = SharedFirewall::new(FirewallConfig {
            default_action: Action::Deny,
            rules: vec![FirewallRule {
                direction: Direction::In,
                action: Action::Allow,
                protocol: Protocol::Any,
                port: None,
                peer: PeerFilter::Identity(test_id(1)),
            }],
        });
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Allow);
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(2)), Action::Deny);
    }

    #[test]
    fn evaluate_first_match_wins() {
        let fw = SharedFirewall::new(FirewallConfig {
            default_action: Action::Deny,
            rules: vec![
                FirewallRule {
                    direction: Direction::In,
                    action: Action::Deny,
                    protocol: Protocol::Tcp,
                    port: Some(PortRange { start: 22, end: 22 }),
                    peer: PeerFilter::Any,
                },
                FirewallRule {
                    direction: Direction::In,
                    action: Action::Allow,
                    protocol: Protocol::Any,
                    port: None,
                    peer: PeerFilter::Any,
                },
            ],
        });
        // SSH denied by first rule even though second allows all
        assert_eq!(fw.evaluate(Direction::In, 6, 22, &test_id(1)), Action::Deny);
        // Other ports allowed by second rule
        assert_eq!(fw.evaluate(Direction::In, 6, 80, &test_id(1)), Action::Allow);
    }

    #[test]
    fn port_range_parsing() {
        let r = parse_port_range("80").unwrap();
        assert_eq!(r, PortRange { start: 80, end: 80 });

        let r = parse_port_range("80-443").unwrap();
        assert_eq!(r, PortRange { start: 80, end: 443 });

        assert!(parse_port_range("443-80").is_err());
        assert!(parse_port_range("abc").is_err());
    }

    #[test]
    fn config_serialization_roundtrip() {
        let config = FirewallConfig {
            default_action: Action::Deny,
            rules: vec![FirewallRule {
                direction: Direction::In,
                action: Action::Allow,
                protocol: Protocol::Tcp,
                port: Some(PortRange { start: 443, end: 443 }),
                peer: PeerFilter::Any,
            }],
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let decoded: FirewallConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(decoded.default_action, Action::Deny);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].port.as_ref().unwrap().start, 443);
    }
}
