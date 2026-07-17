# tetron TODO

## Recently completed

- **CONVERGE-001 FIXED**: read-before-write DHT guard prevents co-coordinator publish race. `spawn_network_publisher` and `spawn_lazy_publisher` now resolve DHT before publishing and skip when another coordinator published a newer blob. Group poller reconciles within 60s.
- **SUBNET-BUG-001 FIXED**: Join with mismatched subnet now rejects with clear error. Verified e2e across 4 nodes. Commit fa29ef9.
- **E2E test session 2026-07-16**: tested SUBNET-BUG-001, co-coordinator admission flow. Found CONVERGE-001 (publish race) and CONVERGE-002 (stale DHT restore). Full log at `docs/TEST_LOG-2026-07-16.md`.
- **TEST_PROCEDURE.md updated**: added Phase 0.5 (stale TUN check), Phase 1c (TUN cleanup), Phase 2 (SUBNET-BUG-001 test), Phase 3 (TUN consistency check).
- **Invite in blob (BLOB-001)**: invites now ride in signed `GroupBlob` (keyed by blake3(secret)). Any network-key holder can mint/list/revoke. Invite code drops coordinator (48 B vs 80 B). Validation against blob, removal on admission. DHT-notify-driven immediate republish. Committed at 79375be.
- **Peer address cache (CACHE-001)**: persistent cache at `<config_dir>/peercache.msgpack`. Solves all-offline reconnection. Committed at aa5715e.
- **Default subnet /16 -> /24**: changed `membership::default_subnet()` from `10.88.0.0/16` to `10.88.0.0/24` (256 addresses, enough for personal/team meshes).
- **Invite key admission**: invite store, IPC handlers, CLI (create/list/revoke), post-create auto-mint, e2e tested.
- **SUBNET_COLLISION.md updated**: added VLAN analogy, /24 refs, policy routing as correct physical model.

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

### FIXED

- **SUBNET-BUG-001: TUN created with local subnet, not network subnet, silently breaking data plane**: Fixed in fa29ef9. Join now rejects with a clear error when the network's subnet differs from the node's configured subnet. Verified in e2e test on 2026-07-16 across 4 nodes.

- **CONVERGE-003: Removed member never cleans up locally (ghost member)**: Fixed (see spec `CONVERGE-003`). A node dropped from the roster (kicked, or a CONVERGE-001 casualty) now leaves the network locally — token cancelled, config deleted, `tetron status` stops lying about a healthy membership — instead of silently redialing forever. Verified live on 3 bare-metal machines (590i-aorus-ultra, xps-17-9720, X10SRA) 2026-07-16: an explicit `tetron kick` correctly triggered local teardown on the kicked node within ~1 poller cycle, and a second, organic recurrence of CONVERGE-001 (below) during the same session was also handled cleanly by the fix rather than producing another ghost.

