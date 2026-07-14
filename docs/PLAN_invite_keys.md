# Plan: Single-Use Invite Keys as Primary Admission

## Problem

tetron currently has no way to mint join credentials. Admission is live approval only (`tetron requests`/`accept`), which requires someone at the coordinator to approve every join. This makes unattended/scripted joins (CI runners, fleet provisioning) impossible.

The upstream invite-minting code was removed entirely in MINIMAL-013 — the `InviteStore` ledger, `invites/<network>.toml`, the `InviteCreate`/`InviteList`/`InviteRevoke` IPC ops and daemon handlers, and the `tetron invite` CLI subcommand are all gone. What remains is:
- Joiner-side invite-code encode/decode in `src/invite.rs` (used for D1 compat — redeeming invites from full-tetron coordinators)
- `JoinRequest.invite_secret` on the wire (still carried, always `None`)
- Reusable-key validation against `GroupBlob.reusable_keys` in `accept.rs` (D1 compat only)

## Design

### Admission model change

The room id (`tetron join <room-id>`) currently queues the joiner for live approval. This becomes **discovery-only**: `tetron join <room-id>` fails with a message saying an invite key is required. The only way onto a network is `tetron join <invite-key>`.

The coordinator mints invite keys. Each invite key is a one-time credential: presenting it during the join handshake admits the bearer automatically — no approval queue, no coordinator attendance required beyond the minting step.

### Invite key format (unchanged from upstream)

```
bs58(network_pubkey(32) || coordinator_pubkey(32) || secret(16))
```

Already implemented in `src/invite.rs`. The coordinator pubkey pins which coordinator to dial; the secret is a random 128-bit value whose blake3 hash is stored locally by the minting coordinator.

### Invite store

Per-network, flat directory under the config dir:

```
/etc/tetron/invites/<network>/
  <invite-id>.toml
```

Each file contains:

```toml
# invite-id = random 8-byte hex (also the filename stem)
id = "a1b2c3d4e5f6g7h8"
secret_hash = "blake3-hex-of-secret"    # 64 hex chars
created_at = 1719000000
expires_at = 1719600000                 # optional, 0 = never
used = false
```

The invite **id** is a short hex string printed alongside the invite key so the coordinator can list and revoke invites by id. The secret itself is never stored — only its hash. This means even if the store is compromised, old secrets cannot be recovered.

### CLI surface

```
tetron invite <network> create [--expires <duration>]
tetron invite <network> list
tetron invite <network> revoke <invite-id>
```

`create` prints the invite key and the invite id. `list` shows all outstanding invites with their status (unused/used/expired). `revoke` marks an invite as used so it cannot be redeemed.

### Join flow with invite key

1. Joiner runs `tetron join <invite-key>` — the CLI already detects invite codes vs room ids in `ipc_join()` (network.rs:83-86)
2. IPC `Join` message carries `invite: Some(secret)` and `coordinator: Some(coord_pubkey)`
3. Joiner dials the pinned coordinator (or falls back to any peer if the pinned coordinator is unreachable)
4. `JoinRequest` carries `invite_secret` on the wire (field already exists in control protocol)
5. Coordinator's `CoordinatorAcceptState::handle_connection` receives the `JoinRequest` with `invite_secret`
6. `redeem_invite_and_admit` checks secret against local invite store (currently only checks `GroupBlob.reusable_keys`)
7. If valid: mark invite as used (single-use), auto-admit the joiner, skip pending queue
8. If invalid/expired/used: send `JoinDenied`

### Reusable keys (future phase)

Reusable keys are a separate mechanism. The hash rides the signed `GroupBlob.reusable_keys` (field already exists, validation already works). Add `tetron invite <network> create --reusable` which:
1. Generates a random secret
2. Adds its hash to `GroupBlob.reusable_keys`
3. Signs and republishes the blob
4. Prints the invite key (which any coordinator on the network can validate against the blob)

This is lower priority — single-use keys cover the primary use case.

### Post-create invite output

When `tetron create` succeeds, it currently prints:

```
  created muddy-sunset-whale
    address  10.88.0.1  ·  abcd…1234
  ──────────────────────────────────────────────
  next: tetron join <room-id>       share this to invite peers
        tetron up                   activate the VPN
```

Change the "share this to invite peers" line to print an initial invite key:

```
  created muddy-sunset-whale
    address  10.88.0.1  ·  abcd…1234
  ──────────────────────────────────────────────
  next: tetron join <invite-key>    single-use invite (one more available)
        tetron invite <net> create  mint another invite
        tetron up                   activate the VPN
```

The daemon mints a single invite automatically on `create` and returns the key in the `Created` IPC response.

---

## Implementation phases

