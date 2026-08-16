//! On-disk config types (MODULARIZE-003). Zero I/O, zero `config set`/`config
//! get` dispatch logic — those live in [`crate::config::overrides`] and
//! [`crate::config::storage`] respectively. Re-exported from `crate::config`
//! so every existing `crate::config::…` path keeps compiling unchanged.

use std::net::Ipv4Addr;

use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use tetron_proto::TransportMode;

use crate::membership::GroupMode;

#[allow(dead_code)]
mod secret_key_hex {
    use iroh::SecretKey;
    use serde::de::Error;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(key: &SecretKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(key.to_bytes()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SecretKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes: [u8; 32] = hex::decode(&s)
            .map_err(Error::custom)?
            .try_into()
            .map_err(|_| Error::custom("secret key must be 32 bytes"))?;
        Ok(SecretKey::from(bytes))
    }
}

mod option_secret_key_hex {
    use iroh::SecretKey;
    use serde::de::Error;
    use serde::{self, Deserializer, Serializer};

    pub fn serialize<S>(key: &Option<SecretKey>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match key {
            Some(k) => super::secret_key_hex::serialize(k, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SecretKey>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = serde::Deserialize::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes: [u8; 32] = hex::decode(&s)
                    .map_err(Error::custom)?
                    .try_into()
                    .map_err(|_| Error::custom("secret key must be 32 bytes"))?;
                Ok(Some(SecretKey::from(bytes)))
            }
            None => Ok(None),
        }
    }
}

/// Info about a member in a saved network config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    #[serde(default)]
    pub is_coordinator: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// A pre-approved peer that hasn't connected yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedConfigEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// A single saved network membership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Human-friendly network alias (local only, not used for discovery).
    pub name: String,
    /// Membership mode: open or restricted.
    #[serde(default)]
    pub group_mode: GroupMode,
    /// Our assigned IP in this network (None if coordinator, Some if member).
    pub my_ip: Option<Ipv4Addr>,
    /// Our hostname in this network (persisted so it survives daemon restarts).
    /// Fixed at join (MINIMAL-014 removed rename); a member adopts the
    /// coordinator's authoritative (collision-resolved) name on reconverge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_hostname: Option<String>,
    /// Known members in this network.
    #[serde(default)]
    pub members: Vec<MemberEntry>,
    /// Pre-approved peers that haven't connected yet.
    #[serde(default)]
    pub approved: Vec<ApprovedConfigEntry>,
    #[serde(default, with = "option_secret_key_hex")]
    pub network_secret_key: Option<SecretKey>,
    #[serde(default)]
    pub network_public_key: Option<EndpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportMode>,
    /// Identities this coordinator has granted the per-network secret key to
    /// (`tetron admin add`). Local tracking only — the key is shared and not
    /// attributable, so this is the coordinator's record of grants, not a
    /// verifiable roster. Never published in the GroupBlob.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admins: Vec<EndpointId>,
    /// This is an auto-minted 2-peer "direct connection" network (`tetron connect`),
    /// not a user-created mesh. Tagged so `tetron status` can label it `[direct]`
    /// and suppress its (non-shareable) room id.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub direct: bool,
    /// This network's own overlay subnet, distinct from [`AppConfig::subnet`]
    /// (the node-wide operative subnet used to build the single shared TUN,
    /// SUBNET-010). `None` means this network uses the node-wide subnet, same
    /// as today. Additive only (MULTISEG-001) — nothing reads this field yet;
    /// it exists so per-network subnet can be persisted ahead of the
    /// multi-segment TUN work (per-network TUN devices) that will consume it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::membership::cidr_opt"
    )]
    pub subnet: Option<crate::membership::Subnet>,
    /// This network's NUKE-CONSENSUS proposer threshold
    /// (NUKE-CONSENSUS-THRESHOLD-001), fixed at `tetron create
    /// --nuke-consensus <n>` and never mutated afterward -- same
    /// immutable-after-create treatment as `subnet`. Always persisted
    /// explicitly (not `Option`; there is no meaningful "use something else"
    /// case the way `subnet`'s `None` means "use the node-wide default").
    /// `#[serde(default = ...)]` exists only so a config predating this field
    /// decodes as the historical hardcoded value of 2.
    #[serde(default = "crate::membership::default_nuke_consensus_threshold")]
    pub nuke_consensus_threshold: u32,
}

