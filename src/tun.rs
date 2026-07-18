//! TUN device creation and I/O.
//!
//! The device is immediately split into [`TunReader`] and [`TunWriter`] halves
//! so that reads and writes can happen concurrently without locking.

// These support the desktop TUN setup (address/route/link configuration via
// `ifconfig`/`ip`/netlink) and the CGNAT preflight, none of which compile on
// Android where the packet interface is a `VpnService` fd.
#[cfg(not(target_os = "android"))]
use std::net::{Ipv4Addr, Ipv6Addr};
#[cfg(not(target_os = "android"))]
use std::process::Command;

#[cfg(not(target_os = "android"))]
use anyhow::{Context, Result, bail};
// The desktop TUN device (the `tun` crate) and its async I/O helpers only exist
// off Android, where the packet interface is a `VpnService` fd instead.
#[cfg(not(target_os = "android"))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(not(target_os = "android"))]
use tun::{Configuration, DeviceReader, DeviceWriter};

/// Read side of a packet interface. Fills the spare capacity of `buf` with one
/// IP packet and returns the number of bytes read. Abstracts the concrete TUN
/// device so the forwarding loop can run over any packet source: the desktop
/// TUN, an Android `VpnService` fd, an iOS `NEPacketTunnelFlow`, or an in-memory
/// fake in tests. Reading into caller-owned spare capacity keeps the forward
/// loop's zero-copy `split_to(n).freeze()` hand-off.
///
/// Contract: `Ok(0)` means "no packet this time, retry" — the forwarding loop
/// treats it as a spurious wakeup and loops again. End-of-stream (e.g. an
/// Android `VpnService` fd whose descriptor is revoked/closed) MUST surface as
/// `Err`, never as a perpetual `Ok(0)`, or `run_mesh` would busy-spin at 100%
/// CPU. The desktop TUN never returns 0, so this only binds future impls.
pub trait TunRead: Send + 'static {
    fn read_into(
        &mut self,
        buf: &mut bytes::BytesMut,
    ) -> impl core::future::Future<Output = anyhow::Result<usize>> + Send;
}

/// Write side of a packet interface. Writes one IP packet to the device.
pub trait TunWrite: Send + 'static {
    fn write_packet(
        &mut self,
        packet: &[u8],
    ) -> impl core::future::Future<Output = anyhow::Result<()>> + Send;
}

/// MTU for the TUN device. IPv6 mandates a minimum link MTU of 1280 bytes
/// (RFC 8200 §5); Linux refuses to enable IPv6 on a device with a smaller MTU,
/// which silently breaks IPv6 address/route installation (`configure_ipv6` /
/// `route_peer_range` fail with `EINVAL`). 1280 is also the value WireGuard and
/// Tailscale use for their TUN interfaces for the same reason, and it still
/// fits within QUIC datagram limits.
#[cfg(not(target_os = "android"))]
const TUN_MTU: u16 = 1280;

/// Read half of the TUN device. Owned by [`forward::run_mesh`].
#[cfg(not(target_os = "android"))]
pub struct TunReader {
    reader: DeviceReader,
}

/// Write half of the TUN device. Owned by [`forward::spawn_tun_writer`].
#[cfg(not(target_os = "android"))]
pub struct TunWriter {
    writer: DeviceWriter,
}

/// Refuse to start if `subnet` overlaps an existing local IPv4 interface, so the
/// overlay never hijacks the host's own routing (SUBNET-012). This is the
/// subnet-aware replacement for the removed hardcoded CGNAT check: it fires only
/// on a genuine overlap between the *chosen* overlay subnet and a real local
/// network, so it does not block the Tailscale-coexistence case. Fail-open: if
/// interfaces can't be enumerated it allows startup rather than blocking on a
/// missing tool.
#[cfg(not(target_os = "android"))]
pub fn check_subnet_overlap(subnet: crate::membership::Subnet) -> Result<()> {
    let (base, prefix) = subnet;
    for (iface, addr, plen) in local_ipv4_interfaces() {
        if crate::membership::subnets_overlap((addr, plen), subnet) {
            bail!(
                "overlay subnet {base}/{prefix} overlaps interface {iface} ({addr}/{plen}); \
                 pick a different range with `tetron config set subnet <cidr>` and restart"
            );
        }
    }
    Ok(())
}

