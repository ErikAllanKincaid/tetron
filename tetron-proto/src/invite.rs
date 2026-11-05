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

/// Longest base58 text an invite code can have (JOIN-INPUT-001): the base58
/// length of an all-0xFF `ENCODED_LEN` payload, `ceil(ENCODED_LEN * 8 /
/// log2(58))`, in integer form (`log2(58)` ~ 5.858, `8 / 5.858` ~ 1.3657).
/// 72 for today's 52-byte code. Callers that read an invite from a stream
/// size their read with it instead of picking a number of their own.
pub const INVITE_CODE_MAX_CHARS: usize = (ENCODED_LEN * 1366).div_ceil(1000);

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

/// JOIN-INPUT-001: read one invite code from `reader`, sized by the codec
/// (`INVITE_CODE_MAX_CHARS`). Surrounding whitespace and line endings are
/// trimmed; exactly one non-empty line is accepted. Errors name `source`
/// (`stdin` or a path), never the input, so no invite text reaches a
/// terminal log, journal or answers file.
pub fn read_invite(reader: impl std::io::Read, source: &str) -> Result<String> {
    use std::io::Read;
    let cap = INVITE_CODE_MAX_CHARS + 64;
    let mut buf = Vec::new();
    reader
        .take(cap as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| anyhow::anyhow!("cannot read invite from {source}: {e}"))?;
    anyhow::ensure!(
        buf.len() <= cap,
        "{source}: input too large to be an invite"
    );
    let text = std::str::from_utf8(&buf)
        .map_err(|_| anyhow::anyhow!("{source}: input is not text, not an invite"))?;
    let mut lines = text
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let code = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("{source}: no invite found (empty input)"))?;
    anyhow::ensure!(
        lines.next().is_none(),
        "{source}: expected exactly one invite, found more than one line"
    );
    anyhow::ensure!(
        code.len() <= INVITE_CODE_MAX_CHARS,
        "{source}: line too long to be an invite"
    );
    Ok(code.to_string())
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
    fn max_chars_matches_largest_encoded_code() {
        // JOIN-INPUT-001: the longest possible base58 text of an ENCODED_LEN
        // payload (all 0xFF bytes) is exactly INVITE_CODE_MAX_CHARS.
        let longest = bs58::encode([0xFFu8; ENCODED_LEN]).into_string();
        assert_eq!(longest.len(), INVITE_CODE_MAX_CHARS);
        assert_eq!(INVITE_CODE_MAX_CHARS, 72);
    }

    #[test]
    fn real_codes_fit_max_chars() {
        for seed in 0..=255u8 {
            let code = encode_invite_code(&test_id(seed), &[0xFFu8; SECRET_LEN]);
            assert!(code.len() <= INVITE_CODE_MAX_CHARS);
        }
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

    // JOIN-INPUT-001: read_invite over byte slices.
    fn sample_code() -> String {
        encode_invite_code(&test_id(9), &[7u8; 16])
    }

    #[test]
    fn read_invite_accepts_code_with_trailing_newline() {
        let code = sample_code();
        let got = read_invite(format!("{code}\n").as_bytes(), "stdin").unwrap();
        assert_eq!(got, code);
    }

    #[test]
    fn read_invite_accepts_crlf_and_surrounding_whitespace() {
        let code = sample_code();
        let got = read_invite(format!("  \r\n{code}\r\n\n").as_bytes(), "stdin").unwrap();
        assert_eq!(got, code);
    }

    #[test]
    fn read_invite_rejects_empty_and_whitespace_only() {
        assert!(read_invite(&b""[..], "stdin").is_err());
        assert!(read_invite(&b" \n\t\r\n"[..], "stdin").is_err());
    }

    #[test]
    fn read_invite_rejects_two_lines() {
        let code = sample_code();
        let err = read_invite(format!("{code}\n{code}\n").as_bytes(), "stdin").unwrap_err();
        assert!(!err.to_string().contains(&code));
    }

    #[test]
    fn read_invite_rejects_input_over_cap() {
        let over = [b'a'; INVITE_CODE_MAX_CHARS + 64 + 1];
        assert!(read_invite(&over[..], "stdin").is_err());
        // A line longer than one invite fails even within the read cap.
        let long = "a".repeat(INVITE_CODE_MAX_CHARS + 1);
        assert!(read_invite(long.as_bytes(), "stdin").is_err());
    }

    #[test]
    fn read_invite_errors_name_source_not_content() {
        let long = "a".repeat(INVITE_CODE_MAX_CHARS + 1);
        let err = read_invite(long.as_bytes(), "/etc/tetron/pending-invite")
            .unwrap_err()
            .to_string();
        assert!(err.contains("/etc/tetron/pending-invite"));
        assert!(!err.contains(&long));
    }
}
