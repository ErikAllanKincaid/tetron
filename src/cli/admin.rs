//! CLI admin (co-coordinator) handlers.

use crate::*;

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