/// Enumerate local IPv4 `(interface, address, prefix)` via `ip -o -4 addr show`,
/// skipping loopback. Returns empty (fail-open) if `ip` is unavailable or fails.
#[cfg(not(target_os = "android"))]
fn local_ipv4_interfaces() -> Vec<(String, Ipv4Addr, u8)> {
    let out = match Command::new("ip").args(["-o", "-4", "addr", "show"]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out);
    let mut result = Vec::new();
    // Each line: "<idx>: <iface>    inet <A.B.C.D>/<prefix> [brd ...] scope ..."
    for line in text.lines() {
        let mut toks = line.split_whitespace();
        let _idx = toks.next();
        let iface = match toks.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if iface == "lo" {
            continue;
        }
        // Advance to the token after "inet" and parse its CIDR.
        while let Some(t) = toks.next() {
            if t == "inet"
                && let Some(cidr) = toks.next()
                && let Some((a, p)) = cidr.split_once('/')
                && let (Ok(addr), Ok(plen)) = (a.parse::<Ipv4Addr>(), p.parse::<u8>())
            {
                result.push((iface.clone(), addr, plen));
                break;
            }
        }
    }
    result
}

/// Creates a TUN device with the given virtual IPs and splits it into
/// independent read/write halves. IPv4 gets the configured overlay subnet's
/// netmask (derived from its prefix length) so the kernel installs the connected
/// route for the whole peer range; IPv6 gets a /7 prefix (`200::/7`) the same way.
#[cfg(not(target_os = "android"))]
pub async fn create(
    v4: Ipv4Addr,
    v6: Ipv6Addr,
    subnet: crate::membership::Subnet,
) -> Result<(TunReader, TunWriter, String)> {
    let (_, prefix) = subnet;
    let gateway = crate::membership::subnet_gateway(subnet);
    let nm = crate::membership::subnet_netmask(prefix).octets();
    let mut config = Configuration::default();
    config
        .address(v4)
        .destination(gateway)
        .netmask((nm[0], nm[1], nm[2], nm[3]))
        .mtu(TUN_MTU)
        .up();

    #[cfg(target_os = "linux")]
    config.platform_config(|p| {
        p.ensure_root_privileges(true);
    });

    let device = tun::create_as_async(&config)?;
    let tun_name = device
        .as_ref()
        .tun_name()
        .unwrap_or_else(|_| "unknown".to_string());
    tracing::info!(addr = %v4, ipv6 = %v6, tun = %tun_name, "TUN device created");

    if let Err(e) = configure_ipv6(&tun_name, v6).await {
        tracing::warn!(error = %e, "failed to configure IPv6 on TUN (IPv6 routing will not work)");
    }

    let (writer, reader) = device.split()?;
    Ok((TunReader { reader }, TunWriter { writer }, tun_name))
}

/// Assigns the TUN's own IPv6 address. The `200::/7` peer range is routed into
/// the TUN separately by [`route_peer_range`], which must run *after* the link
/// is up — assigning the address here at creation time (link still down) is not
/// enough on Linux, where the kernel does not reliably install the connected
/// route until the interface comes up.
#[cfg(target_os = "linux")]
async fn configure_ipv6(tun_name: &str, addr: Ipv6Addr) -> Result<()> {
    use futures::TryStreamExt;
    use std::net::IpAddr;

    let (connection, handle, _) = rtnetlink::new_connection().context("open netlink socket")?;
    // The connection future must be polled while we use the handle; abort it
    // once configuration is done.
    let conn = tokio::spawn(connection);

    let result = async {
        let index = handle
            .link()
            .get()
            .match_name(tun_name.to_owned())
            .execute()
            .try_next()
            .await
            .context("query TUN link")?
            .with_context(|| format!("TUN link {tun_name} not found"))?
            .header
            .index;

        // /128: just our own address. The peer-range route is added explicitly
        // after link-up in `route_peer_range`. `replace()` keeps it idempotent
        // across daemon restarts.
        handle
            .address()
            .add(index, IpAddr::V6(addr), 128)
            .replace()
            .execute()
            .await
            .context("add IPv6 address via netlink")?;

        Ok(())
    }
    .await;

    conn.abort();
    result
}

#[cfg(target_os = "macos")]
async fn configure_ipv6(tun_name: &str, addr: Ipv6Addr) -> Result<()> {
    // macOS has no netlink; assign the address via the BSD tools. The peer-range
    // route is added separately by `route_peer_range` after link-up.
    let status = Command::new("ifconfig")
        .args([tun_name, "inet6", &addr.to_string(), "prefixlen", "128"])
        .status()
        .context("run ifconfig")?;
    anyhow::ensure!(status.success(), "ifconfig inet6 failed with {status}");
    Ok(())
}

