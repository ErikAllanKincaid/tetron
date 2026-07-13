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
                println!("\n  (no admins recorded)\n");
            } else {
                println!();
                for a in &admins {
                    let tag = if a.self_node { "  ·this device·" } else { "" };
                    println!("  {} {tag}", a.short_id);
                }
                println!();
            }
        }
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
