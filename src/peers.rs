use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use iroh::endpoint::Connection;

use crate::control::PeerInfo;

pub struct IpAllocator {
    subnet_index: u8,
    next_octet: u8,
}

impl IpAllocator {
    pub fn for_subnet(subnet_index: u8) -> Self {
        Self {
            subnet_index,
            next_octet: 2,
        }
    }

    pub fn next(&mut self) -> Ipv4Addr {
        let ip = Ipv4Addr::new(100, 64, self.subnet_index, self.next_octet);
        self.next_octet += 1;
        ip
    }
}

#[derive(Clone)]
pub struct PeerTable {
    inner: Arc<RwLock<HashMap<Ipv4Addr, PeerEntry>>>,
}

pub struct PeerEntry {
    pub conn: Connection,
    pub endpoint_id: String,
}

impl PeerTable {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add(&self, ip: Ipv4Addr, conn: Connection, endpoint_id: String) {
        self.inner
            .write()
            .unwrap()
            .insert(ip, PeerEntry { conn, endpoint_id });
    }

    pub fn remove(&self, ip: &Ipv4Addr) -> Option<Connection> {
        self.inner.write().unwrap().remove(ip).map(|e| e.conn)
    }

    pub fn lookup(&self, ip: &Ipv4Addr) -> Option<Connection> {
        self.inner.read().unwrap().get(ip).map(|e| e.conn.clone())
    }

    pub fn all_connections(&self) -> Vec<(Ipv4Addr, Connection)> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(ip, e)| (*ip, e.conn.clone()))
            .collect()
    }

    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(ip, e)| PeerInfo {
                ip: *ip,
                endpoint_id: e.endpoint_id.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_allocator_sequential() {
        let mut alloc = IpAllocator::for_subnet(0);
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 2));
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 3));
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 4));
    }

    #[test]
    fn test_peer_table_empty_lookup() {
        let table = PeerTable::new();
        let ip = Ipv4Addr::new(100, 64, 0, 2);
        assert!(table.lookup(&ip).is_none());
    }

    #[test]
    fn test_peer_table_empty_infos() {
        let table = PeerTable::new();
        assert!(table.peer_infos().is_empty());
    }
}
