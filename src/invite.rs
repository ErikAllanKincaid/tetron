//! Invite-code encoding (joiner side).
//!
//! tetron does not mint invites (approval-only admission, MINIMAL-013). A joiner
//! can still *redeem* an invite minted by a full-torpedo coordinator: the invite
//! *code* is `bs58(network_pubkey || coordinator || secret)` (see
//! [`encode_invite_code`]). `torpedo join <code>` decodes it, dials the pinned
//! coordinator directly, and presents the secret in its `JoinRequest`.

use anyhow::{Result, bail};
use iroh::EndpointId;

/// Length of the random invite secret, in bytes (128 bits).
pub const SECRET_LEN: usize = 16;

/// Encode an invite code: `bs58(network_pubkey(32) || coordinator(32) || secret(16))`.
pub fn encode_invite_code(
    network_pubkey: &EndpointId,
    coordinator: &EndpointId,
    secret: &[u8],
) -> String {
    let mut bytes = Vec::with_capacity(32 + 32 + SECRET_LEN);
    bytes.extend_from_slice(network_pubkey.as_bytes());
    bytes.extend_from_slice(coordinator.as_bytes());
    bytes.extend_from_slice(secret);
    bs58::encode(&bytes).into_string()
}

/// Decode an invite code into `(network_pubkey, coordinator, secret)`.
pub fn decode_invite_code(code: &str) -> Result<(EndpointId, EndpointId, Vec<u8>)> {
    let bytes = bs58::decode(code)
        .into_vec()
        .map_err(|e| anyhow::anyhow!("invalid invite code: {e}"))?;
    if bytes.len() != 32 + 32 + SECRET_LEN {
        bail!(
            "invalid invite code: expected {} bytes, got {}",
            32 + 32 + SECRET_LEN,
            bytes.len()
        );
    }
    let net: [u8; 32] = bytes[0..32].try_into().unwrap();
    let coord: [u8; 32] = bytes[32..64].try_into().unwrap();
    let secret = bytes[64..].to_vec();
    let network_pubkey = EndpointId::from_bytes(&net)
        .map_err(|e| anyhow::anyhow!("invalid network key in invite: {e}"))?;
    let coordinator = EndpointId::from_bytes(&coord)
        .map_err(|e| anyhow::anyhow!("invalid coordinator key in invite: {e}"))?;
    Ok((network_pubkey, coordinator, secret))
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
        let coord = test_id(2);
        let secret: [u8; SECRET_LEN] = rand::random();
        let code = encode_invite_code(&net, &coord, &secret);
        let (dn, dc, ds) = decode_invite_code(&code).unwrap();
        assert_eq!(dn, net);
        assert_eq!(dc, coord);
        assert_eq!(ds, secret.to_vec());
    }

    #[test]
    fn decode_rejects_bad_length() {
        // A 32-byte bs58 string (a bare room id) is not a valid invite.
        let code = bs58::encode(test_id(1).as_bytes()).into_string();
        assert!(decode_invite_code(&code).is_err());
    }
}
