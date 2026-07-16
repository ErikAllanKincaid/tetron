# Upstream rayfish changes worth cherry-picking

Fork point: `9e14241` (upstream master, ~171 commits ahead).
Generated: 2026-07-16.

## HIGH VALUE -- directly applicable bug fixes

### 1. Background roster dial (prevents boot stall)

**Commit:** `02dd60e` -- `fix: dial roster peers in the background so a dead member can't stall boot`

**Problem:** `connect_to_roster_peers` dials every peer serially and blocks the join/restore. A single stale/unreachable peer stalls the whole mesh for iroh's per-peer handshake timeout (~30s). Affects every join and daemon restart with multiple peers.

**Fix:** Spawns concurrent per-peer dials in a `FuturesUnordered` with `MESH_PEER_DIAL_TIMEOUT = 30s` and cancel-awareness. The coordinator link is established first (blocking), then the rest dial in background. Network is usable before the full mesh connects.

**tetron file:** `src/daemon/mesh/join.rs` line 352+ (`connect_to_roster_peers`) -- tetron has the old serial version.

---

### 2. Timeout-bounded full-mesh dials

**Commit:** `fe3f3c0` -- `fix: bound iroh pairing and full-mesh dials with a timeout`

**Problem:** `dial_all_members` (called by coordinator on restore) also dials serially with no timeout. An unreachable peer hangs the restoring coordinator.

**Fix:** Adds `DIAL_TIMEOUT = 10s` + `tokio::select!` on `cancelled()` to `dial_all_members`. Also applies the same pattern to the pairing dial (not relevant to tetron).

**tetron file:** `src/daemon/mesh/create_join.rs` line 1250+ (`dial_all_members`) -- no timeout, no cancel-awareness.

---

## MEDIUM VALUE -- architectural improvements with adaptation cost

### 3. CloseReason enum + kick-vs-leave distinction

**Commit:** `1c193b9` -- `fix(mesh): decide membership only from the signed record; fix unpair, status, overlay flap`

**Problem:** tetron uses `intentional: bool` for disconnect events. KICK_CODE is treated as `intentional = true` (same as LEAVE_CODE), meaning a kick from a member during flapping causes the coordinator to prune itself. This was the root cause of overlay-flap desync upstream.

**Fix:** Introduces `CloseReason { Transient, Left, Kicked }` enum. Only `Left` (LEAVE_CODE) prunes the member from the roster. A `Kicked` close never evicts -- the coordinator reconnects to the peer (a member can not evict its coordinator). The in-band `KickedFromNetwork` control message (signed-record confirmed) is the authoritative kick mechanism.

**tetron adaptation:** Needs changes in `src/forward.rs` (DisconnectEvent struct), `src/daemon/mesh/coordinator.rs` (disconnect handler), `src/daemon/mesh/join.rs` (control listener), and `src/daemon/mesh/runtime.rs` (kick_member). Tetron's per-network `DisconnectEvent` with a `network` field is already a better design than upstream's old model -- the `CloseReason` enum is the piece that matters. Moderate effort, high reward.

---

### 4. Overlay address filter

**Commit:** `f827462` -- `fix(mesh): drop overlay IPs from iroh's direct-address candidates`

**Problem:** Mesh IPs (from `10.88.x.x`) leak into iroh's advertised transport candidates via interface discovery. Peers dial the mesh IP, looping traffic back through the tunnel (flapping path, high relay latency).

**Fix:** Adds `DirectAddrFilter` (requires iroh fork patch -- `Builder::direct_addr_filter`) + `AddrFilter` to strip overlay addresses from iroh's candidate set. Two layers: `DirectAddrFilter` drops them at the source (interface gathering), `AddrFilter` drops them at publish time (defense in depth).

**tetron adaptation:** Requires a patched iroh fork (upstream uses `rayfish/iroh` branch `direct-addr-filter`). tetron uses iroh 1.0.0 vs upstream's 1.0.2 patched. The `AddrFilter` part (`.addr_filter(...)` on the builder) may already be available in iroh 1.0.0. The `is_overlay_ip()` helper is simple and portable. Worth checking if the iroh API surface supports this without a patch.

---

## LOW VALUE -- cosmetic, maintenance, or already in tetron

### 5. Restored networks in status immediately

**Commit:** `b26c26b` -- `fix(daemon): show restored networks in status right after (re)start`

**Cosmetic fix:** Ensures `tetron status` shows networks immediately on boot before the first reconverge completes.

---

### 6. Cargo.lock dependency bump

**Commit:** `fd4bbb0` -- `build: bump transitive dependencies in Cargo.lock`

**Maintenance:** Bumps transitive dependencies. Worth doing periodically.

---

### 7. Install script hardening

**Commit:** `7e53d2f` -- `fix(install): harden the installer and gate it in CI`

**Useful if tetron gets an install.sh:** Upstream hardened their installer (sha256 verification, color detection, proper error handling). tetron currently has no `install.sh`.

---

## Already in tetron

### Co-coordinator key persists across restart

**Upstream commit:** `52951da` -- `fix(connect): persist direct-network co-coordinator key across restart`

**Already handled:** `AdminGrant` handler writes `net.network_secret_key = Some(key)` via `config::save_network`, and `connect_all_networks` reads `net.network_secret_key.is_some()` to decide coordinator restore.

---

## Not applicable (different architecture or removed subsystems)

| Upstream change | Reason N/A |
|---|---|
| `6830cb3` -- Don't sever shared connection when pruning | tetron has per-network connections (HashMap), not shared |
| `63e8ae9` -- First-connection reader spawn | tetron spawns readers at join/accept sites, not gated on add() |
| `16efb41` -- Coordinator self-rename propagation | Hostname rename removed (MINIMAL-014) |
| `acbdcdb` -- Fast reconnect + richer status | Tied to upstream NetworkRegistry + one-connection-per-identity |
| `8f89ec7` -- Route config-writing through daemon | tetron's config model is simpler and already correct |
| All Android commits (~50) | Android support removed |
| All file transfer commits (~20) | File sharing removed |
| All exit-node commits | Exit nodes removed |
| GUI / SSH / pairing / revocation commits | All removed subsystems |
| `5ec651b` -- Switch to tun-rs | tetron uses the `tun` crate, different TUN library |
| `5270932` -- TUN cancel-safe read | tetron uses tokio `AsyncRead::read_buf` (cancel-safe by design) |
