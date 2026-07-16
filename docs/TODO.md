# tetron TODO

## Recently completed

- **Invite in blob (BLOB-001)**: invites now ride in signed `GroupBlob` (keyed by blake3(secret)). Any network-key holder can mint/list/revoke. Invite code drops coordinator (48 B vs 80 B). Validation against blob, removal on admission. DHT-notify-driven immediate republish. Committed at 79375be.
- **Peer address cache (CACHE-001)**: persistent cache at `<config_dir>/peercache.msgpack`. Solves all-offline reconnection. Committed at aa5715e.
- **Default subnet /16 -> /24**: changed `membership::default_subnet()` from `10.88.0.0/16` to `10.88.0.0/24` (256 addresses, enough for personal/team meshes). Updated SUBNET-011/SUBNET-013 spec entries, CLI help text, all Rust doc strings, README, HOWTO. Libspec-linked.
- **Invite key admission** (Phases 1-4): invite store, IPC handlers, CLI (create/list/revoke), post-create auto-mint, e2e tested on 3 machines. Room-id joins still queue for live approval (both paths coexist).
- **Old torpedo cleanup**: service stopped, binary/config removed on AORUS, xps-17, and SB-OS.
- **E2E test results** logged in `docs/TESTING.md` Stage 9.
- **SUBNET_COLLISION.md updated**: added VLAN analogy, /24 refs, policy routing as correct physical model (Phase 2), physical-model alignment for every solution, updated recommendation with phases.
- **Laptop fleet plan drafted** in `docs/IDEAS_LaptopFleet.md`: multi-coordinator (routine), invite-in-blob, peer address cache. Two-tier model is sufficient -- three-tier model set aside.

## Laptop fleet (current plan)

Three changes to make a network of laptop users work without an always-on
member:

### 1. Multi-coordinator (routine) -- DONE (COORD-001)

`tetron admin add <net> <identity>` is now the recommended default practice
in the README quickstart and HOWTO. Every fully trusted member should be a
co-coordinator to avoid a single point of failure for admission, invite
minting, and member management.

### 2. Invite in blob -- DONE (BLOB-001, committed at 79375be)

Invites ride in the signed `GroupBlob`. Any online coordinator validates.
Replay race window (~30-60s DHT poll) accepted as initial implementation.
Fetch-before-publish merge not yet implemented (single-coordinator prompt
is safe; multi-coordinator races are benign for invites since only the
secret hash is stored -- a duplicate is harmless). Local reject cache and
`InviteUsed` gossip are deferred.

### 3. Cache peer addresses to disk -- DONE (CACHE-001, committed at aa5715e)

### See also

- `docs/IDEAS_LaptopFleet.md` -- full writeup with rationale and composition
- `docs/PRIVILEGE_TIERS.md` -- design discussion that informed this direction
- `docs/DECISIONS.md` -- decision tables

## Packaging

- **Build a .deb package** for tetron: systemd service file, config dir, binary, postinst/prerm scripts. Simplifies install on Debian/Ubuntu vs the current `sudo tetron install` from a loose binary.

## WebUI addon

A separate unprivileged process that serves a web UI on localhost, translating
HTTP requests to Unix-socket IPC messages. No daemon changes needed.

**Architecture:**

```
Browser ──HTTP──> tetron-web (unprivileged user process)
                       │
                       │ msgpack over Unix socket (4-byte BE length prefix)
                       ▼
                 /var/run/tetron/tetron.sock (mode 0666)
                       │
                       │ SO_PEERCRED per-request authorization
                       ▼
                 tetron daemon (root)
```

**Why this works:**

- IPC socket is `mode 0666` -- any local user can connect. Authorization is
  per-request via `SO_PEERCRED`, so `tetron-web` is authorized based on the UID
  of the process running it. Mutating operations need root or the configured
  operator UID.
- Protocol is length-prefixed msgpack (`IpcMessage` enum in
  `tetron-proto/src/ipc.rs`). Any language with msgpack + Unix socket support
  (Python, Go, Rust, Node) can speak it.

**WebUI action to IPC message mapping:**

