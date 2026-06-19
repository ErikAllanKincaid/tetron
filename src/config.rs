use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Info about a peer in a saved network config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEntry {
    pub ip: Ipv4Addr,
    pub endpoint_id: String,
}

/// A single saved network membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Human-readable network name.
    pub name: String,
    /// EndpointId of the network coordinator (creator).
    pub coordinator_id: String,
    /// Our assigned IP in this network (None if we are the creator).
    pub assigned_ip: Option<Ipv4Addr>,
    /// Subnet index for network isolation (100.64.{subnet_index}.0/24).
    /// Defaults to 0 for backward compatibility.
    #[serde(default)]
    pub subnet_index: u8,
    /// Known peers in this network.
    #[serde(default)]
    pub peers: Vec<PeerEntry>,
}

/// Top-level config stored at `~/.config/pitopi/networks.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
}

fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine config directory")?
        .join("pitopi");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("networks.toml"))
}

/// Load the config file, returning a default empty config if it doesn't exist.
pub fn load() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let contents = std::fs::read_to_string(&path).context("reading networks.toml")?;
    toml::from_str(&contents).context("parsing networks.toml")
}

/// Save the config file.
pub fn save(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let contents = toml::to_string_pretty(config).context("serializing config")?;
    std::fs::write(&path, contents).context("writing networks.toml")?;
    Ok(())
}

/// Add or update a network in the config. If a network with the same name
/// already exists, it is replaced.
pub fn upsert_network(config: &mut AppConfig, network: NetworkConfig) {
    if let Some(existing) = config.networks.iter_mut().find(|n| n.name == network.name) {
        *existing = network;
    } else {
        config.networks.push(network);
    }
}

/// Find the next unused subnet index (starting from 1; 0 is the default for single-network use).
pub fn next_subnet_index(config: &AppConfig) -> u8 {
    let mut used: Vec<u8> = config.networks.iter().map(|n| n.subnet_index).collect();
    used.sort();
    let mut candidate: u8 = 1;
    for idx in used {
        if idx == candidate {
            candidate += 1;
        }
    }
    candidate
}

/// Remove a network by name. Returns true if it was found and removed.
pub fn remove_network(config: &mut AppConfig, name: &str) -> bool {
    let before = config.networks.len();
    config.networks.retain(|n| n.name != name);
    config.networks.len() < before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_roundtrip() {
        let config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "gaming".to_string(),
                    coordinator_id: "abc123def456".to_string(),
                    assigned_ip: Some(Ipv4Addr::new(100, 64, 1, 2)),
                    subnet_index: 1,
                    peers: vec![
                        PeerEntry {
                            ip: Ipv4Addr::new(100, 64, 1, 1),
                            endpoint_id: "coord-id".to_string(),
                        },
                        PeerEntry {
                            ip: Ipv4Addr::new(100, 64, 1, 3),
                            endpoint_id: "peer-id".to_string(),
                        },
                    ],
                },
                NetworkConfig {
                    name: "work".to_string(),
                    coordinator_id: "xyz789".to_string(),
                    assigned_ip: None,
                    subnet_index: 2,
                    peers: vec![],
                },
            ],
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_deserialize_empty() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert_eq!(config, AppConfig::default());
        assert!(config.networks.is_empty());
    }

    #[test]
    fn test_deserialize_no_peers() {
        let toml_str = r#"
[[networks]]
name = "test"
coordinator_id = "abc"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "test");
        assert_eq!(config.networks[0].assigned_ip, None);
        assert!(config.networks[0].peers.is_empty());
    }

    #[test]
    fn test_upsert_new() {
        let mut config = AppConfig::default();
        let net = NetworkConfig {
            name: "test".to_string(),
            coordinator_id: "abc".to_string(),
            assigned_ip: Some(Ipv4Addr::new(100, 64, 0, 2)),
            subnet_index: 0,
            peers: vec![],
        };
        upsert_network(&mut config, net.clone());
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0], net);
    }

    #[test]
    fn test_upsert_replaces_existing() {
        let mut config = AppConfig {
            networks: vec![NetworkConfig {
                name: "test".to_string(),
                coordinator_id: "old".to_string(),
                assigned_ip: None,
                subnet_index: 0,
                peers: vec![],
            }],
        };
        let updated = NetworkConfig {
            name: "test".to_string(),
            coordinator_id: "new".to_string(),
            assigned_ip: Some(Ipv4Addr::new(100, 64, 0, 5)),
            subnet_index: 0,
            peers: vec![],
        };
        upsert_network(&mut config, updated.clone());
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].coordinator_id, "new");
    }

    #[test]
    fn test_next_subnet_index() {
        let config = AppConfig::default();
        assert_eq!(next_subnet_index(&config), 1);

        let config = AppConfig {
            networks: vec![NetworkConfig {
                name: "a".to_string(),
                coordinator_id: "x".to_string(),
                assigned_ip: None,
                subnet_index: 1,
                peers: vec![],
            }],
        };
        assert_eq!(next_subnet_index(&config), 2);

        // Gaps should be filled
        let config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "a".to_string(),
                    coordinator_id: "x".to_string(),
                    assigned_ip: None,
                    subnet_index: 1,
                    peers: vec![],
                },
                NetworkConfig {
                    name: "b".to_string(),
                    coordinator_id: "y".to_string(),
                    assigned_ip: None,
                    subnet_index: 3,
                    peers: vec![],
                },
            ],
        };
        assert_eq!(next_subnet_index(&config), 2);
    }

    #[test]
    fn test_remove_network() {
        let mut config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "keep".to_string(),
                    coordinator_id: "a".to_string(),
                    assigned_ip: None,
                    subnet_index: 0,
                    peers: vec![],
                },
                NetworkConfig {
                    name: "remove-me".to_string(),
                    coordinator_id: "b".to_string(),
                    assigned_ip: None,
                    subnet_index: 0,
                    peers: vec![],
                },
            ],
        };
        assert!(remove_network(&mut config, "remove-me"));
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "keep");
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut config = AppConfig::default();
        assert!(!remove_network(&mut config, "nope"));
    }
}
