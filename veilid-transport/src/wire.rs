//! Wire types and request builders for `tetron-veilid`'s `client_api`
//! protocol: newline-delimited JSON over TCP, `{"id":u32,"op":...}`
//! requests, `{"type":"Response"|"Update",...}` replies/pushes. Confirmed
//! live 2026-09-14 against a real `tetron-veilid` instance -- see
//! `spec/core.py`'s `VeilidExternalDaemonProtocol` (VEILID-017).
//!
//! Outbound requests are built directly with `serde_json::json!` rather
//! than a derived `Serialize` enum -- this side is write-only, so there is
//! no deserialize-side edge case worth modeling a type for. Incoming lines
//! are deserialized permissively: unknown top-level fields are ignored (no
//! `deny_unknown_fields`), and an `Update` whose `kind` isn't recognized is
//! logged and skipped by the caller rather than failing to parse, so a
//! future `tetron-veilid` update kind doesn't break this client.

use serde::Deserialize;
use serde_json::Value;

/// base64url, no padding -- veilid-core's own `as_human_base64` wire
/// convention for byte payloads (confirmed live: real captured
/// `AppMessage` payloads use this alphabet).
///
/// Unused until `req_app_message` (VEILID-018 wires up `poll_send`) --
/// `#[allow(dead_code)]` rather than deleting, since it lands correctly
/// staged here and is exercised by this module's own round-trip test.
#[allow(dead_code)]
pub(crate) fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn b64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)
}

/// One line received from `tetron-veilid`: either a reply to a request
/// this client sent, or an unsolicited push.
///
/// `Update`'s payload is only pattern-matched with a wildcard until
/// VEILID-018 wires `poll_recv` up to actually read it -- `allow` rather
/// than delete, it lands correctly staged here.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Incoming {
    Response(Response),
    Update(Update),
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireError {
    pub message: String,
}

/// A reply to a request. Only `id`/`value`/`error` are modeled -- `op`/
/// `rc_id`/`rc_op` are never inspected, since a caller already knows what
/// it asked for by the `id` it sent.
#[derive(Debug, Deserialize)]
pub(crate) struct Response {
    pub id: u32,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub error: Option<WireError>,
}

/// An unsolicited push. Only `kind == "AppMessage"` is ever acted on.
/// `route_id` is intentionally not modeled -- this transport addresses by
/// `NodeId` only (VEILID-001), same as the embedded design before it.
///
/// Fields unused outside this module's own tests until VEILID-018 wires
/// `poll_recv` up to actually consume them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct Update {
    pub kind: String,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

pub(crate) fn req_new_routing_context(id: u32) -> Value {
    serde_json::json!({"id": id, "op": "NewRoutingContext"})
}

/// `SafetySelection::Unsafe(Sequencing::PreferUnordered)` -- the safety
/// selection VEILID-001/007's UPDATE paragraphs already decided, reasserted
/// here, not revisited.
pub(crate) fn req_with_safety_unsafe(id: u32, rc_id: u32) -> Value {
    serde_json::json!({
        "id": id,
        "op": "RoutingContext",
        "rc_id": rc_id,
        "rc_op": "WithSafety",
        "safety_selection": {"Unsafe": "PreferUnordered"},
    })
}

pub(crate) fn req_get_state(id: u32) -> Value {
    serde_json::json!({"id": id, "op": "GetState"})
}

/// Unused until VEILID-018 wires `poll_send` up to actually call this.
#[allow(dead_code)]
pub(crate) fn req_app_message(id: u32, rc_id: u32, target_node_id: &str, payload: &[u8]) -> Value {
    serde_json::json!({
        "id": id,
        "op": "RoutingContext",
        "rc_id": rc_id,
        "rc_op": "AppMessage",
        "target": {"NodeId": target_node_id},
        "message": b64_encode(payload),
    })
}

