//! CLI join-request handlers: requests/accept/deny.

use crate::*;

pub(crate) async fn ipc_requests(network: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::Requests {
            network: network.to_string(),
        },
    )
    .await?;
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
                println!("\n  {}\n", style::faint("no pending join requests"));
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
                    style::faint(&format!("admit with: torpedo accept {network} <id>"))
                );
            }
        }
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_accept_request(network: &str, id: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::AcceptRequest {
            network: network.to_string(),
            id: id.to_string(),
        },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

pub(crate) async fn ipc_deny_request(network: &str, id: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::DenyRequest {
            network: network.to_string(),
            id: id.to_string(),
        },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
