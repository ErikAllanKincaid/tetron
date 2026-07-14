//! Invite-key handlers for `MeshManager`: `invite_create` / `invite_list` /
//! `invite_revoke`. Split out of `daemon/mod.rs`.

use super::super::*;

impl MeshManager {
    /// Coordinator-only: mint a single-use invite key for `network`.
    ///
    /// `expires` is an optional human-readable duration ("24h", "7d", "30m").
    /// If absent the invite never expires.
    pub(crate) fn invite_create(
        &self,
        network: &str,
        expires: Option<&str>,
    ) -> IpcMessage {
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let (invite_store, net_pubkey) = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let s = handle.state.read().unwrap();
            if s.invite_store.is_none() {
                return IpcMessage::Error {
                    message: format!("invite store not available for '{network}'"),
                };
            }
            (s.invite_store.clone().unwrap(), handle.network_key)
        };

        let ttl_secs = match expires {
            Some(dur) => match parse_duration(dur) {
                Ok(secs) => Some(secs),
                Err(e) => {
                    return IpcMessage::Error {
                        message: format!("invalid duration '{dur}': {e}"),
                    };
                }
            },
            None => None,
        };

        let (id, secret) = match invite_store.create(ttl_secs) {
            Ok(pair) => pair,
            Err(e) => {
                return IpcMessage::Error {
                    message: format!("failed to create invite: {e}"),
                };
            }
        };

        let invite_key = crate::invite::encode_invite_code(
            &net_pubkey,
            &self.endpoint.id(),
            &secret,
        );

        let expires_at = ttl_secs.map(|ttl| crate::daemon::mesh::reconverge::now_secs() + ttl);

        IpcMessage::InviteCreated {
            invite_key,
            invite_id: id,
            expires_at,
        }
    }

    /// List outstanding invites for `network` (coordinator-only).
    pub(crate) fn invite_list(&self, network: &str) -> IpcMessage {
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let invite_store = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let s = handle.state.read().unwrap();
            match s.invite_store.as_ref() {
                Some(store) => store.clone(),
                None => {
                    return IpcMessage::InviteListResponse {
                        invites: vec![],
                    };
                }
            }
        };

        let invites = match invite_store.list() {
            Ok(list) => list
                .into_iter()
                .map(|s| ipc::InviteInfo {
                    id: s.id,
                    created_at: s.created_at,
                    expires_at: s.expires_at,
                    used: s.used,
                })
                .collect(),
            Err(e) => {
                return IpcMessage::Error {
                    message: format!("failed to list invites: {e}"),
                };
            }
        };

        IpcMessage::InviteListResponse { invites }
    }

    /// Coordinator-only: revoke (mark as used) an invite by its short id.
    pub(crate) fn invite_revoke(&self, network: &str, invite_id: &str) -> IpcMessage {
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let invite_store = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let s = handle.state.read().unwrap();
            match s.invite_store.as_ref() {
                Some(store) => store.clone(),
                None => {
                    return IpcMessage::Error {
                        message: format!("invite store not available for '{network}'"),
                    };
                }
            }
        };

        match invite_store.revoke(invite_id) {
            Ok(()) => IpcMessage::Ok {
                message: format!("invite '{invite_id}' revoked"),
            },
            Err(e) => IpcMessage::Error {
                message: format!("failed to revoke invite '{invite_id}': {e}"),
            },
        }
    }
}

/// Parse a human-readable duration string into seconds.
///
/// Supports suffixes: `s` (seconds), `m` (minutes), `h` (hours), `d` (days),
/// `w` (weeks). Returns an error if the string is malformed or the value
/// overflows `u64`.
fn parse_duration(s: &str) -> std::result::Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, suffix) = if s.ends_with(|c: char| c.is_ascii_alphabetic()) {
        let split = s.len() - 1;
        (&s[..split], &s[split..])
    } else {
        (s, "s") // bare number = seconds
    };
    let value: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number '{num_str}'"))?;
    let multiplier = match suffix {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err(format!("unknown suffix '{suffix}', use s/m/h/d/w")),
    };
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "duration overflows u64".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("30").unwrap(), 30);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), 300);
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), 7200);
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("7d").unwrap(), 604800);
    }

    #[test]
    fn test_parse_duration_weeks() {
        assert_eq!(parse_duration("2w").unwrap(), 1209600);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_duration_overflow() {
        // u64::MAX / 604800 < value, would overflow with multiplier 604800 (weeks).
        let big = format!("{}w", u64::MAX);
        assert!(parse_duration(&big).is_err());
    }
}
