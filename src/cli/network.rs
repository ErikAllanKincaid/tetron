//! CLI handlers for network lifecycle: create / join / nuke / leave.

use crate::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn ipc_create(
    mode: GroupMode,
    network_name: Option<String>,
    hostname: Option<String>,
    subnet: Option<String>,
    nuke_consensus: Option<u32>,
    tor: bool,
    force: bool,
) -> Result<()> {
    // Validate the CIDR locally so the user gets an immediate error, but send it
    // as the raw string; the daemon re-parses it authoritatively.
    if let Some(ref cidr) = subnet {
        membership::parse_cidr(cidr)?;
    }
    // Same early-error convenience as --subnet above; the daemon re-validates
    // authoritatively (NUKE-CONSENSUS-THRESHOLD-001).
    if let Some(n) = nuke_consensus {
        anyhow::ensure!(
            n >= 2,
            "--nuke-consensus must be at least 2 (a value of 0 or 1 would let a single \
             coordinator nuke unilaterally once a second coordinator exists)"
        );
    }
    let transport = if tor {
        Some(config::TransportMode::Tor)
    } else {
        None
    };
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Create {
            mode,
            network_name,
            hostname,
            transport,
            subnet,
            nuke_consensus,
            force,
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Created {
            network,
            network_key,
            my_ip,
            my_ipv6,
            warning,
            initial_invite_key,
            subnet,
        } => {
            if json_enabled() {
                print_json(&created_json(
                    &network,
                    &network_key,
                    my_ip,
                    my_ipv6,
                    &warning,
                    &initial_invite_key,
                    &subnet,
                ));
                return Ok(());
            }
            let key_str = network_key.to_string();
            let short = if key_str.len() > 12 {
                format!("{}…{}", &key_str[..4], &key_str[key_str.len() - 4..])
            } else {
                key_str.clone()
            };
            let _ = my_ipv6;
            println!();
            println!("  created {network}");
            println!("    address  {}  ·  {}", my_ip, short);
            if !subnet.is_empty() {
                // Always shown, not just when it differs from what the caller
                // expected -- every network on this node gets a genuinely
                // distinct subnet now (auto-advanced past a collision when
                // `--subnet` wasn't given), so this is the one place that
                // choice becomes visible instead of being silent.
                println!("    subnet   {subnet}");
            }
            match &initial_invite_key {
                Some(invite) => {
                    let share = format!("tetron join {invite}");
                    print_next(&[
                        (&share, "single-use invite (one more available)"),
                        ("tetron invite <net> create", "mint another invite"),
                        ("tetron resume", "activate the VPN"),
                    ]);
                }
                None => {
                    let share = format!("tetron join {network_key}");
                    print_next(&[
                        (&share, "share this to invite peers"),
                        ("tetron resume", "activate the VPN"),
                    ]);
                }
            }
            if let Some(w) = &warning {
                println!("  ⚠ {w}");
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => {
            print_error("create failed", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

/// Builds the `--json` payload for a successful `tetron create` (CREATE-JSON-001).
/// `network_key` is emitted as the full key, never the pretty-print's truncated form.
fn created_json(
    network: &str,
    network_key: &iroh::EndpointId,
    my_ip: std::net::Ipv4Addr,
    my_ipv6: Option<std::net::Ipv6Addr>,
    warning: &Option<String>,
    initial_invite_key: &Option<String>,
    subnet: &str,
) -> serde_json::Value {
    serde_json::json!({
        "network": network,
        "network_key": network_key.to_string(),
        "my_ip": my_ip.to_string(),
        "my_ipv6": my_ipv6.map(|a| a.to_string()),
        "warning": warning,
        "initial_invite_key": initial_invite_key,
        "subnet": subnet,
    })
}

pub(crate) async fn ipc_join(
    invite_code: &str,
    alias: Option<&str>,
    hostname: Option<String>,
    tor: bool,
    force: bool,
) -> Result<()> {
    let transport = if tor {
        Some(config::TransportMode::Tor)
    } else {
        None
    };
    // `tetron join <arg>` accepts a self-contained invite code that decodes to the
    // network pubkey plus a one-time secret. A bare room id (raw network public key)
    // is still parsed for backward compat but the daemon will deny it (tetron is
    // invite-only — LIVE-001 removed live approval). The daemon side rejects bare
    // room-id joins with "a valid invite key is required". The wire field stays
    // `network_key` regardless -- by the time this crosses IPC it's always the
    // resolved public key, never invite-code text.
    let (network_key, invite) = match invite::decode_invite_code(invite_code) {
        Ok((net_pubkey, secret)) => (net_pubkey.to_string(), Some(secret)),
        // INVITE-CHECKSUM-001: a decode failure is only the room-id fallback
        // for a *genuine* bare room id (32-byte base58). Anything that was
        // clearly meant to be an invite (48/52-byte payload, e.g. a checksum
        // mismatch from a corrupted or mistyped code) surfaces the specific
        // error up front instead of silently reaching the daemon as a room id
        // and getting the generic "a valid invite key is required" denial.
        Err(e) if !invite::is_bare_room_id(invite_code) => return Err(e),
        Err(_) => (invite_code.to_string(), None),
    };
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Join {
            network_key,
            alias: alias.map(|s| s.to_string()),
            hostname,
            transport,
            invite,
            force,
        },
    )
    .await?;
    // Joining dials the coordinator and runs the handshake daemon-side, so this
    // can take a few seconds.
    eprintln!("joining…");
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => {
            println!("{}", message);
        }
        ipc::IpcMessage::Joined {
            network,
            my_ip,
            my_ipv6,
            warning,
        } => {
            let _ = my_ipv6;
            println!();
            println!("  joined {network}");
            println!("    address  {}", my_ip);
            print_next(&[
                ("tetron status", "see who's online"),
                ("tetron resume", "activate the VPN"),
            ]);
            if let Some(w) = &warning {
                println!("  ⚠ {w}");
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => {
            print_error("join failed", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_nuke(
    network_key: &str,
    force: bool,
    cancel: bool,
    second: Option<&str>,
) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Nuke {
            network_key: network_key.to_string(),
            force,
            cancel,
            second: second.map(str::to_string),
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => {
            print_error("error", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_kick(network_key: &str, endpoint_id: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Kick {
            network_key: network_key.to_string(),
            endpoint_id: endpoint_id.to_string(),
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => {
            print_error("error", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_leave(network: &str, force: bool) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Leave {
            network: network.to_string(),
            force,
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => {
            print_error("error", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_json_emits_full_key_and_all_fields() {
        let key = iroh::SecretKey::generate().public();
        let value = created_json(
            "spooky-otter-lake",
            &key,
            "10.88.0.2".parse().unwrap(),
            Some("fd00::2".parse().unwrap()),
            &Some("relay-only".to_string()),
            &Some("invite-abc".to_string()),
            "10.88.0.0/24",
        );
        assert_eq!(value["network"], "spooky-otter-lake");
        // Full key, not the pretty-print's truncated "abcd…wxyz" form.
        assert_eq!(value["network_key"], key.to_string());
        assert!(value["network_key"].as_str().unwrap().len() > 12);
        assert_eq!(value["my_ip"], "10.88.0.2");
        assert_eq!(value["my_ipv6"], "fd00::2");
        assert_eq!(value["warning"], "relay-only");
        assert_eq!(value["initial_invite_key"], "invite-abc");
        assert_eq!(value["subnet"], "10.88.0.0/24");
    }

    #[test]
    fn created_json_nulls_absent_optional_fields() {
        let key = iroh::SecretKey::generate().public();
        let value = created_json(
            "net",
            &key,
            "10.88.0.2".parse().unwrap(),
            None,
            &None,
            &None,
            "10.88.0.0/24",
        );
        assert!(value["my_ipv6"].is_null());
        assert!(value["warning"].is_null());
        assert!(value["initial_invite_key"].is_null());
    }
}
