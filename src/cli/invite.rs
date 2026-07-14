//! CLI invite-key handlers: create/list/revoke single-use invite keys.

use crate::*;

/// Dispatch the `tetron invite <network> <action>` command.
pub(crate) async fn ipc_invite(network: &str, action: InviteAction) -> Result<()> {
    let req = match action {
        InviteAction::Create { expires } => ipc::IpcMessage::InviteCreate {
            network: network.to_string(),
            expires,
        },
        InviteAction::List => ipc::IpcMessage::InviteList {
            network: network.to_string(),
        },
        InviteAction::Revoke { invite_id } => ipc::IpcMessage::InviteRevoke {
            network: network.to_string(),
            invite_id,
        },
    };
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, req).await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::InviteCreated {
            invite_key,
            invite_id,
            expires_at,
        } => {
            if json_enabled() {
                print_json(&serde_json::json!({
                    "invite_key": invite_key,
                    "invite_id": invite_id,
                    "expires_at": expires_at,
                }));
            } else {
                let expiry = expires_at
                    .map(|ts| format!("  expires  unix:{ts}"))
                    .unwrap_or_default();
                println!();
                println!("  invite key  {invite_key}");
                println!("  invite id   {invite_id}{expiry}");
                println!();
                println!("  share: tetron join {invite_key}");
            }
        }
        ipc::IpcMessage::InviteListResponse { invites } => {
            if json_enabled() {
                print_json(&serde_json::json!(invites
                    .iter()
                    .map(|i| serde_json::json!({
                        "id": i.id,
                        "created_at": i.created_at,
                        "expires_at": i.expires_at,
                        "used": i.used,
                    }))
                    .collect::<Vec<_>>()));
            } else if invites.is_empty() {
                println!("\n  (no invites)\n");
            } else {
                let rows: Vec<Vec<String>> = invites
                    .iter()
                    .map(|i| {
                        let status = if i.used {
                            "used".to_string()
                        } else if i.expires_at > 0 && i.expires_at <= crate::membership::now_secs() {
                            "expired".to_string()
                        } else {
                            "active".to_string()
                        };
                        vec![i.id.clone(), status, i.created_at.to_string()]
                    })
                    .collect();
                println!();
                print!("{}", table(&["id", "status", "created"], rows, 2));
                println!();
            }
        }
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
