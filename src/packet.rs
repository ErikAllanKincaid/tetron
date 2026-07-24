//! Minimal IP packet parsing for the data path.
//!
//! Extracts the addressing/port/ICMP fields the forwarder and the in-daemon
//! Magic-DNS responder need from a raw IPv4/IPv6 datagram. This is the packet
//! parser that survived the userspace-firewall removal (MINIMAL-010): the
//! forwarder still needs it for peer routing, the ingress anti-spoof check, and
//! the port-53 Magic-DNS intercept, none of which are firewall logic.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

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

/// Fragment an IPv4 packet into smaller IP fragments, each <= `max_size` bytes
/// total (including the IP header). This is necessary when a TUN packet exceeds
/// the QUIC connection's `max_datagram_size()`, so the forwarder can split the
/// payload across multiple QUIC datagrams. The receiving OS kernel reassembles
/// the fragments.
///
/// Returns `None` when the packet needs no fragmentation (already <= `max_size`),
/// has IP options (IHL > 5, which this implementation doesn't support), or
/// `max_size` is too small for even a single fragment header + 8 payload bytes.
pub fn fragment_ipv4(packet: &[u8], max_size: usize) -> Option<Vec<Vec<u8>>> {
    const HEADER_LEN: usize = 20; // no IP options
    if packet.len() <= max_size {
        return None;
    }
    if packet.len() < HEADER_LEN {
        return None;
    }
    let ihl = (packet[0] & 0x0F) as usize;
    if ihl * 4 != HEADER_LEN {
        return None; // IP options not supported
    }

    // F-04: verify the original header checksum before trusting any of its
    // fields (identification, DF flag) or fragmenting at all. Without this,
    // a corrupted or malformed header would be silently "healed" -- every
    // fragment gets a freshly computed, valid checksum regardless of
    // whether the source header was ever actually intact, forwarding data
    // that should have been dropped instead.
    let mut hdr = [0u8; HEADER_LEN];
    hdr.copy_from_slice(&packet[..HEADER_LEN]);
    let stored_csum = u16::from_be_bytes([hdr[10], hdr[11]]);
    hdr[10] = 0;
    hdr[11] = 0;
    if ip_checksum(&hdr) != stored_csum {
        return None; // corrupt header — refuse to fragment
    }

    let payload_len = packet.len() - HEADER_LEN;
    // Fragment payload must be a multiple of 8 bytes (RFC 791), except the last.
    let max_payload = (max_size - HEADER_LEN) & !7;
    if max_payload < 8 {
        return None;
    }

    // Preserve original identification so all fragments share it for reassembly.
    let id_hi = packet[4];
    let id_lo = packet[5];
    // Preserve the Don't Fragment flag so the fragment set carries the original
    // intent (though we must fragment anyway — the tunnel encapsulates).
    let df = packet[6] & 0x40;

    let mut fragments = Vec::new();
    let mut offset = 0usize;

    while offset < payload_len {
        let frag_payload_len = max_payload.min(payload_len - offset);
        let is_last = offset + frag_payload_len >= payload_len;
        let mf: u8 = if is_last { 0 } else { 1 };
        let frag_offset = (offset / 8) as u16;

        let mut frag = Vec::with_capacity(HEADER_LEN + frag_payload_len);

        // Copy the IP header (unchanged for most fields)
        frag.extend_from_slice(&packet[..HEADER_LEN]);

        // Total Length (bytes 2-3)
        let total_len = (HEADER_LEN + frag_payload_len) as u16;
        frag[2] = (total_len >> 8) as u8;
        frag[3] = total_len as u8;

        // Identification (bytes 4-5) — copy from original
        frag[4] = id_hi;
        frag[5] = id_lo;

        // Flags + Fragment Offset (bytes 6-7). Byte 6 in wire format:
        //   bit 7 = Reserved (0)
        //   bit 6 = DF (preserved from original)
        //   bit 5 = MF (More Fragments)
        //   bits 4-0 = Fragment Offset bits 12-8
        // Byte 7 = Fragment Offset bits 7-0
        frag[6] = df | (mf << 5) | ((frag_offset >> 8) as u8 & 0x1F);
        frag[7] = (frag_offset & 0xFF) as u8;

        // Clear header checksum and recompute
        frag[10] = 0;
        frag[11] = 0;
        let csum = ip_checksum(&frag[..HEADER_LEN]);
        frag[10] = (csum >> 8) as u8;
        frag[11] = csum as u8;

        // Copy the payload portion for this fragment
        let start = HEADER_LEN + offset;
        frag.extend_from_slice(&packet[start..start + frag_payload_len]);

        fragments.push(frag);
        offset += frag_payload_len;
    }

    Some(fragments)
}