/// Routes the peer range into the TUN. Must be called *after* the interface is
/// up (see [`set_link_up`]). On Linux only the IPv6 route needs adding: the
/// kernel does not reliably install an IPv6 connected route while the link is
/// down (peer traffic would otherwise leak out the host's default IPv6 route),
/// whereas it re-installs the network's own IPv4 connected route from the
/// interface's own netmask automatically on link-up. On macOS the
/// point-to-point utun installs neither range reliably, so *both* the
/// network's own IPv4 subnet and its IPv6 range are added explicitly.
/// Idempotent — safe to call on every `up` cycle.
///
/// `network_prefix`/`prefix_len` (IPV6-003) is that network's own `/56` block
/// (`membership::ipv6_network_prefix`, `IPV6_NETWORK_PREFIX_LEN`) rather than
/// one system-wide `200::/7` — every joined network gets its own disjoint
/// route into its own TUN device, closing the "IPv6 works on one segment
/// only" limitation MULTISEG-003 left open. `subnet` is that same network's
/// own IPv4 subnet — only consumed on macOS (MACOS-001), where the IPv4
/// route must be installed explicitly; Linux's kernel handles it via the
/// interface's own address/netmask, so it's unused there.
#[cfg(target_os = "linux")]
pub async fn route_peer_range(
    tun_name: &str,
    _subnet: crate::membership::Subnet,
    network_prefix: Ipv6Addr,
    prefix_len: u8,
) -> Result<()> {
    use futures::TryStreamExt;
    use rtnetlink::RouteMessageBuilder;

    let (connection, handle, _) = rtnetlink::new_connection().context("open netlink socket")?;
    let conn = tokio::spawn(connection);

    let result = async {
        let index = handle
            .link()
            .get()
            .match_name(tun_name.to_owned())
            .execute()
            .try_next()
            .await
            .context("query TUN link")?
            .with_context(|| format!("TUN link {tun_name} not found"))?
            .header
            .index;

        let route = RouteMessageBuilder::<Ipv6Addr>::new()
            .destination_prefix(network_prefix, prefix_len)
            .output_interface(index)
            .build();
        handle
            .route()
            .add(route)
            .replace()
            .execute()
            .await
            .with_context(|| format!("add {network_prefix}/{prefix_len} route via netlink"))?;

        Ok(())
    }
    .await;

    conn.abort();
    result
}

#[cfg(target_os = "macos")]
pub async fn route_peer_range(
    tun_name: &str,
    subnet: crate::membership::Subnet,
    network_prefix: Ipv6Addr,
    prefix_len: u8,
) -> Result<()> {
    // utun is point-to-point, so the address prefix alone does not reliably
    // create the range route — we add both families explicitly. The IPv4
    // range is only installed implicitly by the `tun` crate at device
    // creation and macOS drops it across an `up`/`down` cycle, so (like the
    // IPv6 range) we re-add it on every activate or peers become unreachable
    // over IPv4 while IPv6 still works. `route add` fails if the route
    // already exists (e.g. an earlier `up`), so delete any stale entry first
    // and ignore its result.
    //
    // MACOS-001: this used to hardcode the pre-fork `100.64.0.0/10` CGNAT
    // range regardless of the network's actual configured subnet, silently
    // misrouting IPv4 on any network whose subnet differed from that literal
    // (which is every network by default, since tetron's own default is
    // `10.88.0.0/24` — found 2026-07-17, fixed 2026-07-18, same bug shape as
    // MULTISEG-007's join-side IP derivation). Now takes the network's own
    // subnet and formats its real CIDR instead.
    let (base, prefix) = subnet;
    let v4_net = format!("{base}/{prefix}");
    let v6_net = format!("{network_prefix}/{prefix_len}");
    for (family, net) in [("-inet", v4_net.as_str()), ("-inet6", v6_net.as_str())] {
        let _ = Command::new("route")
            .args(["-n", "delete", family, "-net", net, "-interface", tun_name])
            .status();
        let status = Command::new("route")
            .args(["-n", "add", family, "-net", net, "-interface", tun_name])
            .status()
            .with_context(|| format!("run route add {family} {net}"))?;
        anyhow::ensure!(
            status.success(),
            "route add {family} {net} failed with {status}"
        );
    }
    Ok(())
}

