//! DHT-based membership publishing and resolution.
//!
//! Encodes network membership as signed pkarr DNS TXT records and publishes them
//! to the iroh pkarr relay so peers can discover each other without the coordinator
//! being online.
//!
//! # Record format
//!
//! TXT records are stored under the `_pitopi` DNS name:
//!
//! ```text
//! "v1"                             // version sentinel (always first)
//! "c,<hex_identity>"               // coordinator member
//! "m,<hex_identity>"               // regular member
//! "a,<hex_identity>"               // approved (not yet connected)
//! ```
//!
//! IPs are not stored — they are reconstructed on decode via [`derive_ip`].

use anyhow::{Context as _, Result, bail, ensure};
use iroh::{
    EndpointId, SecretKey,
    address_lookup::PkarrRelayClient,
    dns::DnsResolver,
    endpoint::Endpoint,
};
use iroh_dns::pkarr::SignedPacket;
use url::Url;


// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const RECORD_NAME: &str = "_pitopi";
const RECORD_VERSION: &str = "v1";
const RECORD_TTL: u32 = 300;
/// The production pkarr relay run by number 0.
const PKARR_RELAY_URL: &str = "https://dns.iroh.link/pkarr";

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Derives a deterministic `SecretKey` for this network's DHT membership record.
///
/// The coordinator publishes membership under this key so that peers can find it
/// using only the coordinator's public key and the network name.
pub fn derive_membership_key(coordinator_key: &SecretKey, network_name: &str) -> SecretKey {
    let context = format!("pitopi/membership/{network_name}");
    let derived = blake3::derive_key(&context, &coordinator_key.to_bytes());
    SecretKey::from_bytes(&derived)
}

/// Returns the `EndpointId` (public key) under which membership is published on the DHT.
pub fn membership_dht_id(coordinator_key: &SecretKey, network_name: &str) -> EndpointId {
    derive_membership_key(coordinator_key, network_name).public()
}

// ---------------------------------------------------------------------------
// Record encoding / decoding
// ---------------------------------------------------------------------------

/// Encodes a membership hash into a signed pkarr packet.
///
/// The record contains only the version tag and a blake3 hash pointer.
/// Peers request the full membership data from any online peer using
/// this hash.
pub fn encode_membership_record(
    key: &SecretKey,
    hash: &str,
) -> Result<SignedPacket> {
    let values = vec![RECORD_VERSION.to_string(), format!("h,{hash}")];
    SignedPacket::from_txt_strings(key, RECORD_NAME, values, RECORD_TTL)
        .map_err(|e| anyhow::anyhow!("failed to build signed packet: {e}"))
}

/// Decodes a signed pkarr packet, extracting the membership hash.
///
/// Accepts both hash-only records (`h,<blake3>`) and legacy member records
/// (`c,`/`m,`/`a,` entries), skipping the latter for forward compatibility.
pub fn decode_membership_record(
    packet: &SignedPacket,
) -> Result<String> {
    let records = packet.txt_records(RECORD_NAME);
    ensure!(!records.is_empty(), "no membership records found");
    ensure!(
        records[0] == RECORD_VERSION,
        "unsupported record version: {}",
        records[0]
    );

    for record in &records[1..] {
        if let Some(hash) = record.strip_prefix("h,") {
            return Ok(hash.to_string());
        }
    }

    bail!("no membership hash found in record")
}

// ---------------------------------------------------------------------------
// Pkarr client
// ---------------------------------------------------------------------------

/// Creates a [`PkarrRelayClient`] using the endpoint's TLS and DNS configuration.
pub fn create_pkarr_client(ep: &Endpoint) -> Result<PkarrRelayClient> {
    let tls_config = ep.tls_config().clone();
    let dns_resolver: DnsResolver = ep
        .dns_resolver()
        .context("endpoint has no DNS resolver")?
        .clone();
    let relay_url: Url = PKARR_RELAY_URL.parse().expect("relay URL is valid");
    Ok(PkarrRelayClient::new(relay_url, tls_config, dns_resolver))
}

