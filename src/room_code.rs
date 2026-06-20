use anyhow::{Context, Result};
use iroh::EndpointId;

pub struct RoomCode {
    pub network_name: String,
    pub endpoint_id: EndpointId,
}

pub fn encode(network_name: &str, id: &EndpointId) -> String {
    let z32 = id.to_z32();
    let mut result = String::with_capacity(network_name.len() + 1 + z32.len() + z32.len() / 4);
    result.push_str(network_name);
    result.push('/');
    for (i, ch) in z32.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            result.push('-');
        }
        result.push(ch);
    }
    result
}

pub fn decode(code: &str) -> Result<RoomCode> {
    let (name, id_part) = code
        .rsplit_once('/')
        .context("room code must contain network name (name/code)")?;
    let stripped: String = id_part.chars().filter(|c| *c != '-').collect();
    let endpoint_id = EndpointId::from_z32(&stripped).context("invalid room code")?;
    Ok(RoomCode {
        network_name: name.to_string(),
        endpoint_id,
    })
}

pub fn parse_input(input: &str) -> Result<RoomCode> {
    if let Ok(id) = input.parse::<EndpointId>() {
        return Ok(RoomCode {
            network_name: String::new(),
            endpoint_id: id,
        });
    }
    decode(input).context("could not parse as EndpointId or room code")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = iroh::SecretKey::generate();
        let id = key.public();
        let code = encode("gaming", &id);
        let decoded = decode(&code).unwrap();
        assert_eq!(decoded.network_name, "gaming");
        assert_eq!(decoded.endpoint_id, id);
    }

    #[test]
    fn format_has_dashes_and_name() {
        let key = iroh::SecretKey::generate();
        let id = key.public();
        let code = encode("test-net", &id);
        assert!(code.starts_with("test-net/"));
        let id_part = code.split('/').last().unwrap();
        assert!(id_part.contains('-'));
    }

    #[test]
    fn parse_accepts_both_formats() {
        let key = iroh::SecretKey::generate();
        let id = key.public();

        let raw = id.to_string();
        let parsed = parse_input(&raw).unwrap();
        assert_eq!(parsed.endpoint_id, id);
        assert!(parsed.network_name.is_empty());

        let code = encode("mynet", &id);
        let parsed = parse_input(&code).unwrap();
        assert_eq!(parsed.endpoint_id, id);
        assert_eq!(parsed.network_name, "mynet");
    }

    #[test]
    fn invalid_code_errors() {
        assert!(decode("not-a-valid-code!!!").is_err());
    }
}
