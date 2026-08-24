//! Invite-code codec (INVITE-010, moved from `tetron`'s own `src/invite.rs`).
//!
//! An invite _code_ is `bs58(network_pubkey(32) || secret(16) ||
//! blake3(payload)[..4])` — 52 bytes (INVITE-CHECKSUM-001). The trailing
//! 4-byte blake3 checksum catches dropped/garbled base58 characters that would
//! otherwise decode to a "well-formed" invite for a network that doesn't
//! exist. The decoder also accepts the legacy 48-byte unchecksummed form so
//! codes handed out before the checksum landed keep working.
//!
//! Moved here (rather than staying in the `tetron` binary/lib crate) so
//! GUI frontends can call the real implementation instead of hand-copying it
//! — `tetron-webui`'s own code comment on its copy: "that function lives in
//! a binary crate not meant to be imported". `tetron::invite` keeps a `pub
//! use` re-export of everything below so `tetron-mobile` (which embeds the
//! full `tetron` crate directly) needs no changes.
//!
//! `ReusableKey`/`InviteEntry` (the record *types* minted from these codes)
//! stay in `tetron`'s own `src/invite.rs` — only the stateless codec moves.

use anyhow::{Result, bail};
use iroh::EndpointId;

/// Length of the random invite secret, in bytes (128 bits). Duplicated from
/// `tetron::invite::SECRET_LEN` rather than imported — this crate cannot
/// depend on `tetron` (dependency direction is the other way around), and
/// this is a wire-format constant intrinsic to the codec itself, not a
/// borrowed value that could drift independently of the format it's part of.
const SECRET_LEN: usize = 16;

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
pub fn encode_invite_code(network_pubkey: &EndpointId, secret: &[u8]) -> String {
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
        assert!(is_bare_room_id(
            &bs58::encode(test_id(5).as_bytes()).into_string()
        ));
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
