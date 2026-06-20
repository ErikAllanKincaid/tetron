use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use iroh::EndpointId;
use iroh::endpoint::Connection;

#[derive(Clone)]
pub struct PeerTable {
    inner: Arc<RwLock<HashMap<Ipv4Addr, PeerEntry>>>,
}

pub struct PeerEntry {
    pub conn: Connection,
    #[allow(dead_code)]
    pub endpoint_id: EndpointId,
}

impl PeerTable {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add(&self, ip: Ipv4Addr, conn: Connection, endpoint_id: EndpointId) {
        self.inner
            .write()
            .unwrap()
            .insert(ip, PeerEntry { conn, endpoint_id });
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

    #[cfg(test)]
    pub fn all_peer_ids(&self) -> Vec<(Ipv4Addr, EndpointId)> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(ip, e)| (*ip, e.endpoint_id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_table_empty_lookup() {
        let table = PeerTable::new();
        let ip = Ipv4Addr::new(100, 64, 0, 2);
        assert!(table.lookup(&ip).is_none());
    }

    #[test]
    fn test_peer_table_empty_ids() {
        let table = PeerTable::new();
        assert!(table.all_peer_ids().is_empty());
    }
}