/// In-memory aggregate of the on-disk config. Reads assemble this from
/// `settings.toml` (globals) + one `networks/<name>.toml` per network; writes
/// are targeted (`save_settings` / `save_network` / `delete_network`) so a write
/// to one network can never clobber another. See `crate::config::storage`.
/// A global server override (relay / discovery-DNS / DNS-upstreams). `servers`
/// holds a preset keyword (see `crate::config::overrides::relay_urls`/
/// `discovery_urls` for the actual keyword string) or literal URLs/IPs as the
/// user typed them; an empty list means unset (use the iroh n0 defaults). `replace` swaps
/// the defaults out instead of augmenting them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerOverride {
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default)]
    pub replace: bool,
}

impl ServerOverride {
    pub fn is_unset(&self) -> bool {
        self.servers.is_empty()
    }
}

/// Rate-limit policy overrides for `src/ratelimit.rs`'s `ControlGate`
/// (per-connection) and `GlobalRateLimiter` (daemon-wide) (HARDEN-005). Each
/// field `None` means "use the compiled default" — see `ratelimit.rs` for the
/// values. Set via `tetron config set ratelimit.<key> <value>`; an empty
/// value resets that one key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refill_per_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_capacity: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_refill_per_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_strike_limit: Option<u32>,
}

/// Proactive drop-rate monitor policy overrides (LOG-002). Each field `None`
/// means "use the compiled default" (threshold=0 = disabled). Set via `tetron
/// config set drop-monitor.<key> <value>`; an empty value resets that one key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DropMonitorConfig {
    /// Seconds per monitoring window. Default 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u64>,
    /// Drops in one window to trigger a warn. 0 = disabled (default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u64>,
    /// Minimum seconds between warns for the same reason. Default 300 (5m).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<u64>,
}

/// Per-peer path-selection flap-logging policy (PATH-DIAG-006). Each field
/// `None` means "use the compiled default". Set via `tetron config set
/// path-flap.<key> <value>`; an empty value resets that one key. Does not
/// change iroh's own path-selection behavior (out of tetron's control) --
/// only how aggressively repeated `Selected` transitions for the same peer
/// get logged at `info` vs. quieted to `debug`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathFlapConfig {
    /// `Selected` transitions for one peer within one window before further
    /// ones in that same window are quieted to `debug`. Default 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u32>,
    /// Seconds per window before the count resets and the next transition is
    /// shown at `info` again. Default 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u64>,
}

/// Per-peer reconnect-loop logging policy (LOG-005). Each field `None` means
/// "use the compiled default". Set via `tetron config set reconnect-log.<key>
/// <value>`; an empty value resets that one key. Does not change reconnect
/// *behavior* (backoff timing, retry logic) -- only how aggressively the
/// "reconnecting in ... secs=N" line for one persistently-unreachable peer
/// gets logged at `info` vs. quieted to `debug`. Same shape as
/// [`PathFlapConfig`]/`PATH-DIAG-006`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReconnectLogConfig {
    /// Reconnect attempts for one peer within one window before further ones
    /// in that same window are quieted to `debug`. Default 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u32>,
    /// Seconds per window before the count resets and the next attempt is
    /// shown at `info` again. Default 300 (5m) -- longer than path-flap's 60s
    /// default since reconnect backoff itself already spaces attempts out to
    /// 30s at steady state (`BACKOFF_MAX`), so a 60s window would barely ever
    /// debounce anything; 300s reduces steady-state "still down" noise from
    /// once per 30s to once per 5m while still periodically confirming the
    /// peer is still being retried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u64>,
}