// ---------------------------------------------------------------------------
// Publish / resolve
// ---------------------------------------------------------------------------

/// Publishes a membership hash to the pkarr relay.
pub async fn publish_membership(
    client: &PkarrRelayClient,
    key: &SecretKey,
    hash: &str,
) -> Result<()> {
    let packet = encode_membership_record(key, hash)?;
    client
        .publish(&packet)
        .await
        .map_err(|e| anyhow::anyhow!("failed to publish membership: {e}"))
}

/// Resolves the membership hash from the pkarr relay.
pub async fn resolve_membership_hash(
    client: &PkarrRelayClient,
    dht_id: EndpointId,
) -> Result<String> {
    let packet = client
        .resolve(dht_id)
        .await
        .map_err(|e| anyhow::anyhow!("failed to resolve membership: {e}"))?;
    decode_membership_record(&packet)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    // -- Key derivation -------------------------------------------------------

    #[test]
    fn test_derive_membership_key_deterministic() {
        let key = SecretKey::generate();
        let k1 = derive_membership_key(&key, "gaming");
        let k2 = derive_membership_key(&key, "gaming");
        assert_eq!(k1.public(), k2.public());
    }

    #[test]
    fn test_derive_membership_key_differs_by_network() {
        let key = SecretKey::generate();
        let k1 = derive_membership_key(&key, "gaming");
        let k2 = derive_membership_key(&key, "work");
        assert_ne!(k1.public(), k2.public());
    }

    #[test]
    fn test_membership_dht_id() {
        let key = SecretKey::generate();
        let dht_id = membership_dht_id(&key, "gaming");
        let derived = derive_membership_key(&key, "gaming");
        assert_eq!(dht_id, derived.public());
    }

    #[test]
    fn test_derive_membership_key_differs_from_source() {
        let key = SecretKey::generate();
        let derived = derive_membership_key(&key, "gaming");
        assert_ne!(key.public(), derived.public());
    }

    // -- Hash encode / decode roundtrip ---------------------------------------

    #[test]
    fn test_encode_decode_hash_roundtrip() {
        let key = SecretKey::generate();
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let packet = encode_membership_record(&key, hash).unwrap();
        let decoded = decode_membership_record(&packet).unwrap();
        assert_eq!(decoded, hash);
    }

    #[test]
    fn test_record_version_check() {
        let key = SecretKey::generate();
        let packet = encode_membership_record(&key, "somehash").unwrap();
        let records = packet.txt_records("_pitopi");
        assert_eq!(records[0], "v1");
    }

    #[test]
    fn test_decode_skips_legacy_entries() {
        let key = SecretKey::generate();
        let values = vec!["v1", "c,legacy_id", "h,the_real_hash", "m,another_legacy"];
        let packet = SignedPacket::from_txt_strings(&key, "_pitopi", values, 300).unwrap();
        let hash = decode_membership_record(&packet).unwrap();
        assert_eq!(hash, "the_real_hash");
    }

    #[test]
    fn test_decode_rejects_unknown_version() {
        let key = SecretKey::generate();
        let values = vec!["v99".to_string()];
        let packet = SignedPacket::from_txt_strings(&key, "_pitopi", values, 300).unwrap();
        let result = decode_membership_record(&packet);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported record version"));
    }

    #[test]
    fn test_decode_rejects_empty_packet() {
        let key = SecretKey::generate();
        let values = vec!["v1".to_string()];
        let packet = SignedPacket::from_txt_strings(&key, "_other", values, 300).unwrap();
        let result = decode_membership_record(&packet);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no membership records found"));
    }

    #[test]
    fn test_decode_rejects_missing_hash() {
        let key = SecretKey::generate();
        let values = vec!["v1", "c,some_identity"];
        let packet = SignedPacket::from_txt_strings(&key, "_pitopi", values, 300).unwrap();
        let result = decode_membership_record(&packet);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no membership hash"));
    }
}
