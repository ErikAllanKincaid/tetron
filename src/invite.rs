//! Invite record types (joiner side). The codec functions that used to live
//! here (`encode_invite_code`/`decode_invite_code`/`is_bare_room_id`) moved
//! to `tetron-proto` (INVITE-010) so GUI frontends can call the real
//! implementation instead of hand-copying it; re-exported below so every
//! existing `crate::invite::…` path keeps compiling unchanged.
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

use serde::{Deserialize, Serialize};

pub use tetron_proto::invite::{decode_invite_code, encode_invite_code, is_bare_room_id};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