/// Cold-peer reconnect backoff escalation (CONVERGE-011). `None` means "use
/// the compiled default". Set via `tetron config set reconnect-cold.<key>
/// <value>`; unset (or set to 0/empty) to return to the default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconnectColdConfig {
    /// Consecutive failed dial attempts against one peer before the backoff
    /// cap escalates from the warm 30s (`BACKOFF_MAX`) to the cold cap
    /// below. Default 10 (roughly 4.5 minutes of trying).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u32>,
    /// The cold backoff cap in seconds, once escalated. Default 600 (10m).
    /// A returning peer dials us from its own side immediately, so this
    /// cadence bounds outbound retry churn against long-gone peers, not
    /// reconnection latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<u64>,
}

/// Frozen-tier reconnect backoff escalation (CONVERGE-013), above cold: a
/// floor for a peer that has stayed cold for a long time (roughly a day by
/// default), not a full stop -- see `backoff_cap`'s spec docstring for why
/// a hard stop was rejected. `None` means "use the compiled default". Set
/// via `tetron config set reconnect-frozen.<key> <value>`; unset (or set
/// to 0/empty) to return to the default.
/// Status-snapshot cache overrides (STATUS-CACHE-001). Set via
/// `tetron config set status-cache.<key> <value>`; unset (or set to 0) to
/// return to the default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusCacheConfig {
    /// Minimum seconds between rebuilds of the cached per-peer connection
    /// snapshot. Default 12. This is a *floor*, not a timer: the snapshot is
    /// rebuilt lazily on read, so a daemon nobody is querying does no work at
    /// all, while any number of polling clients cost at most one rebuild per
    /// interval between them. Raising it cuts daemon work further at the cost
    /// of older path/RTT/MTU figures; the traffic and drop counters are read
    /// live on every request regardless and are never affected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconnectFrozenConfig {
    /// Consecutive failed dial attempts against one peer before the backoff
    /// cap escalates a second time, from the cold cap to the frozen cap
    /// below. Default 154 (roughly 24h of continuous retrying).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u32>,
    /// The frozen backoff cap in seconds, once escalated. Default 86400
    /// (24h). A returning peer dials us from its own side immediately, so
    /// this cadence bounds standing outbound dial churn against a
    /// long-unreachable peer, not reconnection latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Local UID authorized to control the daemon without root (Tailscale's
    /// `--operator` model). `None` means root-only for mutating commands.
    #[serde(default)]
    pub operator_uid: Option<u32>,
    /// Personal default hostname used when creating/joining a network without an
    /// explicit `--hostname`. Set via `tetron resume --hostname <name>`. `None` falls
    /// back to a random generated name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_hostname: Option<String>,
    /// The node's default overlay IPv4 subnet, cached locally as a CIDR string
    /// (e.g. "10.88.0.0/24"). The authoritative value for an established
    /// network lives in its own signed `GroupBlob` (and, locally, in that
    /// network's own `NetworkConfig.subnet`, MULTISEG-001) — since MULTISEG-004
    /// this field's only remaining job is seeding the *default* subnet for a
    /// `create`/identity built before any network exists yet, or with no
    /// explicit `--subnet` given. `None` means the default 10.88.0.0/24.
    /// Written by `create --subnet`, `join`, and `tetron config set subnet`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::membership::cidr_opt"
    )]
    pub subnet: Option<crate::membership::Subnet>,
    /// Custom iroh transport relay servers (NAT-traversal fallback).
    #[serde(default)]
    pub relay: ServerOverride,
    /// Custom iroh discovery-DNS / pkarr server (endpoint resolution + record
    /// publish). Also redirects the `dht.rs` pkarr client.
    #[serde(default)]
    pub discovery_dns: ServerOverride,
    /// Rate-limit policy overrides (HARDEN-005). See [`RateLimitConfig`].
    #[serde(default)]
    pub ratelimit: RateLimitConfig,
    /// Proactive drop-rate monitor overrides (LOG-002). See [`DropMonitorConfig`].
    #[serde(default)]
    pub drop_monitor: DropMonitorConfig,
    /// Path-selection flap-logging overrides (PATH-DIAG-006). See [`PathFlapConfig`].
    #[serde(default)]
    pub path_flap: PathFlapConfig,
    /// Reconnect-loop logging overrides (LOG-005). See [`ReconnectLogConfig`].
    #[serde(default)]
    pub reconnect_log: ReconnectLogConfig,
    /// Cold-peer reconnect backoff overrides (CONVERGE-011). See
    /// [`ReconnectColdConfig`].
    #[serde(default)]
    pub reconnect_cold: ReconnectColdConfig,
    /// Frozen-tier reconnect backoff overrides (CONVERGE-013), above cold.
    /// See [`ReconnectFrozenConfig`].
    #[serde(default)]
    pub reconnect_frozen: ReconnectFrozenConfig,
    /// Status-snapshot cache overrides (STATUS-CACHE-001). See
    /// [`StatusCacheConfig`].
    #[serde(default)]
    pub status_cache: StatusCacheConfig,
    /// Override for `membership::NUKE_PROPOSAL_TTL_SECS` (compiled default
    /// 24h). `None` uses the compiled default. Set via `tetron config set
    /// nuke-proposal-ttl <duration>` (CONFIG-AUDIT-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nuke_proposal_ttl: Option<u64>,
    /// Override for `transport::TETRON_LISTEN_PORT` (compiled default
    /// 43737). `None` uses the compiled default. Daemon-wide, not
    /// per-network — one shared iroh `Endpoint`/UDP socket serves every
    /// joined network. Set via `tetron config set listen-port <port>`
    /// (CONFIG-AUDIT-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    /// Override for the DHT/group poller's tick interval (compiled default
    /// 60s). `None` uses the compiled default. Set via `tetron config set
    /// poller-interval <seconds>` (CONFIG-AUDIT-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poller_interval: Option<u64>,
    /// Override for the daemon's rolling log retention, in days (compiled
    /// default 7). `None` uses the compiled default. Set via `tetron config
    /// set log-retention <days>` (CONFIG-AUDIT-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_retention: Option<u32>,
    /// Override for a freshly minted invite's default expiry when
    /// `--expires` is omitted (compiled default 7 days). `None` uses the
    /// compiled default; `Some(0)` makes the configured default "never
    /// expires", matching `--expires 0`/`--expires never`. Set via `tetron
    /// config set invite-default-expiry <duration>` (CONFIG-AUDIT-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_default_expiry: Option<u64>,
    /// Whether the self-capture routing mitigation (`SELFCAPTURE-ROUTE-001`)
    /// is applied at daemon startup. `None`/compiled default is `true`
    /// (enabled) -- an advanced user running their own conflicting policy
    /// routing needs an escape hatch. Set via `tetron config set
    /// selfcapture-mitigation off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selfcapture_mitigation: Option<bool>,
    /// Override for the daemon's file-log level (compiled default `info`,
    /// LOG-003). `None` uses the compiled default. One of
    /// `trace`/`debug`/`info`/`warn`/`error`. Read once at daemon startup
    /// (`init_tracing`); `RUST_LOG` still wins over this if set. Set via
    /// `tetron config set log-level <level>` (CONFIG-AUDIT-002 style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
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

