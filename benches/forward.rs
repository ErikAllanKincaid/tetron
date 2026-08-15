//! Microbenchmarks for the per-packet data path.
//!
//! These isolate the CPU/allocation work tetron does **per forwarded packet**,
//! away from the network. The Scaleway harness (`tests/bench/`) measures
//! end-to-end throughput, but on a shared-vCPU box single-stream TCP is
//! loss/congestion-bound, which hides per-packet CPU savings. These benches are
//! the complementary instrument: they hold everything else constant and time
//! only the work the data plane does, so a regression (or the gain from the
//! zero-copy hand-off) is visible and stable run-to-run.
//!
//! Two groups:
//! - `handoff` — the packet ownership transfer that the zero-copy change
//!   touched. `copy` reproduces the old allocate-and-copy (`Bytes::copy_from_slice`
//!   on TX, `Vec::to_vec` on RX); `zerocopy` is the current pooled
//!   `split_to(n).freeze()` (TX) and `Bytes` clone (RX). The delta is the saving.
//! - `parse` — `parse_packet_info`, the unavoidable per-packet parse run on
//!   every forwarded packet (peer routing, anti-spoof, magic-DNS). A regression guard.
//! - `fragment` — the oversized-packet path (`FRAG-006`). `allocating`
//!   reproduces the old shape (a `Vec<Vec<u8>>` from the fragmenter, then a
//!   `Bytes::copy_from_slice` per fragment at the send site); `pooled` is the
//!   current `packet::Fragmenter`, which slices each fragment out of a reused
//!   pool. The delta is the saving, paid per fragmented packet.

use bytes::{Bytes, BytesMut};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use tetron::packet;

/// Datagram sizes spanning the MTU: a 64-byte control/ACK-ish packet and a
/// full 1280-byte (TUN MTU) data packet. The copy cost scales with size; the
/// zero-copy path should be flat.
const SIZES: &[usize] = &[64, 1280];

/// Pool chunk size mirrors `forward::TX_POOL_CHUNK` (64 KiB) so the amortized
/// allocation behaviour matches production.
const POOL_CHUNK: usize = 64 * 1024;
const MAX_DATAGRAM: usize = 1500;

/// Build a minimal but valid IPv4/TCP packet of `len` bytes destined for
/// `100.64.0.3:dst_port`, padded with zeros. Mirrors the test helpers in
/// `forward.rs` so the parser walks a realistic header.
fn ipv4_tcp_packet(len: usize, dst_port: u16) -> Vec<u8> {
    let mut p = vec![0u8; len.max(24)];
    p[0] = 0x45; // IPv4, IHL=5
    p[9] = 6; // TCP
    p[16..20].copy_from_slice(&[100, 64, 0, 3]); // dst ip
    p[20] = 0;
    p[21] = 80; // src port 80
    p[22] = (dst_port >> 8) as u8;
    p[23] = dst_port as u8;
    p.truncate(len.max(24));
    p
}

/// The packet ownership hand-off: old copy path vs. current zero-copy path,
/// for both the TX (TUN -> peer) and RX (peer -> TUN) directions.
fn bench_handoff(c: &mut Criterion) {
    let mut group = c.benchmark_group("handoff");
    for &size in SIZES {
        let packet = ipv4_tcp_packet(size, 443);
        group.throughput(Throughput::Bytes(size as u64));

        // TX old: allocate a fresh Bytes and copy the packet into it — what
        // `Bytes::copy_from_slice(&buf[..n])` did before the pooled path.
        group.bench_with_input(BenchmarkId::new("tx_copy", size), &packet, |b, pkt| {
            b.iter(|| {
                let owned = Bytes::copy_from_slice(black_box(&pkt[..]));
                black_box(owned)
            });
        });

        // TX new: read into a reused pool and slice the packet out as an owned
        // Bytes sharing the chunk allocation — `split_to(n).freeze()`. The pool
        // is reserved across iterations exactly as `run_mesh` does, so a fresh
        // 64 KiB chunk is amortized over ~50 packets, not paid per iteration.
        group.bench_with_input(BenchmarkId::new("tx_zerocopy", size), &packet, |b, pkt| {
            let mut pool = BytesMut::with_capacity(POOL_CHUNK);
            b.iter(|| {
                if pool.capacity() < MAX_DATAGRAM {
                    pool.reserve(POOL_CHUNK);
                }
                pool.extend_from_slice(black_box(&pkt[..]));
                let out = pool.split_to(pkt.len()).freeze();
                black_box(out)
            });
        });

        // RX old: `datagram.to_vec()` — a heap allocation + copy per inbound
        // packet before handing it to the TUN writer channel.
        let datagram = Bytes::copy_from_slice(&packet);
        group.bench_with_input(BenchmarkId::new("rx_copy", size), &datagram, |b, dg| {
            b.iter(|| {
                let v = black_box(dg).to_vec();
                black_box(v)
            });
        });

        // RX new: the datagram is already an owned `Bytes`; forwarding it is a
        // refcount bump, no copy. This is what `tun_tx.send(datagram)` now does.
        group.bench_with_input(BenchmarkId::new("rx_zerocopy", size), &datagram, |b, dg| {
            b.iter(|| {
                let cloned = black_box(dg).clone();
                black_box(cloned)
            });
        });
    }
    group.finish();
}

