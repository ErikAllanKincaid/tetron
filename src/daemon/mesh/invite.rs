//! Join-request handlers for `MeshManager`: list/accept/deny pending join
//! requests on a closed network. Split out of `daemon/mod.rs`.

use super::super::*;

impl MeshManager {
    pub fn list_requests(&self, network: &str) -> IpcMessage {
        let Some(handle) = self.networks.get(network) else {
            return IpcMessage::Error {
                message: format!("network '{network}' not active"),
            };
        };
        if !handle.role.is_coordinator() {
            return IpcMessage::Error {
                message: format!("only the coordinator of '{network}' has join requests"),
            };
        }
        let s = handle.state.read().unwrap();
        let requests = s
            .pending
            .iter()
            .map(|(id, pj)| ipc::PendingRequestInfo {
                short_id: id.fmt_short().to_string(),
                hostname: pj.hostname.clone(),
                waiting_secs: pj.requested_at.elapsed().as_secs(),
            })
            .collect();
        IpcMessage::PendingRequests { requests }
    }

    pub async fn accept_request(&self, network: &str, id_prefix: &str) -> IpcMessage {
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }
        // Find and remove the pending request matching the short id prefix.
        let pending = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let mut s = handle.state.write().unwrap();
            let found = s
                .pending
                .keys()
                .find(|k| {
                    k.fmt_short().to_string().starts_with(id_prefix)
                        || k.to_string().starts_with(id_prefix)
                })
                .copied();
            found.and_then(|id| s.pending.remove(&id).map(|pj| (id, pj)))
        };
        let Some((identity, pj)) = pending else {
            return IpcMessage::Error {
                message: format!("no pending request matching '{id_prefix}'"),
            };
        };

        let user_id = pj.device_cert.as_ref().map(|c| c.user_identity);
        let ip = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let mut s = handle.state.write().unwrap();
            // Assign authoritatively from the current roster so two coordinators
            // accepting concurrently can be reconciled by the reconverge tiebreak.
            let (ip, collision_index) = crate::membership::assign_ip(&s.members, &identity, s.subnet);
            let members = s.members.clone();
            let _ = s.approved.approve(
                ApprovedEntry {
                    identity,
                    ip,
                    hostname: pj.hostname.clone(),
                    user_identity: user_id,
                    device_cert: pj.device_cert.clone(),
                    collision_index,
                },
                &members,
            );
            s.refresh_snapshot();
            ip
        };
        self.store_and_publish_group(network).await;
        broadcast_control_msg(
            &self.peers,
            &ControlMsg::MemberApproved {
                identity,
                ip,
                hostname: pj.hostname.clone(),
                device_cert: pj.device_cert.clone(),
            },
        )
        .await;
        IpcMessage::Ok {
            message: format!("accepted {} — they'll join shortly", identity.fmt_short()),
        }
    }

    pub fn deny_request(&self, network: &str, id_prefix: &str) -> IpcMessage {
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }
        let Some(handle) = self.networks.get(network) else {
            return IpcMessage::Error {
                message: format!("network '{network}' not active"),
            };
        };
        let mut s = handle.state.write().unwrap();
        let found = s
            .pending
            .keys()
            .find(|k| {
                k.fmt_short().to_string().starts_with(id_prefix)
                    || k.to_string().starts_with(id_prefix)
            })
            .copied();
        match found {
            Some(id) => {
                s.pending.remove(&id);
                IpcMessage::Ok {
                    message: format!("denied {}", id.fmt_short()),
                }
            }
            None => IpcMessage::Error {
                message: format!("no pending request matching '{id_prefix}'"),
            },
        }
    }
}