/// Remove a network by name. Returns true if it was found and removed.
pub fn remove_network(config: &mut AppConfig, name: &str) -> bool {
    let before = config.networks.len();
    config.networks.retain(|n| n.name != name);
    config.networks.len() < before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        SecretKey::from(key_bytes).public()
    }

    #[test]
    fn test_serialize_roundtrip() {
        let config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "gaming".to_string(),
                    group_mode: GroupMode::Open,
                    my_ip: Some(Ipv4Addr::new(10, 88, 10, 5)),
                    members: vec![
                        MemberEntry {
                            identity: test_id(2),
                            ip: Ipv4Addr::new(10, 88, 5, 3),
                            is_coordinator: true,
                            hostname: None,
                        },
                        MemberEntry {
                            identity: test_id(3),
                            ip: Ipv4Addr::new(10, 88, 10, 5),
                            is_coordinator: false,
                            hostname: None,
                        },
                    ],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                    subnet: None,
                    nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
                },
                NetworkConfig {
                    name: "work".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                    subnet: None,
                    nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
                },
            ],
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks.len(), 2);
        assert_eq!(parsed.networks[0].name, "gaming");
        assert_eq!(parsed.networks[0].members.len(), 2);
        assert_eq!(parsed.networks[1].name, "work");
    }

    #[test]
    fn test_deserialize_empty() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert!(config.networks.is_empty());
    }

    #[test]
    fn test_upsert_new() {
        let mut config = AppConfig::default();
        let net = NetworkConfig {
            name: "test".to_string(),
            group_mode: GroupMode::Open,
            my_ip: Some(Ipv4Addr::new(10, 88, 10, 5)),
            members: vec![],
            approved: vec![],
            network_secret_key: None,
            network_public_key: None,
            my_hostname: None,
            transport: None,
            admins: vec![],
            direct: false,
            subnet: None,
            nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
        };
        upsert_network(&mut config, net);
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "test");
        assert_eq!(config.networks[0].group_mode, GroupMode::Open);
    }

    #[test]
    fn test_upsert_replaces_existing() {
        let mut config = AppConfig {
            networks: vec![NetworkConfig {
                name: "test".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: None,
                members: vec![],
                approved: vec![],
                network_secret_key: None,
                network_public_key: None,
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
                subnet: None,
                nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
            }],
            ..Default::default()
        };
        let updated = NetworkConfig {
            name: "test".to_string(),
            group_mode: GroupMode::Open,
            my_ip: Some(Ipv4Addr::new(10, 88, 10, 5)),
            members: vec![],
            approved: vec![],
            network_secret_key: None,
            network_public_key: None,
            my_hostname: None,
            transport: None,
            admins: vec![],
            direct: false,
            subnet: None,
            nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
        };
        upsert_network(&mut config, updated.clone());
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].group_mode, GroupMode::Open);
        assert_eq!(config.networks[0].my_ip, Some(Ipv4Addr::new(10, 88, 10, 5)));
    }

    #[test]
    fn test_remove_network() {
        let mut config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "keep".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                    subnet: None,
                    nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
                },
                NetworkConfig {
                    name: "remove-me".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                    subnet: None,
                    nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
                },
            ],
            ..Default::default()
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

    #[test]
    fn test_serialize_with_approved() {
        let id1 = test_id(1);
        let id2 = test_id(2);
        let config = AppConfig {
            networks: vec![NetworkConfig {
                name: "gaming".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: Some(Ipv4Addr::new(10, 88, 10, 5)),
                members: vec![MemberEntry {
                    identity: id1,
                    ip: Ipv4Addr::new(10, 88, 5, 3),
                    is_coordinator: true,
                    hostname: None,
                }],
                approved: vec![ApprovedConfigEntry {
                    identity: id2,
                    ip: Ipv4Addr::new(10, 88, 12, 34),
                    hostname: None,
                }],
                network_secret_key: None,
                network_public_key: None,
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
                subnet: None,
                nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
            }],
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks[0].approved.len(), 1);
        assert_eq!(parsed.networks[0].approved[0].identity, id2);
    }

    #[test]
    fn test_serialize_with_network_key() {
        let secret = SecretKey::generate();
        let public = secret.public();
        let config = AppConfig {
            networks: vec![NetworkConfig {
                name: "gaming".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: Some(Ipv4Addr::new(10, 88, 10, 5)),
                members: vec![],
                approved: vec![],
                network_secret_key: Some(secret.clone()),
                network_public_key: Some(public),
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
                subnet: None,
                nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
            }],
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks[0].network_public_key, Some(public));
        assert!(parsed.networks[0].network_secret_key.is_some());
    }

    #[test]
    fn test_direct_flag_default_false() {
        let toml_str = r#"
[[networks]]
name = "dario-alice"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.networks[0].direct);
    }

    #[test]
    fn test_deserialize_minimal() {
        let toml_str = r#"
[[networks]]
name = "test"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "test");
        assert_eq!(config.networks[0].group_mode, GroupMode::Restricted);
        assert!(config.networks[0].members.is_empty());
        assert!(config.networks[0].approved.is_empty());
        assert!(config.networks[0].network_secret_key.is_none());
        assert!(config.networks[0].network_public_key.is_none());
    }
}
