//! Invite-code encoding (joiner side).
//!
//! An invite _code_ is `bs58(network_pubkey(32) || secret(16))` — 48 bytes.
//! `tetron join <code>` decodes it, resolves the network's blob (which carries
//! the invite entry), and dials any coordinator to present the secret.
//! Pinning a specific coordinator in the code is no longer needed because every
//! network-key holder validates from the signed blob (BLOB-001).

use anyhow::{Result, bail};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};

/// Length of the random invite secret, in bytes (128 bits).
pub const SECRET_LEN: usize = 16;

/// A reusable, expiring join key (Tailscale auth-key analog). Only the
/// `blake3(secret)` hash is published — the raw secret lives solely in the code
/// handed to a joiner. Because it rides the signed `GroupBlob`, *any* network-key
/// holder can verify-and-admit and revocation propagates to every admin.
///
/// Moved from `membership.rs` (MODULARIZE-001); `crate::membership::…`
/// paths keep working via re-export. The map-level `revoke_reusable`/
/// `validate_reusable_key` functions stay in `membership.rs` — they operate
/// on the whole `GroupBlob.reusable_keys` map, not just this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReusableKey {
    /// Short human id: the first 8 hex chars of the secret hash.
    pub id: String,
    /// Unix seconds when minted.
    pub created: u64,
    /// Unix seconds after which the key is no longer redeemable.
    pub expires: u64,
    /// Set by `tetron invite revoke`; a revoked key admits no one.
    pub revoked: bool,
}

impl ReusableKey {
    /// Build a reusable key from a freshly generated secret. Returns the map key
    /// (hex `blake3(secret)`) and the entry. `created`/`ttl_secs` are Unix seconds;
    /// the raw secret is the caller's to encode into the join code and discard.
    pub fn from_secret(secret: &[u8], created: u64, ttl_secs: u64) -> (String, ReusableKey) {
        let hash = blake3::hash(secret).to_hex().to_string();
        let id = hash[..8].to_string();
        (
            hash,
            ReusableKey {
                id,
                created,
                expires: created.saturating_add(ttl_secs),
                revoked: false,
            },
        )
    }
}

/// A single-use invite entry carried in the signed `GroupBlob`.
///
/// Keyed in the blob by hex `blake3(secret)`, the same hash convention
/// [`ReusableKey`] uses. An `InviteEntry` is minted by any network-key holder
/// and validated by any network-key holder — no machine-local store required.
/// Once redeemed the entry is removed from the blob (the hash changes, the
/// updated blob propagates to every coordinator).
///
/// Moved from `membership.rs` (MODULARIZE-001); see [`ReusableKey`]'s note
/// on why the map-level `revoke_invite`/`validate_invite` functions did not
/// move with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteEntry {
    /// Short human id: the first 8 hex chars of the secret hash.
    pub id: String,
    /// Unix seconds when minted.
    pub created: u64,
    /// Unix seconds after which the invite is no longer redeemable.
    /// `0` means permanent (never expires).
    pub expires: u64,
    /// Set by `tetron invite revoke`; a revoked invite admits no one.
    pub revoked: bool,
}

impl InviteEntry {
    /// Build an invite entry from a freshly generated secret. Returns the map key
    /// (hex `blake3(secret)`) and the entry. `created`/`ttl_secs` are Unix seconds;
    /// `ttl_secs=0` means permanent (never expires). The raw secret is the caller's
    /// to encode into the invite code and discard.
    pub fn from_secret(secret: &[u8], created: u64, ttl_secs: u64) -> (String, InviteEntry) {
        let hash = blake3::hash(secret).to_hex().to_string();
        let id = hash[..8].to_string();
        (
            hash,
            InviteEntry {
                id,
                created,
                expires: if ttl_secs == 0 {
                    0
                } else {
                    created.saturating_add(ttl_secs)
                },
                revoked: false,
            },
        )
    }
}

/// Encode an invite code: `bs58(network_pubkey(32) || secret(16))`.
pub fn encode_invite_code(
    network_pubkey: &EndpointId,
    secret: &[u8],
) -> String {
    let mut bytes = Vec::with_capacity(32 + SECRET_LEN);
    bytes.extend_from_slice(network_pubkey.as_bytes());
    bytes.extend_from_slice(secret);
    bs58::encode(&bytes).into_string()
}

/// Decode an invite code into `(network_pubkey, secret)`.
pub fn decode_invite_code(code: &str) -> Result<(EndpointId, Vec<u8>)> {
    let bytes = bs58::decode(code)
        .into_vec()
        .map_err(|e| anyhow::anyhow!("invalid invite code: {e}"))?;
    if bytes.len() != 32 + SECRET_LEN {
        bail!(
            "invalid invite code: expected {} bytes, got {}",
            32 + SECRET_LEN,
            bytes.len()
        );
    }
    let net: [u8; 32] = bytes[0..32].try_into().unwrap();
    let secret = bytes[32..].to_vec();
    let network_pubkey = EndpointId::from_bytes(&net)
        .map_err(|e| anyhow::anyhow!("invalid network key in invite: {e}"))?;
    Ok((network_pubkey, secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        iroh::SecretKey::from(key_bytes).public()
    }

    #[test]
    fn code_roundtrip() {
        let net = test_id(1);
        let secret: [u8; SECRET_LEN] = rand::random();
        let code = encode_invite_code(&net, &secret);
        let (dn, ds) = decode_invite_code(&code).unwrap();
        assert_eq!(dn, net);
        assert_eq!(ds, secret.to_vec());
    }

    #[test]
    fn decode_rejects_bad_length() {
        // A 32-byte bs58 string (a bare room id) is not a valid invite.
        let code = bs58::encode(test_id(1).as_bytes()).into_string();
        assert!(decode_invite_code(&code).is_err());
    }
}