/// Install host routes for our *own* dual-stack addresses via the loopback
/// interface so traffic to ourselves (e.g. `ping dario.field.ray` resolving to
/// our own IP) is short-circuited locally instead of being sent out the TUN —
/// where the forwarding loop would drop it as "no peer for dst".
///
/// On a normal broadcast interface macOS auto-installs a `<own-ip> -> lo0` route
/// for exactly this. A point-to-point `utun` does not get one (the local address
/// only exists as the source end of the `addr --> gateway` pair), so we add it
/// explicitly, mirroring what Tailscale does. Delete-then-add keeps it
/// idempotent across `up`/`down` cycles. Must run after the address is assigned.
///
/// On Linux this is a no-op: assigning an address makes the kernel add a
/// `local` route in the `local` table that already delivers self-traffic via
/// loopback, so pinging your own TUN address works out of the box.
#[cfg(target_os = "macos")]
pub async fn route_self_loopback(v4: Ipv4Addr, v6: Ipv6Addr) -> Result<()> {
    for (family, addr) in [("-inet", v4.to_string()), ("-inet6", v6.to_string())] {
        let _ = Command::new("route")
            .args(["-n", "delete", family, "-host", &addr, "-interface", "lo0"])
            .status();
        let status = Command::new("route")
            .args(["-n", "add", family, "-host", &addr, "-interface", "lo0"])
            .status()
            .context("run route add (loopback self-route)")?;
        anyhow::ensure!(
            status.success(),
            "route add {family} -host {addr} via lo0 failed with {status}"
        );
    }
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "android")))]
pub async fn route_self_loopback(_v4: Ipv4Addr, _v6: Ipv6Addr) -> Result<()> {
    // Linux installs the loopback `local` route automatically on address
    // assignment; self-traffic already works without an explicit route.
    Ok(())
}

/// Bring the TUN interface administratively up (used when activating the VPN).
#[cfg(not(target_os = "android"))]
pub fn set_link_up(tun_name: &str) -> Result<()> {
    set_link_state(tun_name, true)
}

/// Bring the TUN interface administratively down (standby). The underlying file
/// descriptor stays open, so the device can be brought back up without
/// recreating it.
#[cfg(not(target_os = "android"))]
pub fn set_link_down(tun_name: &str) -> Result<()> {
    set_link_state(tun_name, false)
}

/// Explicitly destroy a TUN device (MULTISEG-003). Before per-network TUN
/// teardown existed, the code relied entirely on the kernel auto-removing a
/// non-persistent TUN once the owning process's last fd closed — fine when
/// there was exactly one TUN, alive for the daemon's whole life, but stale
/// devices have been observed surviving daemon restarts/crashes in practice
/// (the "TUN-CLEANUP" procedural workaround this replaces). Per-network
/// teardown (`leave`/`nuke`/kick-of-self) now runs mid-process, with other
/// networks' devices still live, so an explicit, synchronous delete here is
/// the belt to the fd-close-triggers-cleanup suspenders rather than relying
/// solely on timing.
#[cfg(target_os = "linux")]
pub fn delete(tun_name: &str) -> Result<()> {
    let status = Command::new("ip")
        .args(["link", "delete", tun_name])
        .status()
        .context("run ip link delete")?;
    anyhow::ensure!(status.success(), "ip link delete failed with {status}");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn delete(tun_name: &str) -> Result<()> {
    let status = Command::new("ifconfig")
        .args([tun_name, "destroy"])
        .status()
        .context("run ifconfig destroy")?;
    anyhow::ensure!(status.success(), "ifconfig destroy failed with {status}");
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn set_link_state(tun_name: &str, up: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let state = if up { "up" } else { "down" };
        let status = Command::new("ifconfig")
            .args([tun_name, state])
            .status()
            .context("run ifconfig")?;
        anyhow::ensure!(status.success(), "ifconfig {state} failed with {status}");
    }
    #[cfg(target_os = "linux")]
    {
        let state = if up { "up" } else { "down" };
        let status = Command::new("ip")
            .args(["link", "set", tun_name, state])
            .status()
            .context("run ip link set")?;
        anyhow::ensure!(status.success(), "ip link set {state} failed with {status}");
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
impl TunRead for TunReader {
    /// Reads one packet from the TUN device, appending into the spare capacity
    /// of `buf` without zeroing or reallocating. The caller MUST ensure `buf`
    /// has at least one MTU of spare capacity. Returns the number of bytes read.
    async fn read_into(&mut self, buf: &mut bytes::BytesMut) -> anyhow::Result<usize> {
        let n = self.reader.read_buf(buf).await?;
        Ok(n)
    }
}

#[cfg(not(target_os = "android"))]
impl TunWrite for TunWriter {
    async fn write_packet(&mut self, packet: &[u8]) -> anyhow::Result<()> {
        self.writer.write_all(packet).await?;
        Ok(())
    }
}
