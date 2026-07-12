//! CLI direct-connection, contact, and admin handlers.

use crate::*;

pub(crate) async fn ipc_connect(contact_id: &str, hostname: Option<String>) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Connect {
            contact_id: contact_id.to_string(),
            hostname,
        },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Joined { name, my_ip, .. } => {
            println!(
                "  {} connected — direct network {} ({})",
                style::green("✓"),
                style::value(&name),
                style::faint(&my_ip.to_string()),
            );
        }
        ipc::IpcMessage::Error { message } => print_error("connect failed", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_connections(action: Option<ConnectionsAction>) -> Result<()> {
    match action.unwrap_or(ConnectionsAction::List) {
        ConnectionsAction::List => ipc_connections_list().await,
        ConnectionsAction::Approve { id } => ipc_connections_approve(&id).await,
    }
}

pub(crate) async fn ipc_connections_list() -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, ipc::IpcMessage::Connections).await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::PendingRequests { requests } => {
            if json_enabled() {
                print_json(&serde_json::json!(requests
                    .iter()
                    .map(|r| serde_json::json!({
                        "id": r.short_id, "hostname": r.hostname, "waiting_secs": r.waiting_secs,
                    }))
                    .collect::<Vec<_>>()));
            } else if requests.is_empty() {
                println!("\n  {}\n", style::faint("no pending connection requests"));
            } else {
                let rows = requests
                    .iter()
                    .map(|r| {
                        let host = r.hostname.clone().unwrap_or_else(|| "—".to_string());
                        let wait = format!("{}s", r.waiting_secs);
                        vec![
                            layout::Cell::new(r.short_id.clone(), style::rose(&r.short_id)),
                            layout::Cell::new(host.clone(), style::value(&host)),
                            layout::Cell::right(wait.clone(), style::faint(&wait)),
                        ]
                    })
                    .collect();
                println!();
                print!("{}", table(&["id", "host", "waiting"], rows, 2));
                println!(
                    "\n  {}",
                    style::faint("approve with: torpedo connections approve <id>")
                );
            }
        }
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_connections_approve(id: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::ApproveConnection { id: id.to_string() },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_contact(action: Option<ContactAction>) -> Result<()> {
    let req = match action.unwrap_or(ContactAction::Id) {
        ContactAction::Id => ipc::IpcMessage::ContactId,
        ContactAction::Rotate => ipc::IpcMessage::RotateContact,
    };
    let rotating = matches!(req, ipc::IpcMessage::RotateContact);
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, req).await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::ContactIdResponse { contact_id } => {
            if json_enabled() {
                print_json(&serde_json::json!({ "contact_id": contact_id }));
            } else {
                if rotating {
                    println!("  {} contact id rotated", style::green("✓"));
                }
                println!("{}", contact_id);
                println!(
                    "  {}",
                    style::faint("share this so others can: torpedo connect <contact-id>")
                );
            }
        }
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_admin(network: &str, action: AdminAction) -> Result<()> {
    let req = match action {
        AdminAction::Add { identity } => ipc::IpcMessage::AdminAdd {
            network: network.to_string(),
            identity,
        },
        AdminAction::List => ipc::IpcMessage::AdminList {
            network: network.to_string(),
        },
    };
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, req).await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::AdminListResponse { admins } => {
            if json_enabled() {
                print_json(&serde_json::json!(
                    admins
                        .iter()
                        .map(|a| serde_json::json!({ "id": a.short_id, "self": a.self_node }))
                        .collect::<Vec<_>>()
                ));
            } else if admins.is_empty() {
                println!("\n  {}\n", style::faint("no admins recorded"));
            } else {
                println!();
                let mut rows = Vec::new();
                for a in &admins {
                    let (glyph, tag) = if a.self_node {
                        (style::dot_online(), style::marker("this device"))
                    } else {
                        (style::dot_offline(), String::new())
                    };
                    rows.push(vec![
                        layout::Cell::new("●", glyph),
                        layout::Cell::new(a.short_id.clone(), style::value(&a.short_id)),
                        layout::Cell::new(if a.self_node { "this device" } else { "" }, tag),
                    ]);
                }
                print!("{}", indent(&layout::columns(&rows, 2), 2));
                println!();
            }
        }
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
