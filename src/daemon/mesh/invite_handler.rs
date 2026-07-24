//! Invite-key handlers for `MeshManager`: `invite_create` / `invite_list` /
//! `invite_revoke`. The invite entries ride in the signed `GroupBlob` (BLOB-001)
//! instead of a machine-local invite store, so any network-key holder can mint,
//! list, or revoke, and the state is authoritative across the mesh.

use super::super::*;

impl MeshManager {
    /// Coordinator-only: mint a single-use invite key for `network`.
    ///
    /// `expires` is an optional human-readable duration ("24h", "7d", "30m").
    /// If absent, defaults to a 7-day expiry (or `tetron config set
    /// invite-default-expiry <duration>`'s configured value, CONFIG-AUDIT-002);
    /// pass `"0"` or `"never"` for a permanent invite.
    pub(crate) async fn invite_create(
        &self,
        network: &str,
        expires: Option<&str>,
    ) -> IpcMessage {
        let network = match self.resolve_network_name_or_key(network) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let (dht_notify, net_pubkey) = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            (handle.dht_notify.clone(), handle.network_key)
        };

        // INVITE-009: default 7-day expiry, overridable via `tetron config set
        // invite-default-expiry <duration>` (CONFIG-AUDIT-002). `--expires 0`
        // or `--expires never` maps to 0 for permanent (never expires).
        let ttl_secs: u64 = match expires {
            None => config::load()
                .ok()
                .and_then(|c| c.invite_default_expiry)
                .unwrap_or(7 * 24 * 3600),
            Some(dur) if dur == "0" || dur == "never" => 0,
            Some(dur) => match config::parse_duration(dur) {
                Ok(secs) => secs,
                Err(e) => {
                    return IpcMessage::Error {
                        message: format!("invalid duration '{dur}': {e}"),
                    };
                }
            },
        };

        let secret: [u8; crate::invite::SECRET_LEN] = rand::random();
        let now = now_secs();
        let (key, entry) = crate::membership::InviteEntry::from_secret(&secret, now, ttl_secs);

        let snap_bytes = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let mut s = handle.state.write().unwrap();
            s.invites.insert(key, entry);
            s.bump_generation_and_refresh();
            s.snapshot.as_ref().map(|snap| snap.msgpack_bytes.clone())
        };

        // Persist the updated blob bytes in the store so peers can fetch by hash.
        if let Some(ref bytes) = snap_bytes
            && let Err(e) = self.blob_store.blobs().add_slice(bytes).await
        {
            tracing::error!(error = %e, "invite_create: add_slice failed");
            return IpcMessage::Error {
                message: format!("failed to persist invite blob: {e}"),
            };
        }

        // Pulse the background publisher to sign + publish the updated blob.
        if let Some(ref notify) = dht_notify {
            notify.notify_one();
        }

        let invite_key = crate::invite::encode_invite_code(&net_pubkey, &secret);

        let expires_at = match ttl_secs {
            0 => None,
            ttl => Some(now + ttl),
        };

        IpcMessage::InviteCreated {
            invite_key,
            invite_id: crate::membership::InviteEntry::from_secret(&secret, now, ttl_secs).1.id,
            expires_at,
        }
    }

    /// List outstanding invites for `network` (coordinator-only).
    pub(crate) fn invite_list(&self, network: &str) -> IpcMessage {
        let network = match self.resolve_network_name_or_key(network) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let invites = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let s = handle.state.read().unwrap();
            s.invites
                .values()
                .map(|entry| ipc::InviteInfo {
                    id: entry.id.clone(),
                    created_at: entry.created,
                    expires_at: entry.expires,
                    revoked: entry.revoked,
                })
                .collect::<Vec<_>>()
        };

        IpcMessage::InviteListResponse { invites }
    }

    /// Coordinator-only: revoke (mark as revoked) an invite by its short id.
    pub(crate) async fn invite_revoke(&self, network: &str, invite_id: &str) -> IpcMessage {
        let network = match self.resolve_network_name_or_key(network) {
            Ok(name) => name,
            Err(message) => return IpcMessage::Error { message },
        };
        let network = network.as_str();
        if let Err(e) = self.coordinator_handle(network) {
            return e;
        }

        let (dht_notify, snap_bytes) = {
            let Some(handle) = self.networks.get(network) else {
                return IpcMessage::Error {
                    message: format!("network '{network}' not active"),
                };
            };
            let mut s = handle.state.write().unwrap();
            if let Err(e) = crate::membership::revoke_invite(&mut s.invites, invite_id) {
                return IpcMessage::Error {
                    message: format!("{e:#}"),
                };
            }
            s.bump_generation_and_refresh();
            (
                handle.dht_notify.clone(),
                s.snapshot.as_ref().map(|snap| snap.msgpack_bytes.clone()),
            )
        };

        // Persist the updated blob bytes in the store so peers can fetch by hash.
        if let Some(ref bytes) = snap_bytes
            && let Err(e) = self.blob_store.blobs().add_slice(bytes).await
        {
            tracing::error!(error = %e, "invite_revoke: add_slice failed");
            return IpcMessage::Error {
                message: format!("failed to persist revoked blob: {e}"),
            };
        }

        // Pulse the background publisher so the revoked entry is signed + published.
        if let Some(ref notify) = dht_notify {
            notify.notify_one();
        }

        IpcMessage::Ok {
            message: format!("invite '{invite_id}' revoked"),
        }
    }
}