### Phase 1: Invite store + daemon minting

**Files to create:**
- `src/daemon/mesh/invite_store.rs` — the `InviteStore` type: load/save per-network invites, create/validate/revoke

**Files to modify:**
- `src/daemon/mod.rs` — add `invite_store` to `NetworkState`
- `src/daemon/mesh/accept.rs` — modify `redeem_invite_and_admit` to check local invite store first, fall back to reusable keys for D1 compat
- `src/daemon/mesh/create_join.rs` — mint initial invite on `create_network_inner`, return it in IPC response; thread invite store to accept paths
- `src/daemon/mesh/mod.rs` — register the new module

**Key types:**

```rust
// src/daemon/mesh/invite_store.rs

pub struct StoredInvite {
    pub id: String,          // 8-byte hex
    pub secret_hash: String, // blake3 hex
    pub created_at: u64,
    pub expires_at: u64,     // 0 = never
    pub used: bool,
}

pub struct InviteStore {
    dir: PathBuf,
}

impl InviteStore {
    pub fn new(network_dir: &Path) -> Self;
    pub fn create(&self, ttl_secs: Option<u64>) -> Result<(String, String)>;  // (invite_key, invite_id)
    pub fn list(&self) -> Result<Vec<StoredInvite>>;
    pub fn revoke(&self, id: &str) -> Result<()>;
    pub fn validate_and_burn(&self, secret: &[u8]) -> Result<bool>;  // single-use: burns on success
}
```

### Phase 2: IPC + daemon handlers

**Files to modify:**
- `tetron-proto/src/ipc.rs` — add `InviteCreate`, `InviteList`, `InviteRevoke` IPC ops + response types
- `src/daemon/mod.rs` — add `invite_create`, `invite_list`, `invite_revoke` handler methods on `MeshManager`
- `src/ipc.rs` — add dispatch arms for new IPC ops

**New IPC messages:**

```rust
enum IpcMessage {
    InviteCreate {
        network: String,
        expires: Option<String>,   // human duration, parsed daemon-side
    },
    InviteCreated {
        invite_key: String,
        invite_id: String,
        expires_at: Option<u64>,
    },
    InviteList {
        network: String,
    },
    InviteListResponse {
        invites: Vec<StoredInviteInfo>,
    },
    InviteRevoke {
        network: String,
        invite_id: String,
    },
}
```

### Phase 3: CLI

**Files to modify:**
- `src/main.rs` — add `Invite` subcommand under `Command` with `Create`, `List`, `Revoke` sub-subcommands
- `src/cli/invite.rs` — rename current file (join-request handlers) to `requests.rs` or merge; add `ipc_invite_create`, `ipc_invite_list`, `ipc_invite_revoke` handlers
- `src/cli/mod.rs` — update module references

**New CLI structure:**

```
tetron invite <network> create [--expires <duration>]
tetron invite <network> list
tetron invite <network> revoke <invite-id>
```

`tetron invite` with no subcommand shows help with available subcommands.

### Phase 4: Post-create invite

**Files to modify:**
- `tetron-proto/src/ipc.rs` — add `initial_invite_key: Option<String>` to `Created` response
- `src/daemon/mesh/create_join.rs` — mint one invite on `create_network_inner`, include key in response
- `src/cli/network.rs` — display the initial invite key in `ipc_create` output
- `src/main.rs` — `Command::Create` help text update

### Phase 5: Reusable keys (optional, lower priority)

- Add `--reusable` flag to `tetron invite <network> create --reusable`
- On mint: add hash to `GroupBlob.reusable_keys`, republish blob
- Validation path already exists via `validate_reusable_key`

---

## Files not changed

- `src/invite.rs` — encode/decode already works, no changes needed
- `src/config.rs` — invite store dir auto-created by config_dir helper, no new config keys
- `tetron-proto/src/control.rs` — `JoinRequest.invite_secret` already on the wire
- `src/daemon/mesh/join.rs` — `JoinContext` already carries `invite_secret`

## Wire compat with full-tetron

The invite store is local to each coordinator (no gossip between coordinators in v1). The invite key encodes the minting coordinator's pubkey, so the joiner dials that specific coordinator. If that coordinator is offline, the join fails — the joiner must wait or obtain a fresh invite from another coordinator. Cross-coordinator invite gossip (via `InviteShare`/`InviteUsed`) is deferred.

Reusable keys (Phase 5) do not have this limitation — any coordinator can validate against the signed blob.

---

## Future work (not in scope)

- Cross-coordinator invite gossip
- `--count N` flag to mint multiple invites at once
- QR code output for mobile
- `--hostname` binding on invite (coordinator-authoritative name)
- Email/share link generation