/// The QUIC datagram ceiling to fragment against. 1162 is the value observed on
/// a real relay path in the `FRAG-001` bug report, and splits a 1280-byte TUN
/// packet into two fragments — the overwhelmingly common case.
const FRAG_MAX_DATAGRAM: usize = 1162;

/// Build a valid IPv4 packet of `len` bytes with a correct header checksum, so
/// `fragment_ipv4` accepts it (it verifies the checksum before splitting).
fn ipv4_fragmentable(len: usize) -> Vec<u8> {
    let mut p = ipv4_tcp_packet(len, 443);
    p[2] = (len >> 8) as u8;
    p[3] = len as u8;
    p[8] = 64; // TTL
    p[10] = 0;
    p[11] = 0;
    // RFC 1071 internet checksum over the 20-byte header.
    let mut sum: u32 = 0;
    for i in (0..20).step_by(2) {
        sum += u32::from(u16::from_be_bytes([p[i], p[i + 1]]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let csum = !(sum as u16);
    p[10] = (csum >> 8) as u8;
    p[11] = csum as u8;
    p
}

/// The oversized-packet path: old allocate-per-fragment shape vs. the current
/// pooled `Fragmenter` (`FRAG-006`).
fn bench_fragment(c: &mut Criterion) {
    let packet = ipv4_fragmentable(1280);
    let mut group = c.benchmark_group("fragment");
    group.throughput(Throughput::Bytes(packet.len() as u64));

    // Old: a Vec per fragment out of the fragmenter, then a second allocation
    // and copy per fragment at the send site.
    group.bench_function("allocating", |b| {
        b.iter(|| {
            let frags = fragment_ipv4_allocating(black_box(&packet), FRAG_MAX_DATAGRAM)
                .expect("1280 > 1162, must fragment");
            let sent: Vec<Bytes> = frags.iter().map(|f| Bytes::copy_from_slice(f)).collect();
            black_box(sent)
        });
    });

    // New: fragments are sliced out of a pool reused across packets, and the
    // send site clones a `Bytes` (a refcount bump, no copy).
    group.bench_function("pooled", |b| {
        let mut fragmenter = packet::Fragmenter::new();
        b.iter(|| {
            let frags = fragmenter
                .fragment_ipv4(black_box(&packet), FRAG_MAX_DATAGRAM)
                .expect("1280 > 1162, must fragment");
            let sent: Vec<Bytes> = frags.to_vec();
            black_box(sent)
        });
    });

    group.finish();
}

/// The pre-`FRAG-006` fragmenter, reproduced here so the bench can measure what
/// the pooled version replaced. Kept deliberately simple: the fields it writes
/// are the same ones `packet::fragment_ipv4` writes, and the allocation shape
/// (one `Vec` per fragment) is the point being measured.
fn fragment_ipv4_allocating(packet: &[u8], max_size: usize) -> Option<Vec<Vec<u8>>> {
    const HEADER_LEN: usize = 20;
    if packet.len() <= max_size || max_size < HEADER_LEN + 8 {
        return None;
    }
    let payload_len = packet.len() - HEADER_LEN;
    let max_payload = (max_size - HEADER_LEN) & !7;
    let mut fragments = Vec::new();
    let mut offset = 0usize;
    while offset < payload_len {
        let frag_payload_len = max_payload.min(payload_len - offset);
        let mut frag = Vec::with_capacity(HEADER_LEN + frag_payload_len);
        frag.extend_from_slice(&packet[..HEADER_LEN]);
        let total_len = (HEADER_LEN + frag_payload_len) as u16;
        frag[2] = (total_len >> 8) as u8;
        frag[3] = total_len as u8;
        let start = HEADER_LEN + offset;
        frag.extend_from_slice(&packet[start..start + frag_payload_len]);
        fragments.push(frag);
        offset += frag_payload_len;
    }
    Some(fragments)
}

/// `parse_packet_info`: the per-packet header parse run on every forwarded
/// packet, regardless of the hand-off strategy.
fn bench_parse(c: &mut Criterion) {
    let packet = ipv4_tcp_packet(1280, 443);

    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Elements(1));

    group.bench_function("parse_only", |b| {
        b.iter(|| black_box(packet::parse_packet_info(black_box(&packet))));
    });

    group.finish();
}

criterion_group!(benches, bench_handoff, bench_fragment, bench_parse);
criterion_main!(benches);