/// Marker byte for a tetron-internal IPv6 fragment envelope (see
/// [`fragment_ipv6`]). Its top nibble (0xF) can never collide with a real IP
/// packet's first byte, whose top nibble is always the IP version (4 or 6).
pub const FRAG6_MAGIC: u8 = 0xF6;

/// Wire size of the tetron-internal fragment envelope: magic(1) + id(4) +
/// offset(2) + more-flag(1).
const FRAG6_HEADER_LEN: usize = 8;

/// A reassembled IPv6 packet claiming to be bigger than this is refused.
/// Mirrors `tun::TUN_MTU` (1280) -- no packet [`fragment_ipv6`] ever produced
/// can legitimately be bigger than the TUN device's own MTU, so a bigger
/// claim is either a bug or a malicious peer inflating offsets, never
/// legitimate mesh traffic.
const MAX_REASSEMBLED_IPV6_PACKET: usize = 1280;

/// Fragment an oversized IPv6 packet into tetron-internal envelopes that each
/// fit in `max_size` bytes on the wire (envelope + payload slice).
///
/// Unlike IPv4, an IPv6 packet has no fragmentation fields in its base
/// header -- RFC 8200 SS4.5 requires a separate Fragment extension header,
/// legal only at the packet's true originating host, reassembled by a
/// generic IP stack. tetron is both the true origin (reads the whole packet
/// off its own TUN) and the reassembling party (its own code,
/// [`Ipv6Reassembler`], not a generic IP stack) for both ends of this hop, so
/// a small tetron-internal envelope is the natural fit here rather than
/// reimplementing RFC 8200 SS4.5 verbatim (see `FRAG-002` in
/// `spec/design_spec.py`). `id` should be unique per fragmented packet on
/// this connection (the caller supplies it, typically from a per-connection
/// counter) so concurrent or overlapping fragment sets can't be confused
/// with each other by the receiver.
///
/// Returns `None` when the packet already fits in `max_size` (no
/// fragmentation needed) or `max_size` leaves no room for the envelope
/// header plus at least one payload byte.
pub fn fragment_ipv6(packet: &[u8], id: u32, max_size: usize) -> Option<Vec<Vec<u8>>> {
    if packet.len() <= max_size {
        return None;
    }
    if max_size <= FRAG6_HEADER_LEN {
        return None;
    }
    let max_payload = max_size - FRAG6_HEADER_LEN;
    let mut fragments = Vec::new();
    let mut offset = 0usize;

    while offset < packet.len() {
        let frag_len = max_payload.min(packet.len() - offset);
        let more: u8 = if offset + frag_len < packet.len() { 1 } else { 0 };

        let mut frag = Vec::with_capacity(FRAG6_HEADER_LEN + frag_len);
        frag.push(FRAG6_MAGIC);
        frag.extend_from_slice(&id.to_be_bytes());
        frag.extend_from_slice(&(offset as u16).to_be_bytes());
        frag.push(more);
        frag.extend_from_slice(&packet[offset..offset + frag_len]);

        fragments.push(frag);
        offset += frag_len;
    }

    Some(fragments)
}

/// Outcome of feeding one raw peer datagram to [`Ipv6Reassembler::accept`].
#[derive(Debug, PartialEq, Eq)]
pub enum Ipv6FragmentOutcome {
    /// Every fragment for this id has now arrived; the reassembled packet is
    /// ready to forward.
    Complete(Vec<u8>),
    /// A valid fragment was accepted; still waiting on the rest of its set.
    Waiting,
    /// The datagram doesn't carry the fragment magic byte at all -- the
    /// ordinary, unfragmented path. Callers should process it as a normal
    /// packet, not wait on it.
    NotAFragment,
    /// A fragment envelope was truncated/malformed, or its claimed offset
    /// would grow the reassembled packet past what tetron could have
    /// legitimately produced -- discarded outright.
    Rejected,
}

