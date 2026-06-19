use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tun::{AsyncDevice, Configuration};

const TUN_MTU: u16 = 1200;

pub struct TunDevice {
    device: Arc<Mutex<AsyncDevice>>,
}

impl TunDevice {
    pub fn create_mesh_subnet(addr: Ipv4Addr, subnet_index: u8) -> Result<Self> {
        let gateway = Ipv4Addr::new(100, 64, subnet_index, 1);
        let mut config = Configuration::default();
        config
            .address(addr)
            .destination(gateway)
            .netmask((255, 255, 255, 0))
            .mtu(TUN_MTU)
            .up();

        #[cfg(target_os = "linux")]
        config.platform_config(|p| {
            p.ensure_root_privileges(true);
        });

        let device = tun::create_as_async(&config)?;
        tracing::info!(%addr, subnet_index, "TUN device created (mesh)");
        Ok(Self {
            device: Arc::new(Mutex::new(device)),
        })
    }

    /// Compute the coordinator IP for a given subnet index: 100.64.{subnet_index}.1
    pub fn coordinator_ip(subnet_index: u8) -> Ipv4Addr {
        Ipv4Addr::new(100, 64, subnet_index, 1)
    }

    pub fn share(&self) -> TunDevice {
        TunDevice {
            device: self.device.clone(),
        }
    }

    pub async fn read_packet(&self, buf: &mut [u8]) -> Result<usize> {
        let mut dev = self.device.lock().await;
        let n = dev.read(buf).await?;
        Ok(n)
    }

    pub async fn write_packet(&self, packet: &[u8]) -> Result<()> {
        let mut dev = self.device.lock().await;
        dev.write_all(packet).await?;
        Ok(())
    }
}