- **CONVERGE-001 / CONVERGE-005: Co-coordinator publish race with original coordinator**: Fixed at the root (see spec `CONVERGE-005`). The read-before-write guard (a9b0afa) only compared raw hashes — it could tell "did the DHT change under me" but not "is that change actually newer," so an out-of-date coordinator's periodic republish could permanently win over a co-coordinator's fresher admission just by writing later, and once wedged a publisher deferred to the unrecognized hash *forever*. Reproduced twice more on 2026-07-16 with a9b0afa + 6b2954d already deployed, confirming it wasn't fully closed.

  Real fix: a monotonic `generation: u64` on `GroupBlob` and the signed pkarr network record (`g,<n>`, mirroring the existing cert-generation-floor record). `dht_read_before_write` now arbitrates by generation, not write order — always publish over a strictly lower generation regardless of hash, always defer to a strictly higher one; an exact same-generation tie with divergent content is left alone rather than fought over (the loser's next mutation bumps past it). `spawn_group_poller` also gained the seed-peer fallback `fetch_verified_blob` already had, fixing a compounding "could not fetch updated group blob from any peer" failure even with a live connection.

  Verified live on the same 3 machines: aorus's log shows `group blob changed current_generation=0 remote_generation=3`, correctly fetching and applying the co-coordinator's admission within one 60s poller cycle — zero manual restart, where it previously stayed wedged at 2 members indefinitely every time. Re-ran `tetron kick` afterward to confirm CONVERGE-003 still fires correctly alongside the new logic.

  **Found:** 2026-07-16, e2e test with aorus (original) + xps (co-coordinator). **Fixed:** 2026-07-16, same session.

- **CONVERGE-002: Stale DHT restore on coordinator restart**: Fixed as a consequence of CONVERGE-005 — the DHT record's generation is now always monotonically correct, so a restarting coordinator's restore-from-blob fetch reliably finds the current state instead of a stale one.

  **Found:** 2026-07-16, consequence of CONVERGE-001.

- **CONVERGE-006: Member boot-restore had no config fallback**: Fixed (see spec `CONVERGE-006`). `join_network_inner`'s boot-restore call (`initial=false`) now falls back to a `GroupBlob` built from the persisted `NetworkConfig` roster when `resolve_and_fetch_blob` fails (pkarr unreachable, no dialable seed peer), matching the config-fallback the coordinator restore path already had. Verified live on X10SRA: blocked the pkarr relay via iptables/ip6tables DROP across a full daemon restart — the fallback fired, and the member fully reconnected to both peers (direct, 0% ping loss) using the fallback roster to dial the coordinator, since DHT reachability is only needed to *resolve* the peer, not to talk to it once dialed. Superseded the original CONVERGE-004 write-up below, which was based on an incomplete diagnosis.

### OPEN

- **Exact same-generation tie with divergent content is not merged (CONVERGE-005 known limitation)**: If two coordinators independently mutate from the same base generation (rare — requires near-simultaneous admits before either publish lands), the guard leaves the DHT alone rather than picking a winner; the loser's admission is deferred until its own next local mutation bumps past the tie. Not a regression (the old code had the same effective outcome, just via uncontrolled write-order races instead), and admission is idempotent (a deferred admit can simply be retried), so not treated as a priority fix.

### SUPERSEDED

- **CONVERGE-004 (original write-up, inaccurate)**: Originally claimed "group poller never spawned on boot-time member reconnect." Re-verified live on 2026-07-16 by restarting X10SRA's daemon under normal conditions: the log clearly shows `group poller spawned network=converge-test` during an ordinary boot-restore. Reading `finalize_join` confirmed why — it's the single shared endpoint for both a fresh `tetron join` and a boot-restore (`connect_all_networks` → `join_network_inner(initial=false)` → `dial_reconnect` → `finalize_join`), and it spawns the poller unconditionally regardless of role; `dial_reconnect`'s own fallback (`state_from_blob`, for when the coordinator is unreachable) still reaches `finalize_join` too. The original diagnosis was made mid-way through a messier live incident (X10SRA already being actively denied by both coordinators from the CONVERGE-001 race) without isolating the actual cause carefully enough. The real, narrower gap this investigation actually found is CONVERGE-006 above (`resolve_and_fetch_blob` itself has no fallback when DHT resolution fails outright) — now fixed.

## Procedural Notes (from e2e test 2026-07-16)

- **TUN-CLEANUP**: Stale TUN devices survive `sudo tetron uninstall`. Always verify and delete them: `for dev in $(ip -o link show | grep -oP 'tun\d+'); do sudo ip link delete "$dev"; done`
- **SCP-TRUNCATION**: The 29MB binary transfer through SSH jump host may time out. Verify file size after copy. Use `scp -C` (compression) or longer timeout for remote deploys.
- **xeon40-specific**: Binary at `/tmp/tetron-new` must be re-copied after each fresh build (separate from the local machine's binary).
