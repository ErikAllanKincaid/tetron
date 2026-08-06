//! Invite-code encoding (joiner side).
//!
//! An invite _code_ is `bs58(network_pubkey(32) || secret(16) ||
//! blake3(payload)[..4])` — 52 bytes (INVITE-CHECKSUM-001). The trailing
//! 4-byte blake3 checksum catches dropped/garbled base58 characters that would
//! otherwise decode to a "well-formed" invite for a network that doesn't
//! exist. The decoder also accepts the legacy 48-byte unchecksummed form so
//! codes handed out before the checksum landed keep working.
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

/// Length of the blake3 integrity checksum appended to an invite payload
/// (INVITE-CHECKSUM-001). 4 bytes: a corruption strong enough to matter is
/// detected with overwhelming probability, and the code stays short.
const CHECKSUM_LEN: usize = 4;

/// Payload length of an invite code before the checksum.
const PAYLOAD_LEN: usize = 32 + SECRET_LEN;

/// Total raw length of a checksummed invite code (payload + checksum).
const ENCODED_LEN: usize = PAYLOAD_LEN + CHECKSUM_LEN;

/// Encode an invite code: `bs58(network_pubkey(32) || secret(16) ||
/// blake3(payload)[..4])`.
pub fn encode_invite_code(
    network_pubkey: &EndpointId,
    secret: &[u8],
) -> String {
    let mut bytes = Vec::with_capacity(PAYLOAD_LEN + CHECKSUM_LEN);
    bytes.extend_from_slice(network_pubkey.as_bytes());
    bytes.extend_from_slice(secret);
    bytes.extend_from_slice(&blake3::hash(&bytes).as_bytes()[..CHECKSUM_LEN]);
    bs58::encode(&bytes).into_string()
}

/// True when `s` is a bare room id: a base58 string that decodes to exactly
/// the 32-byte network public key. Used by the CLI (INVITE-CHECKSUM-001) to
/// tell a *corrupted invite code* (a failed `decode_invite_code` that proves
/// the input was meant to be an invite — e.g. a 52-byte checksum mismatch)
/// apart from a *bare room id*, which must keep flowing to the daemon so it
/// can deny it with the invite-required message.
pub fn is_bare_room_id(s: &str) -> bool {
    matches!(bs58::decode(s).into_vec(), Ok(v) if v.len() == 32)
}

/// Decode an invite code into `(network_pubkey, secret)`.
///
/// Accepts both the checksummed 52-byte form (verifies the trailing 4-byte
/// blake3 checksum, rejecting on mismatch) and the legacy 48-byte
/// unchecksummed form (INVITE-CHECKSUM-001).
pub fn decode_invite_code(code: &str) -> Result<(EndpointId, Vec<u8>)> {
    let bytes = bs58::decode(code)
        .into_vec()
        .map_err(|e| anyhow::anyhow!("invalid invite code: {e}"))?;
    let payload = match bytes.len() {
        // Checksummed form: 48-byte payload + 4-byte checksum.
        ENCODED_LEN => {
            let (payload, csum) = bytes.split_at(PAYLOAD_LEN);
            if csum != &blake3::hash(payload).as_bytes()[..CHECKSUM_LEN] {
                bail!("invalid invite code: checksum mismatch (corrupted or mistyped)");
            }
            payload
        }
        // Legacy unchecksummed form: 48-byte payload only.
        PAYLOAD_LEN => &bytes[..],
        other => {
            bail!(
                "invalid invite code: expected {} or {} bytes, got {other}",
                ENCODED_LEN,
                PAYLOAD_LEN,
            );
        }
    };
    let net: [u8; 32] = payload[0..32].try_into().unwrap();
    let secret = payload[32..].to_vec();
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
    fn reusable_key_from_secret_sets_id_and_expiry() {
        let secret = [5u8; 16];
        let (hash, key) = ReusableKey::from_secret(&secret, 100, 50);
        assert_eq!(hash, blake3::hash(&secret).to_hex().to_string());
        assert_eq!(key.id, hash[..8]);
        assert_eq!(key.created, 100);
        assert_eq!(key.expires, 150);
        assert!(!key.revoked);
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

    #[test]
    fn encoded_code_carries_checksum() {
        // INVITE-CHECKSUM-001: a freshly encoded invite is 52 raw bytes
        // (48-byte payload + 4-byte blake3 checksum), and the checksum
        // validates against the payload.
        let net = test_id(2);
        let secret: [u8; SECRET_LEN] = rand::random();
        let code = encode_invite_code(&net, &secret);
        let bytes = bs58::decode(&code).into_vec().unwrap();
        assert_eq!(bytes.len(), 32 + SECRET_LEN + 4);
        let (payload, csum) = bytes.split_at(32 + SECRET_LEN);
        assert_eq!(csum, &blake3::hash(payload).as_bytes()[..4]);
    }

    #[test]
    fn decode_accepts_legacy_unchecksummed() {
        // INVITE-CHECKSUM-001: legacy 48-byte codes (no checksum) still
        // decode — codes handed out before this change keep working.
        let net = test_id(3);
        let secret: [u8; SECRET_LEN] = rand::random();
        let mut payload = Vec::with_capacity(32 + SECRET_LEN);
        payload.extend_from_slice(net.as_bytes());
        payload.extend_from_slice(&secret);
        let legacy = bs58::encode(&payload).into_string();
        let (dn, ds) = decode_invite_code(&legacy).unwrap();
        assert_eq!(dn, net);
        assert_eq!(ds, secret.to_vec());
    }

    #[test]
    fn decode_rejects_checksum_mismatch() {
        // INVITE-CHECKSUM-001: a code whose payload was altered without
        // updating the checksum is rejected as invalid, not silently
        // accepted as a "well-formed" invite for a nonexistent network.
        let net = test_id(4);
        let secret: [u8; SECRET_LEN] = rand::random();
        let code = encode_invite_code(&net, &secret);
        let mut bytes = bs58::decode(&code).into_vec().unwrap();
        // Corrupt one payload byte but leave the checksum alone.
        bytes[0] ^= 0x01;
        let tampered = bs58::encode(&bytes).into_string();
        assert!(decode_invite_code(&tampered).is_err());
    }

    #[test]
    fn bare_room_id_detection() {
        // INVITE-CHECKSUM-001 CLI discrimination: a base58 string that
        // decodes to 32 bytes (a bare network pubkey) is a room id, not an
        // invite — so the CLI lets it flow to the daemon for denial.
        assert!(is_bare_room_id(&bs58::encode(test_id(5).as_bytes()).into_string()));
        // An encoded invite (48-byte legacy or 52-byte checksummed) is not.
        let secret: [u8; SECRET_LEN] = rand::random();
        assert!(!is_bare_room_id(&encode_invite_code(&test_id(6), &secret)));
        // Neither is a tampered one.
        let code = encode_invite_code(&test_id(6), &secret);
        let mut bytes = bs58::decode(&code).into_vec().unwrap();
        bytes[0] ^= 0x01;
        let tampered = bs58::encode(&bytes).into_string();
        assert!(!is_bare_room_id(&tampered));
        // Garbage that isn't base58 at all is not a room id either.
        assert!(!is_bare_room_id("not base58!!"));
    }
}