struct PartialIpv6 {
    chunks: BTreeMap<u16, Vec<u8>>,
    total_len: Option<usize>,
    last_seen: Instant,
}

impl PartialIpv6 {
    /// `Some(packet)` once `chunks` covers `[0, total_len)` with no gaps or
    /// overlaps (checked by walking offsets in order and requiring each to
    /// start exactly where the previous one ended); `None` while any part is
    /// still missing, out of order, or the final (more=0) fragment hasn't
    /// arrived yet to reveal `total_len`.
    fn try_complete(&self) -> Option<Vec<u8>> {
        let total = self.total_len?;
        let mut out = Vec::with_capacity(total);
        let mut expected = 0usize;
        for (&offset, bytes) in &self.chunks {
            if offset as usize != expected {
                return None;
            }
            out.extend_from_slice(bytes);
            expected += bytes.len();
        }
        (expected == total).then_some(out)
    }
}

/// Bounds how many distinct fragment sets ("ids") a single connection may
/// have in flight at once. A peer that opens many fragment sets and never
/// completes any of them (accidentally or maliciously) evicts its own
/// oldest entry here, rather than growing this map without limit.
const MAX_REASSEMBLY_ENTRIES: usize = 16;

/// A fragment set that hasn't received a new fragment within this long is
/// abandoned and its partial state freed -- covers a dropped final fragment,
/// which would otherwise leak memory for the life of the connection.
const REASSEMBLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-peer-connection reassembly state for [`fragment_ipv6`] envelopes. One
/// instance lives for the lifetime of a single peer's reader task
/// (`forward::spawn_peer_reader`) -- reassembly is connection-scoped, so no
/// locking is needed.
pub struct Ipv6Reassembler {
    partial: HashMap<u32, PartialIpv6>,
}

impl Ipv6Reassembler {
    pub fn new() -> Self {
        Self {
            partial: HashMap::new(),
        }
    }

    /// Feed one raw datagram read from the peer connection. See
    /// [`Ipv6FragmentOutcome`] for what each outcome means.
    pub fn accept(&mut self, datagram: &[u8]) -> Ipv6FragmentOutcome {
        if datagram.first() != Some(&FRAG6_MAGIC) {
            return Ipv6FragmentOutcome::NotAFragment;
        }
        if datagram.len() < FRAG6_HEADER_LEN {
            return Ipv6FragmentOutcome::Rejected;
        }
        self.gc_stale();

        let id = u32::from_be_bytes([datagram[1], datagram[2], datagram[3], datagram[4]]);
        let offset = u16::from_be_bytes([datagram[5], datagram[6]]);
        let more = datagram[7] != 0;
        let payload = &datagram[FRAG6_HEADER_LEN..];

        if offset as usize + payload.len() > MAX_REASSEMBLED_IPV6_PACKET {
            self.partial.remove(&id);
            return Ipv6FragmentOutcome::Rejected;
        }

        if !self.partial.contains_key(&id)
            && self.partial.len() >= MAX_REASSEMBLY_ENTRIES
            && let Some(oldest) = self
                .partial
                .iter()
                .min_by_key(|(_, p)| p.last_seen)
                .map(|(k, _)| *k)
        {
            self.partial.remove(&oldest);
        }

        let entry = self.partial.entry(id).or_insert_with(|| PartialIpv6 {
            chunks: BTreeMap::new(),
            total_len: None,
            last_seen: Instant::now(),
        });
        entry.last_seen = Instant::now();
        if !more {
            entry.total_len = Some(offset as usize + payload.len());
        }
        entry.chunks.insert(offset, payload.to_vec());

        match entry.try_complete() {
            Some(full) => {
                self.partial.remove(&id);
                Ipv6FragmentOutcome::Complete(full)
            }
            None => Ipv6FragmentOutcome::Waiting,
        }
    }

    fn gc_stale(&mut self) {
        let now = Instant::now();
        self.partial
            .retain(|_, p| now.duration_since(p.last_seen) < REASSEMBLY_TIMEOUT);
    }
}

