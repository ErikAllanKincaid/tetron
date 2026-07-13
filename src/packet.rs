//! Minimal IP packet parsing for the data path.
//!
//! Extracts the addressing/port/ICMP fields the forwarder and the in-daemon
//! Magic-DNS responder need from a raw IPv4/IPv6 datagram. This is the packet
//! parser that survived the userspace-firewall removal (MINIMAL-010): the
//! forwarder still needs it for peer routing, the ingress anti-spoof check, and
//! the port-53 Magic-DNS intercept, none of which are firewall logic.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy)]
pub struct PacketInfo {
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub protocol: u8,
    pub src_port: u16,
    pub dst_port: u16,
    /// TCP flags byte (offset 13 of the TCP header). 0 for non-TCP. Bits:
    /// FIN 0x01, SYN 0x02, RST 0x04, ACK 0x10.
    pub tcp_flags: u8,
    /// ICMP/ICMPv6 type byte (offset 0 of the ICMP header). 0 for non-ICMP.
    pub icmp_type: u8,
    /// ICMP echo identifier (offset 4..6 of the ICMP header) for echo
    /// request/reply, else 0.
    pub icmp_id: u16,
}

/// ICMP (v4) and ICMPv6 protocol numbers.
fn is_icmp(proto: u8) -> bool {
    proto == 1 || proto == 58
}

/// True for an ICMP echo-*request* (ICMPv4 type 8 / ICMPv6 type 128).
fn is_icmp_echo_request(proto: u8, icmp_type: u8) -> bool {
    (proto == 1 && icmp_type == 8) || (proto == 58 && icmp_type == 128)
}

/// True for an ICMP echo-*reply* (ICMPv4 type 0 / ICMPv6 type 129).
fn is_icmp_echo_reply(proto: u8, icmp_type: u8) -> bool {
    (proto == 1 && icmp_type == 0) || (proto == 58 && icmp_type == 129)
}

pub fn parse_packet_info(packet: &[u8]) -> Option<PacketInfo> {
    if packet.is_empty() {
        return None;
    }
    match packet[0] >> 4 {
        4 => parse_ipv4(packet),
        6 => parse_ipv6(packet),
        _ => None,
    }
}

fn parse_ipv4(packet: &[u8]) -> Option<PacketInfo> {
    if packet.len() < 20 {
        return None;
    }
    let ihl = (packet[0] & 0x0F) as usize;
    let header_len = ihl * 4;
    if packet.len() < header_len {
        return None;
    }

    let protocol = packet[9];
    let src_ip = IpAddr::V4(Ipv4Addr::new(
        packet[12], packet[13], packet[14], packet[15],
    ));
    let dst_ip = IpAddr::V4(Ipv4Addr::new(
        packet[16], packet[17], packet[18], packet[19],
    ));

    let (src_port, dst_port) = extract_ports(protocol, packet, header_len);
    let tcp_flags = extract_tcp_flags(protocol, packet, header_len);
    let (icmp_type, icmp_id) = extract_icmp(protocol, packet, header_len);

    Some(PacketInfo {
        src_ip,
        dst_ip,
        protocol,
        src_port,
        dst_port,
        tcp_flags,
        icmp_type,
        icmp_id,
    })
}

fn parse_ipv6(packet: &[u8]) -> Option<PacketInfo> {
    if packet.len() < 40 {
        return None;
    }
    let protocol = packet[6]; // Next Header
    let mut src_octets = [0u8; 16];
    let mut dst_octets = [0u8; 16];
    src_octets.copy_from_slice(&packet[8..24]);
    dst_octets.copy_from_slice(&packet[24..40]);
    let src_ip = IpAddr::V6(Ipv6Addr::from(src_octets));
    let dst_ip = IpAddr::V6(Ipv6Addr::from(dst_octets));

    let header_len = 40; // fixed IPv6 header (extension headers not yet supported)
    let (src_port, dst_port) = extract_ports(protocol, packet, header_len);
    let tcp_flags = extract_tcp_flags(protocol, packet, header_len);
    let (icmp_type, icmp_id) = extract_icmp(protocol, packet, header_len);

    Some(PacketInfo {
        src_ip,
        dst_ip,
        protocol,
        src_port,
        dst_port,
        tcp_flags,
        icmp_type,
        icmp_id,
    })
}

fn extract_ports(protocol: u8, packet: &[u8], header_len: usize) -> (u16, u16) {
    if (protocol == 6 || protocol == 17) && packet.len() >= header_len + 4 {
        (
            u16::from_be_bytes([packet[header_len], packet[header_len + 1]]),
            u16::from_be_bytes([packet[header_len + 2], packet[header_len + 3]]),
        )
    } else {
        (0, 0)
    }
}

fn extract_tcp_flags(protocol: u8, packet: &[u8], header_len: usize) -> u8 {
    if protocol == 6 && packet.len() >= header_len + 14 {
        packet[header_len + 13]
    } else {
        0
    }
}

/// Extract the ICMP/ICMPv6 (type, echo-identifier) from a packet. The type byte
/// is the first byte of the ICMP header; the identifier (bytes 4..6) is only
/// meaningful for echo request/reply, so it is 0 for every other ICMP type.
/// Returns (0, 0) for non-ICMP packets.
fn extract_icmp(protocol: u8, packet: &[u8], header_len: usize) -> (u8, u16) {
    if !is_icmp(protocol) || packet.len() < header_len + 1 {
        return (0, 0);
    }
    let icmp_type = packet[header_len];
    let id = if (is_icmp_echo_request(protocol, icmp_type)
        || is_icmp_echo_reply(protocol, icmp_type))
        && packet.len() >= header_len + 6
    {
        u16::from_be_bytes([packet[header_len + 4], packet[header_len + 5]])
    } else {
        0
    };
    (icmp_type, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_ipv4() {
        let mut packet = vec![0u8; 24];
        packet[0] = 0x45;
        packet[9] = 6; // TCP
        packet[16] = 100;
        packet[17] = 64;
        packet[18] = 0;
        packet[19] = 3;
        let info = parse_packet_info(&packet).unwrap();
        assert_eq!(info.dst_ip, Ipv4Addr::new(100, 64, 0, 3));
        assert_eq!(info.protocol, 6);
    }

    #[test]
    fn parse_too_short() {
        assert!(parse_packet_info(&[0x45; 10]).is_none());
    }

    #[test]
    fn parse_ipv6_packet() {
        let mut packet = vec![0u8; 40];
        packet[0] = 0x60; // IPv6
        packet[6] = 6; // TCP next header
        packet[24] = 0x02;
        packet[25] = 0x01;
        let info = parse_packet_info(&packet).unwrap();
        assert!(info.dst_ip.is_ipv6());
    }
}