| Button in UI | IPC message |
|---|---|
| List networks + peers | `Status` |
| Create network | `Create { mode, name, hostname, ... }` |
| Mint invite | `InviteCreate { network, expires }` |
| List invites | `InviteList { network }` |
| Revoke invite | `InviteRevoke { network, invite_id }` |
| Kick member | `Kick { network, peer }` |
| Promote co-coordinator | `AdminAdd { network, identity }` |
| List admins | `AdminList { network }` |
| Leave network | `Leave { name }` |
| Nuke network | `Nuke { name }` |

Every operation maps 1:1 from a browser button click to an IPC round-trip.
No WebSocket streaming needed for basic use -- poll `Status` every few seconds.

**Trade-offs:**

- **Session/auth.** If bound to `localhost`, no TLS or login needed (like
  Syncthing at `localhost:8384`). Remote access needs a reverse proxy with TLS
  + auth.
- **Daemon restarts.** The socket disappears and reappears on daemon restart.
  WebUI must watch for the socket and reconnect.
- **Wire types.** `tetron-proto` is an internal crate. A WebUI in the same repo
  (Rust) shares it trivially. A different language reimplements the wire format.
- **Deployment.** Two binaries. `tetron-web` needs its own install step, systemd
  unit, and data directory. Could ship as an optional Cargo feature or companion
  repo.

## UX cleanup

- **`tetron join --name` rename to `--local-nickname`**: the current `--name` flag on join is a local-only alias, but `--name` on create sets the published network name. Same flag, different scopes, confusing. Rename to `--local-nickname` on join, keep `--name` on create.

- **`tetron hostname` rename command**: see `docs/DECISIONS.md` section 6 for feasibility analysis. The kick+rejoin workaround is not a real substitute (requires coordinator to mint invite, connectivity interruption). Simplified design (~110 lines) needs: (a) IpcMessage::SetHostname variant, (b) MeshManager::set_hostname handler that updates config + sends MeshHello to coordinators, (c) re-add hostname processing in coordinator control reader (30 lines, deleted by MINIMAL-014, available in git history), (d) CLI command. Deferred pending user demand.

- **`tetron leave` accept network key as well as name**: users may only have the invite key or room id handy when uninstalling. Add a resolution helper: try exact name match first, then scan all known networks for a public-key prefix match against the per-network keys in config. Update uninstall docs to show the key form.

## Subnet collision

- **Reject overlapping subnets on create/join**: check all active networks before creating or joining. See `docs/SUBNET_COLLISION.md` for scenario analysis, solutions, and recommendation (Solution 1+2 with `--force` flag).
- **Policy routing (deferred)**: per-network routing tables so identical subnets do not collide. Higher effort, correct long-term fix.

## Hardening

- **KICK-REQUIRES-ID: tetron kick requires endpoint-id only (no hostname/IP resolution)**: `tetron kick` currently accepts hostname, mesh IP, or short id (via `resolve_peer_name`). For a destructive action like kicking, the peer should be identified by its cryptographic identity only -- human-friendly names are ambiguous and a kick by the wrong name is disruptive. Change `kick_member` to call `resolve_short_id_any_network` directly instead of `resolve_peer_name`. Update CLI help text, docs/HOWTO.md, and README.md to show only the short-id form. `admin add` keeps the friendly resolution.

## High priority

- **Reusable keys (--reusable)**: add `--reusable` flag to `tetron invite <net> create` -- adds hash to `GroupBlob.reusable_keys`, signs + republishes blob. Any coordinator validates against the blob.

## Docs cleanup (deferred — do once the application works)

A full docs sanitization pass is needed before public release. The current docs
were written during development and contain internal details that should not be
in the final public docs. Specific items:

1. **Real hostnames**: replace `590I-AORUS-ULTRA`, `xps-17-9720`, `xps-17`,
   `usbos-1`, `SB-OS`, `AORUS` with generic names like `node-a`, `node-b`,
   `coordinator`, `member`.

2. **Real IPs / subnet values**: testing used `10.77.0.0/24`, `10.88.169.205`,
   etc. Replace with example addresses (`10.88.0.1`, `10.88.0.2`) or the
   `SUBNET_COLLISION.md` examples.

