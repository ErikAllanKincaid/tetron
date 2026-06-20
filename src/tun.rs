use std::net::Ipv4Addr;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tun::{Configuration, DeviceReader, DeviceWriter};

const TUN_MTU: u16 = 1200;

pub struct TunReader {
    reader: DeviceReader,
}

pub struct TunWriter {
    writer: DeviceWriter,
}

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
