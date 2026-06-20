//! TUN device creation and I/O.
//!
//! The device is immediately split into [`TunReader`] and [`TunWriter`] halves
//! so that reads and writes can happen concurrently without locking.

use std::net::Ipv4Addr;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tun::{Configuration, DeviceReader, DeviceWriter};

/// MTU sized to fit within QUIC datagram limits.
const TUN_MTU: u16 = 1200;

/// Read half of the TUN device. Owned by [`forward::run_mesh`].
pub struct TunReader {
    reader: DeviceReader,
}

/// Write half of the TUN device. Owned by [`forward::spawn_tun_writer`].
pub struct TunWriter {
    writer: DeviceWriter,
}

/// Creates a TUN device with the given virtual IP and /10 netmask (100.64.0.0/10),
/// then splits it into independent read/write halves.
pub fn create(addr: Ipv4Addr) -> Result<(TunReader, TunWriter)> {
    let gateway = Ipv4Addr::new(100, 64, 0, 1);
    let mut config = Configuration::default();
    config
        .address(addr)
        .destination(gateway)
        .netmask((255, 192, 0, 0)) // /10
        .mtu(TUN_MTU)
        .up();

    #[cfg(target_os = "linux")]
    config.platform_config(|p| {
        p.ensure_root_privileges(true);
    });

    let device = tun::create_as_async(&config)?;
    tracing::info!(%addr, "TUN device created");

    let (writer, reader) = device.split()?;
    Ok((TunReader { reader }, TunWriter { writer }))
}

impl TunReader {
    pub async fn read_packet(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.reader.read(buf).await?;
        Ok(n)
    }
}

impl TunWriter {
    pub async fn write_packet(&mut self, packet: &[u8]) -> Result<()> {
        self.writer.write_all(packet).await?;
        Ok(())
    }
}
