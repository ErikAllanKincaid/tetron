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

### 1. Multi-coordinator (routine)

`tetron admin add <net> <identity>` should be the default practice. Every
fully trusted user gets the network key. No single machine is the SPOF for
administration. Already implemented -- just needs to be the recommended
workflow in docs and HOWTO.

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

## Subnet collision

- **Reject overlapping subnets on create/join**: check all active networks before creating or joining. See `docs/SUBNET_COLLISION.md` for scenario analysis, solutions, and recommendation (Solution 1+2 with `--force` flag).
- **Policy routing (deferred)**: per-network routing tables so identical subnets do not collide. Higher effort, correct long-term fix.

## High priority

- **Reusable keys (--reusable)**: add `--reusable` flag to `tetron invite <net> create` -- adds hash to `GroupBlob.reusable_keys`, signs + republishes blob. Any coordinator validates against the blob.