/// Extracts `value.network.node_ids` from a `GetState` response's `value`
/// field, without modeling the rest of `VeilidState` -- everything else in
/// that shape is deliberately ignored.
pub(crate) fn extract_node_ids(get_state_value: &Value) -> Vec<String> {
    #[derive(Deserialize, Default)]
    struct NetworkNodeIds {
        #[serde(default)]
        node_ids: Vec<String>,
    }
    get_state_value
        .get("network")
        .and_then(|n| serde_json::from_value::<NetworkNodeIds>(n.clone()).ok())
        .map(|n| n.node_ids)
        .unwrap_or_default()
}

/// Extracts a `u32` from a response `value` field (e.g. a routing context
/// id from `NewRoutingContext`/`WithSafety`).
pub(crate) fn extract_u32(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|v| u32::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_round_trip() {
        let payload = b"hello tetron-veilid";
        let encoded = b64_encode(payload);
        assert_eq!(b64_decode(&encoded).unwrap(), payload);
        // No padding, no '+'/'/' -- matches veilid-core's own convention.
        assert!(!encoded.contains('='));
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }

    #[test]
    fn parses_real_captured_response() {
        // Exact shape captured live 2026-09-14 against a real tetron-veilid
        // instance (verify-e2e-logs/appmessage-rpc-send.json).
        let line = r#"{"type":"Response","id":2,"op":"RoutingContext","rc_id":1,"rc_op":"WithSafety","value":2}"#;
        let incoming: Incoming = serde_json::from_str(line).unwrap();
        match incoming {
            Incoming::Response(r) => {
                assert_eq!(r.id, 2);
                assert_eq!(extract_u32(&r.value), Some(2));
                assert!(r.error.is_none());
            }
            Incoming::Update(_) => panic!("expected Response"),
        }
    }

    #[test]
    fn parses_real_captured_error_response() {
        let line = r#"{"type":"Response","id":2,"op":"RoutingContext","rc_id":1,"rc_op":"WithSafety","error":{"kind":"Generic","message":"Unsafe routing mode is not allowed without the 'footgun-nodeid-target' feature enabled"}}"#;
        let incoming: Incoming = serde_json::from_str(line).unwrap();
        match incoming {
            Incoming::Response(r) => {
                assert!(r.error.is_some());
                assert!(r.error.unwrap().message.contains("footgun-nodeid-target"));
            }
            Incoming::Update(_) => panic!("expected Response"),
        }
    }

    #[test]
    fn parses_real_captured_app_message_push() {
        // Exact shape captured live 2026-09-14 (verify-e2e-logs/appmessage-rpc-listen-node2.jsonl).
        let line = r#"{"type":"Update","kind":"AppMessage","sender":"VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo","route_id":null,"message":"dGV0cm9uLXZlaWxpZC12ZXJpZnktaGVsbG8tdHYtdmVyaWZ5LTIyMzUx"}"#;
        let incoming: Incoming = serde_json::from_str(line).unwrap();
        match incoming {
            Incoming::Update(u) => {
                assert_eq!(u.kind, "AppMessage");
                assert_eq!(
                    u.sender.as_deref(),
                    Some("VLD0:GHK_QOS6VCvjxIaxkwvMU3kd-MmAlc8edzo-YmNqHLo")
                );
                let decoded = b64_decode(&u.message.unwrap()).unwrap();
                assert_eq!(decoded, b"tetron-veilid-verify-hello-tv-verify-22351");
            }
            Incoming::Response(_) => panic!("expected Update"),
        }
    }

    #[test]
    fn ignores_unknown_update_kind() {
        let line = r#"{"type":"Update","kind":"Network","started":true}"#;
        let incoming: Incoming = serde_json::from_str(line).unwrap();
        match incoming {
            Incoming::Update(u) => assert_eq!(u.kind, "Network"),
            Incoming::Response(_) => panic!("expected Update"),
        }
    }

    #[test]
    fn get_state_extracts_node_ids() {
        let value = serde_json::json!({
            "network": {"node_ids": ["VLD0:abc"], "started": true},
            "attachment": {"public_internet_ready": true},
        });
        assert_eq!(extract_node_ids(&value), vec!["VLD0:abc".to_string()]);
    }

    #[test]
    fn get_state_missing_network_is_empty() {
        assert_eq!(
            extract_node_ids(&serde_json::json!({})),
            Vec::<String>::new()
        );
    }
}