3. **Real network names**: `"shallows"`, `"testnet"`, `"multicoord"` are
   testing artifacts. Use `"mynetwork"` or `"example"` in user-facing docs.

4. **Commit SHAs**: `docs/TODO.md` and `CHANGELOG.md` reference specific SHAs
   (`79375be`, `aa5715e`, etc.). These are development history. For users,
   replace with feature names or version numbers. For internal dev docs, they
   can stay.

5. **Libspec class names / requirement IDs**: `BLOB-001`, `COORD-001`,
   `LIVE-001`, `CACHE-001`, `FRAG-001`, `SUBNET-BUG-001`, `NUKE-CONSENSUS`
   are internal tracking labels. User-facing docs should not reference them.

6. **Real-world incident details**: "SSH key exchange stalled on 'shallows'",
   "found 2026-07-15 while testing co-coordinator promotion on network
   'shallows'" — these are bug-hunting notes. Public docs should not contain
   them.

7. **Outdated feature references**: any remaining mentions of `torpedo`,
   `rayfish`, removed features (firewall, Magic DNS, SSH, etc.) in docs that
   are supposed to describe current tetron.

8. **docs/ that are dev-internal**: `DECISIONS.md`, `IDEAS_LaptopFleet.md`,
   `SUBNET_COLLISION.md`, `PRIVILEGE_TIERS.md` — decide which are internal
   dev notes (maybe move to a `docs/internal/` subdirectory or delete before
   publishing).

9. **`.claude/` references**: `MEMORY.md`, `MEMORY/MEMORY_tetron.md`,
   `TODO/TODO_tetron.md` contain the same real hostnames, IPs, and SHAs.
   These stay local (gitignored) but should still be cleaned up if shared.

**Rule of thumb for the cleanup pass:** every file that a new user would read
(README, HOWTO, TESTING, AGENTS.md, CHANGELOG, man page) should have zero
development-environment fingerprints. Every file that is internal (spec,
.debriefs, design docs) can keep them.

## Bugs

- **SUBNET-BUG-001: TUN created with local subnet, not network subnet, silently breaking data plane**: When a node joins a network whose subnet differs from the node's locally configured subnet (`tetron config set subnet` or default), the TUN device is created with the *local* subnet, not the network's subnet from the `GroupBlob`. The member is assigned a mesh IP from the network's subnet (visible in `tetron status`), but its TUN interface has an IP from the local subnet instead. Packets addressed to the member's correct mesh IP arrive via QUIC but are written to a TUN whose IP is in a different range -- the kernel does not recognize the dst IP as local and drops the packet. This scilently breaks the data plane (no ping, no TCP) with no error message.

  **How to reproduce:**
  1. Node A (coordinator) creates network with `--subnet 10.77.0.0/24` (or has its node config set to that subnet).
  2. Node B joins using an invite key but has a different local subnet (e.g. `10.88.0.0/16` default).
  3. `tetron status` on both sides shows the member with the correct mesh IP from the network's subnet (e.g. `10.77.0.205`).
  4. TUN on node B shows a different IP (e.g. `10.88.169.205`), not the one in status.
  5. Ping from A to B: ICMP echos go out on A's TUN, reach B via QUIC, but B's kernel drops them because dst IP `10.77.0.205` does not match B's TUN IP `10.88.169.205`.

  **Severity:** medium -- silent data-plane failure, no errors logged anywhere. Only affects networks where members have inconsistent local subnet configs (common when subnet was changed after initial setup).

  **Suggested fix:** On join, compare the network's subnet (from blob) against the local node subnet. If they differ, either:
  - (a) Reject the join with a clear error: "network subnet 10.77.0.0/24 differs from your node subnet 10.88.0.0/16; run `tetron config set subnet 10.77.0.0/24 && sudo tetron restart` first."
  - (b) Auto-adopt: update the local node subnet to match the network's subnet and warn the user.
  - (c) Per-network TUN (or policy routing) as the correct long-term fix (see SUBNET_COLLISION.md).

  **Found:** 2026-07-15, real-world deployment with AORUS (10.77.0.0/24) and usbos-1 (10.88.0.0/16) on network "shallows".