impl Default for Ipv6Reassembler {
    fn default() -> Self {
        Self::new()
    }
}

/// Internet checksum (RFC 1071) over an even-length byte slice. The caller must
/// zero the checksum field before passing the data in.
fn ip_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    // Odd-length: pad with zero byte
    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }
    // Fold 32-bit sum to 16-bit 1's complement
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_ipv4() {
        let mut packet = vec![0u8; 24];
        packet[0] = 0x45;
        packet[9] = 6; // TCP
        packet[16] = 10;
        packet[17] = 88;
        packet[18] = 0;
        packet[19] = 3;
        let info = parse_packet_info(&packet).unwrap();
        assert_eq!(info.dst_ip, Ipv4Addr::new(10, 88, 0, 3));
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

    // ── IPv4 fragmentation tests ──────────────────────────────

    fn make_ipv4(payload_len: usize, id: u16, df: bool) -> Vec<u8> {
        let total = 20 + payload_len;
        let mut p = vec![0u8; total];
        p[0] = 0x45; // IPv4, IHL=5
        p[2] = (total >> 8) as u8;
        p[3] = total as u8;
        p[4] = (id >> 8) as u8;
        p[5] = id as u8;
        if df {
            p[6] |= 0x40;
        }
        p[8] = 64; // TTL
        p[9] = 6;  // TCP
        // Fill payload with a pattern so reassembly ordering can be verified
        for i in 0..payload_len {
            p[20 + i] = (i & 0xFF) as u8;
        }
        // Compute and store the header checksum
        p[10] = 0;
        p[11] = 0;
        let csum = ip_checksum(&p[..20]);
        p[10] = (csum >> 8) as u8;
        p[11] = csum as u8;
        p
    }

    #[test]
    fn frag_no_fragmentation_needed() {
        // 60-byte packet fits in a 1200-byte max
        let pkt = make_ipv4(40, 0x1234, false);
        assert!(fragment_ipv4(&pkt, 1200).is_none());
    }

    #[test]
    fn frag_malformed_too_short() {
        assert!(fragment_ipv4(&[0x45, 0x00, 0x00, 0x14], 100).is_none());
    }

    #[test]
    fn frag_rejects_corrupt_header_checksum() {
        // F-04: a header whose stored checksum doesn't match its own content
        // must be refused, not silently "healed" by computing a fresh valid
        // checksum for each fragment.
        let mut pkt = make_ipv4(1208, 0x1234, false);
        pkt[10] ^= 0xFF; // flip the stored checksum's high byte
        assert!(fragment_ipv4(&pkt, 1200).is_none());
    }

    #[test]
    fn frag_accepts_a_genuinely_valid_header() {
        // F-04 live-testing regression (2026-07-24): the only prior coverage
        // of the checksum check exercised the "corrupt gets rejected"
        // direction; nothing asserted a genuinely valid, untouched header is
        // still accepted. A stray extra `!` made the check reject every
        // header unconditionally -- confirmed live on a real 3-machine mesh,
        // where every oversized IPv4 packet needing fragmentation was
        // dropped with "cannot fragment ... (options or malformed)".
        let pkt = make_ipv4(1208, 0x1234, false);
        assert!(
            fragment_ipv4(&pkt, 1200).is_some(),
            "a valid, untouched header must fragment successfully, not be treated as corrupt"
        );
    }

    #[test]
    fn ip_checksum_matches_a_known_good_real_world_header() {
        // Ground truth against an independently-known-valid textbook IPv4
        // header (checksum bytes 10-11 = 0xb1e6), so `ip_checksum`'s sign
        // convention can never silently invert again without a test failing
        // -- the class of bug `frag_accepts_a_genuinely_valid_header` above
        // catches at the `fragment_ipv4` level, this catches at the raw
        // helper level.
        let mut hdr: [u8; 20] = [
            0x45, 0x00, 0x00, 0x3c, 0x1c, 0x46, 0x40, 0x00, 0x40, 0x06, 0xb1, 0xe6, 0xac, 0x10,
            0x0a, 0x63, 0xac, 0x10, 0x0a, 0x0c,
        ];
        let stored = u16::from_be_bytes([hdr[10], hdr[11]]);
        hdr[10] = 0;
        hdr[11] = 0;
        assert_eq!(ip_checksum(&hdr), stored);
    }

    #[test]
    fn frag_ip_options_unsupported() {
        // IHL=6 → header_len=24 (has options)
        let mut pkt = make_ipv4(100, 0x1234, false);
        pkt[0] = 0x46;
        assert!(fragment_ipv4(&pkt, 100).is_none());
    }

    #[test]
    fn frag_max_size_too_small() {
        let pkt = make_ipv4(100, 0x1234, false);
        // max_size=20 → max_payload=0 → None
        assert!(fragment_ipv4(&pkt, 27).is_none());
    }

    #[test]
    fn frag_single_fragment_boundary() {
        // payload 200 bytes, max_size=1200 → 200+20=220 ≤ 1200 → no fragmentation
        let pkt = make_ipv4(200, 0x1234, false);
        assert!(fragment_ipv4(&pkt, 1200).is_none());
    }

    #[test]
    fn frag_1228_packet_into_two() {
        // This is the real-world case: 1228-byte SSH/TCP packet with QUIC
        // max_datagram_size=1200.
        // payload = 1228 - 20 = 1208
        // max_payload = (1200 - 20) & !7 = 1180 & 0xFFF8 = 1176
        // Fragment 1: 20 + 1176 = 1196 bytes, offset=0, MF=1
        // Fragment 2: 20 + (1208-1176=32) = 52 bytes, offset=1176/8=147, MF=0
        let pkt = make_ipv4(1208, 0xABCD, false);
        let frags = fragment_ipv4(&pkt, 1200).expect("should fragment");
        assert_eq!(frags.len(), 2);

        // Fragment 1
        assert_eq!(frags[0].len(), 1196, "frag 1 length");
        assert_eq!(frags[0][0] >> 4, 4, "IPv4");
        assert_eq!(frags[0][4], 0xAB, "id hi");
        assert_eq!(frags[0][5], 0xCD, "id lo");
        let total_len1 = u16::from_be_bytes([frags[0][2], frags[0][3]]);
        assert_eq!(total_len1, 1196, "frag 1 total length");
        // MF=1, offset=0
        assert!(frags[0][6] & 0x20 != 0, "frag 1 should have MF set");
        let offset1 = ((frags[0][6] as u16 & 0x1F) << 8) | frags[0][7] as u16;
        assert_eq!(offset1, 0, "frag 1 offset = 0");
        // DF flag should be preserved (was 0)
        assert_eq!(frags[0][6] & 0x40, 0, "DF preserved (was 0)");
        // Verify checksum on frag 1
        let csum1 = u16::from_be_bytes([frags[0][10], frags[0][11]]);
        let mut hdr1 = frags[0][..20].to_vec();
        hdr1[10] = 0;
        hdr1[11] = 0;
        assert_eq!(csum1, ip_checksum(&hdr1), "frag 1 checksum");
        // Payload content (first 1176 bytes of original)
        assert_eq!(&frags[0][20..], &pkt[20..20 + 1176], "frag 1 payload");

        // Fragment 2
        assert_eq!(frags[1].len(), 52, "frag 2 length");
        let total_len2 = u16::from_be_bytes([frags[1][2], frags[1][3]]);
        assert_eq!(total_len2, 52, "frag 2 total length");
        // MF=0, offset=147
        assert!(frags[1][6] & 0x20 == 0, "frag 2 should NOT have MF set");
        let offset2 = ((frags[1][6] as u16 & 0x1F) << 8) | frags[1][7] as u16;
        assert_eq!(offset2, 1176 / 8, "frag 2 offset = 147");
        // Verify checksum on frag 2
        let csum2 = u16::from_be_bytes([frags[1][10], frags[1][11]]);
        let mut hdr2 = frags[1][..20].to_vec();
        hdr2[10] = 0;
        hdr2[11] = 0;
        assert_eq!(csum2, ip_checksum(&hdr2), "frag 2 checksum");
        // Remaining payload
        assert_eq!(&frags[1][20..], &pkt[20 + 1176..], "frag 2 payload");
    }

    #[test]
    fn frag_preserves_df_flag() {
        // DF=1, payload 1208, max 1200 → should preserve DF=1 in fragments
        let pkt = make_ipv4(1208, 0x42, true);
        let frags = fragment_ipv4(&pkt, 1200).expect("should fragment");
        assert_eq!(frags.len(), 2);
        assert!(frags[0][6] & 0x40 != 0, "DF preserved in frag 1");
        assert!(frags[1][6] & 0x40 != 0, "DF preserved in frag 2");
    }

    #[test]
    fn frag_three_fragments() {
        // payload = 3000 bytes, max_size = 1200
        // max_payload = 1176
        // frag 1: offset=0, payload=1176 (total=1196)
        // frag 2: offset=147, payload=1176 (total=1196)
        // frag 3: offset=294, payload=648 (total=668)
        let pkt = make_ipv4(3000, 0x99, false);
        let frags = fragment_ipv4(&pkt, 1200).expect("should fragment");
        assert_eq!(frags.len(), 3);
        // All three share the ID
        assert_eq!(frags[0][4], 0x00);
        assert_eq!(frags[0][5], 0x99);
        assert_eq!(frags[1][4], 0x00);
        assert_eq!(frags[1][5], 0x99);
        assert_eq!(frags[2][4], 0x00);
        assert_eq!(frags[2][5], 0x99);
        // MF flags
        assert!(frags[0][6] & 0x20 != 0, "frag 1 MF");
        assert!(frags[1][6] & 0x20 != 0, "frag 2 MF");
        assert!(frags[2][6] & 0x20 == 0, "frag 3 no MF");
        // Offsets
        let off0 = ((frags[0][6] as u16 & 0x1F) << 8) | frags[0][7] as u16;
        let off1 = ((frags[1][6] as u16 & 0x1F) << 8) | frags[1][7] as u16;
        let off2 = ((frags[2][6] as u16 & 0x1F) << 8) | frags[2][7] as u16;
        assert_eq!(off0, 0);
        assert_eq!(off1, 1176 / 8);
        assert_eq!(off2, 2352 / 8);
        // Payload sizes
        assert_eq!(frags[0].len() - 20, 1176);
        assert_eq!(frags[1].len() - 20, 1176);
        assert_eq!(frags[2].len() - 20, 3000 - 2352);
        // Full payload reassembly
        let mut reassembled = Vec::new();
        reassembled.extend_from_slice(&frags[0][20..]);
        reassembled.extend_from_slice(&frags[1][20..]);
        reassembled.extend_from_slice(&frags[2][20..]);
        assert_eq!(reassembled.len(), 3000);
        assert_eq!(&reassembled, &pkt[20..], "reassembled payload matches");
    }

    #[test]
    fn frag_checksum_correct() {
        // Verify that each fragment's IP checksum is valid
        let pkt = make_ipv4(1208, 0x1234, false);
        let frags = fragment_ipv4(&pkt, 1200).expect("should fragment");
        for (i, frag) in frags.iter().enumerate() {
            let stored = u16::from_be_bytes([frag[10], frag[11]]);
            let mut hdr = frag[..20].to_vec();
            hdr[10] = 0;
            hdr[11] = 0;
            let computed = ip_checksum(&hdr);
            assert_eq!(stored, computed, "frag {i} checksum mismatch");
        }
    }

    // ── IPv6 fragmentation tests (FRAG-002) ──────────────────────────────

    fn make_ipv6(payload_len: usize) -> Vec<u8> {
        let mut p = vec![0u8; 40 + payload_len];
        p[0] = 0x60; // IPv6
        p[6] = 6; // TCP next header
        for i in 0..payload_len {
            p[40 + i] = (i & 0xFF) as u8;
        }
        p
    }

    #[test]
    fn frag6_no_fragmentation_needed() {
        let pkt = make_ipv6(40);
        assert!(fragment_ipv6(&pkt, 1, 1200).is_none());
    }

    #[test]
    fn frag6_max_size_too_small() {
        let pkt = make_ipv6(1240);
        assert!(fragment_ipv6(&pkt, 1, FRAG6_HEADER_LEN).is_none());
    }

    #[test]
    fn frag6_splits_into_two_and_reassembles() {
        // 1280-byte packet (TUN_MTU), max_dgram=1200 -- the real-world case.
        let pkt = make_ipv6(1240);
        let frags = fragment_ipv6(&pkt, 42, 1200).expect("should fragment");
        assert_eq!(frags.len(), 2);
        for f in &frags {
            assert!(f.len() <= 1200);
            assert_eq!(f[0], FRAG6_MAGIC);
            assert_eq!(u32::from_be_bytes([f[1], f[2], f[3], f[4]]), 42);
        }
        // MF set on first, clear on last
        assert_eq!(frags[0][7], 1);
        assert_eq!(frags[1][7], 0);

        let mut r = Ipv6Reassembler::new();
        assert_eq!(r.accept(&frags[0]), Ipv6FragmentOutcome::Waiting);
        match r.accept(&frags[1]) {
            Ipv6FragmentOutcome::Complete(full) => assert_eq!(full, pkt),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn frag6_reassembles_out_of_order() {
        let pkt = make_ipv6(1240);
        let frags = fragment_ipv6(&pkt, 7, 1200).expect("should fragment");
        assert_eq!(frags.len(), 2);

        let mut r = Ipv6Reassembler::new();
        assert_eq!(r.accept(&frags[1]), Ipv6FragmentOutcome::Waiting);
        match r.accept(&frags[0]) {
            Ipv6FragmentOutcome::Complete(full) => assert_eq!(full, pkt),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn frag6_three_fragments_reassemble() {
        // 1250-byte packet (under the 1280 TUN_MTU ceiling), small max_dgram
        // forces 3 fragments: 492 + 492 + 266.
        let pkt = make_ipv6(1210);
        let frags = fragment_ipv6(&pkt, 99, 500).expect("should fragment");
        assert_eq!(frags.len(), 3);

        let mut r = Ipv6Reassembler::new();
        assert_eq!(r.accept(&frags[0]), Ipv6FragmentOutcome::Waiting);
        assert_eq!(r.accept(&frags[1]), Ipv6FragmentOutcome::Waiting);
        match r.accept(&frags[2]) {
            Ipv6FragmentOutcome::Complete(full) => assert_eq!(full, pkt),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn frag6_not_a_fragment_passthrough() {
        let mut r = Ipv6Reassembler::new();
        let ordinary = make_ipv6(40);
        assert_eq!(r.accept(&ordinary), Ipv6FragmentOutcome::NotAFragment);
    }

    #[test]
    fn frag6_truncated_envelope_rejected() {
        let mut r = Ipv6Reassembler::new();
        let truncated = [FRAG6_MAGIC, 0, 0, 0];
        assert_eq!(r.accept(&truncated), Ipv6FragmentOutcome::Rejected);
    }

    #[test]
    fn frag6_oversized_claim_rejected() {
        // Claims an offset that would put the reassembled packet over
        // MAX_REASSEMBLED_IPV6_PACKET (1280).
        let mut r = Ipv6Reassembler::new();
        let mut bogus = vec![FRAG6_MAGIC, 0, 0, 0, 1]; // id=1
        bogus.extend_from_slice(&2000u16.to_be_bytes()); // offset=2000
        bogus.push(0); // more=0
        bogus.extend_from_slice(&[0u8; 10]); // payload
        assert_eq!(r.accept(&bogus), Ipv6FragmentOutcome::Rejected);
    }

    #[test]
    fn frag6_stale_entry_gc_prevents_unbounded_growth() {
        let mut r = Ipv6Reassembler::new();
        let pkt = make_ipv6(1240);
        let frags = fragment_ipv6(&pkt, 1, 1200).expect("should fragment");
        // Only send the first fragment of many distinct ids, never completing any.
        for id in 0..(MAX_REASSEMBLY_ENTRIES as u32 + 5) {
            let mut f = frags[0].clone();
            f[1..5].copy_from_slice(&id.to_be_bytes());
            assert_eq!(r.accept(&f), Ipv6FragmentOutcome::Waiting);
        }
        assert!(r.partial.len() <= MAX_REASSEMBLY_ENTRIES);
    }
}
