//! CLI handlers for network lifecycle: create / join / nuke / leave.

use crate::*;

pub(crate) async fn ipc_create(
    mode: GroupMode,
    name: Option<String>,
    hostname: Option<String>,
    subnet: Option<String>,
    tor: bool,
) -> Result<()> {
    // Validate the CIDR locally so the user gets an immediate error, but send it
    // as the raw string; the daemon re-parses it authoritatively.
    if let Some(ref cidr) = subnet {
        membership::parse_cidr(cidr)?;
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
            name,
            hostname,
            transport,
            subnet,
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Created {
            name,
            network_key,
            my_ip,
            my_ipv6,
            warning,
            initial_invite_key,
        } => {
            let key_str = network_key.to_string();
            let short = if key_str.len() > 12 {
                format!("{}…{}", &key_str[..4], &key_str[key_str.len() - 4..])
            } else {
                key_str.clone()
            };
            let _ = my_ipv6;
            println!();
            println!("  created {name}");
            println!("    address  {}  ·  {}", my_ip, short);
            match &initial_invite_key {
                Some(invite) => {
                    let share = format!("tetron join {invite}");
                    print_next(&[
                        (&share, "single-use invite (one more available)"),
                        ("tetron invite <net> create", "mint another invite"),
                        ("tetron up", "activate the VPN"),
                    ]);
                }
                None => {
                    let share = format!("tetron join {network_key}");
                    print_next(&[
                        (&share, "share this to invite peers"),
                        ("tetron up", "activate the VPN"),
                    ]);
                }
            }
            if let Some(w) = &warning {
                println!("  ⚠ {w}");
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => print_error("create failed", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_join(
    network_key: &str,
    name: Option<&str>,
    hostname: Option<String>,
    tor: bool,
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
    // room-id joins with "a valid invite key is required".
    let (network_key, invite) = match invite::decode_invite_code(network_key) {
        Ok((net_pubkey, secret)) => (net_pubkey.to_string(), Some(secret)),
        Err(_) => (network_key.to_string(), None),
    };
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Join {
            network_key,
            name: name.map(|s| s.to_string()),
            hostname,
            transport,
            invite,
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
            name,
            my_ip,
            my_ipv6,
            warning,
        } => {
            let _ = my_ipv6;
            println!();
            println!("  joined {name}");
            println!("    address  {}", my_ip);
            print_next(&[
                ("tetron status", "see who's online"),
                ("tetron up", "activate the VPN"),
            ]);
            if let Some(w) = &warning {
                println!("  ⚠ {w}");
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => print_error("join failed", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_nuke(
    name: &str,
    force: bool,
    cancel: bool,
    second: Option<&str>,
) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Nuke {
            name: name.to_string(),
            force,
            cancel,
            second: second.map(str::to_string),
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_kick(network: &str, peer: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Kick {
            network: network.to_string(),
            peer: peer.to_string(),
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_leave(name: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Leave {
            name: name.to_string(),
        },
    )
    .await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
