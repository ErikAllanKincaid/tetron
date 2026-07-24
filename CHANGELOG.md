# Changelog

All notable changes to Torpedo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`tetron invite`/`tetron admin` now accept a network key, not just the local display name (INVITE-ADMIN-NETWORK-KEY-001)**: same fallback `tetron leave` already had (`LEAVE-NETWORK-KEY-001`) -- if you only had the invite key or room id handy, there was no way to mint an invite or grant admin at all. Both now try the local name first (unchanged), then fall back to a `network_key` prefix match (>=10 characters, or the full key).
- **NUKE-CONSENSUS's proposer threshold is now configurable at creation (NUKE-CONSENSUS-THRESHOLD-001)**: previously always exactly 2 distinct coordinators regardless of network size -- meaningless consensus on a large multi-coordinator network. `tetron create --nuke-consensus <n>` (default 2, must be >= 2) sets it once at creation; fixed thereafter, same treatment as `--subnet`. Visible via `tetron status`/`--json`.
- **`tetron status --json`'s per-network `name` field gains a `network` twin (STATUS-NETWORK-FIELD-001)**: `name` reads stale next to `subnet`/`network_key`/etc. Both fields are populated identically for now -- `name` is not removed yet, since `tetron-webui`/`tetron-systray` both read it as a direct Rust field, not just a JSON key. New code (including tetron's own CLI) should read `network`; see `DO-NOT-COMMIT/TODO.md` for the fleet checklist gating `name`'s eventual removal.

### Fixed

- **`tetron status`'s "members" count included admins (STATUS-003)**: found live on a real multi-admin network -- the per-network header's `members <online>/<total>` counted every peer, admin or not, so an online co-coordinator was counted twice (once under `admins`, again under `members`), inflating both numbers. Now filtered to non-admin peers only, matching the peer table directly below it. `--json` output was never affected, only the derived text-mode header.
- **`tetron admin <net> add <hostname>` failed against a hostname shown in `tetron status` moments earlier (STATUS-004)**: found live right after the above -- `erikk-ThinkPad-P1` (the host's real OS hostname, mixed-case) failed to resolve, even though `tetron status` had just displayed it as `erikk-thinkpad-p1`. Every stored hostname is always lowercased, so a user typing it back with its original casing had no way to match. Hostname resolution for `admin add` is now case-insensitive; `kick`/`nuke --second`'s short-id resolution (a cryptographic identifier, not a recalled name) is unaffected.

## [0.7.0] - 2026-07-20

### Fixed

- **A second network created without an explicit `--subnet` silently reused the first one's (SUBNET-UNIQUE-001)**: found immediately while live-testing `SUBNET-DRIFT-001` with a second network -- the same node ended up with the identical address on two supposedly-independent networks. Harmless functionally (each network's TUN is already isolated), but defeats a real point of configurable subnets and is a foreseeable source of routing/firewall confusion. `tetron create` now automatically picks a genuinely free subnet (advancing past any collision with a network this node already has) when `--subnet` isn't given, and rejects an explicit `--subnet` outright (rather than silently overriding it) if it collides with one you already have. The resolved subnet is now always printed in `create`'s own output, so the choice is never silent even when auto-picked.
- **A handful of unit tests were silently testing nothing (found while fixing the above)**: several tests across `membership.rs`/`config.rs`/`control.rs`/`packet.rs`/`peers.rs`/`forward.rs` used `100.64.x.x` (the pre-fork default subnet) as test data, inherited from before this fork changed the default and never updated. Most were harmless placeholder values, but three genuinely passed for the wrong reason -- e.g. a test checking that derived addresses avoid the *reserved* addresses of the current default subnet was comparing against the *old* subnet's reserved addresses, so the assertion was vacuously true no matter what. Fixed the vacuous ones to test what they actually claim to; swapped the rest to the current default for consistency.
- **`tetron invite list` showed a revoked invite as "used" (INVITE-STATUS-001)**: found bug-hunting after the above -- revoking an invite nobody ever redeemed made it indistinguishable from one someone actually joined with. The two are genuinely different states, but there was no way to tell them apart: an invite that's actually redeemed is removed from the roster entirely, so "used" could only ever really mean "revoked" for anything still in the list -- it just claimed a distinction the data never supported. `--json`'s field and the text `status` column now say `revoked` instead.

## [0.6.0] - 2026-07-20

### Fixed

- **A daemon restart could silently break a network's own data-plane routing (SUBNET-DRIFT-001)**: found live-testing the `tetron status` subnet display -- a coordinator restart could silently re-resolve its own network to the wrong subnet (falling back to the compiled default instead of the network's actual one), attach its TUN there, and republish that wrong value into the signed roster every other member trusts, spreading the corruption. The underlying encrypted connection stayed up and even exchanged control-channel bytes, so nothing looked broken in `tetron status` -- but real application traffic over the mesh was silently dropped (confirmed: 100% ping loss). Fixed at the root: a network's subnet is now always persisted explicitly (never inferred from the node's current, unrelated default), and the daemon now refuses to bring a network up at all -- with a clear error -- if the resolved subnet ever disagrees with what that network's own signed roster already says this node's address is, rather than silently routing on top of the inconsistency. Verified live on real hardware: a fresh create/join round-trips its subnet correctly across a daemon restart with 0% ping loss; the previously-broken test network now fails to restore with a clear error on both sides instead of silently misrouting.
- **`tetron kick` no longer claims to refuse on "open networks" (RENAME-M02 follow-up)**: dead code left over from the D1 severance cleanup -- tetron has never been able to create an open network (`MINIMAL-013`) and can't coordinate a full-torpedo one either (D1's ALPN split makes that connection impossible), so this branch could never actually trigger. Removed the check and the doc/help text implying it was a real behavior. `kick` is now refused only against a coordinator or yourself.

### Added

- **`tetron leave` now accepts a network key, not just the local display name (LEAVE-NETWORK-KEY-001)**: previously the only way to identify which network to leave was its locally-assigned display name (as shown in `tetron status`) -- if you only had the invite key or room id handy (e.g. at uninstall time), there was no way to `leave` at all. `tetron leave` now tries the local name first (unchanged), then falls back to a `network_key` prefix match (same rules `nuke`/`kick` already use: >=10 characters, or the full key).
- **`tetron status` now shows each network's subnet, and a real aligned peer table (STATUS-002)**: a `network <name>  subnet <cidr>  admins <online>/<total>  members <online>/<total>  interface <tun_name>` header, followed by a column-aligned `role / host / ip / via` table (real per-column width, not just consistent spacing) with your own node included as the first row. `role` (`admin`/`member`) is new -- shows who else holds the network key at a glance, without a separate `tetron admin list` call.

### Changed

- **`tetron status`'s output redesigned for clarity (STATUS-002)**: the ambiguous `id`/`join` lines (same value, two different labels, two different lengths, one of them stale/misleading post-invite-only) are now one `network_key` line -- shown only to admins, since a plain member can't use it anyway. The daemon header gained a `traffic` line (previously computed, never shown in text mode). Per-peer IPv6 and connection-health metrics (rtt/tx/rx) are no longer in the default text view -- both were adding width/clutter for detail most users don't need day to day, and remain fully available via `--json`. `coordinator` is now displayed as `admin` throughout `tetron status` (matches `tetron admin`, the command that already used that word for the same concept) -- display text only, no behavior change.
- **`sudo tetron install` now says what it's actually doing (INSTALL-OUTPUT-001)**: previously ran silently until "waiting for daemon…", giving no indication a privileged system service was being written and enabled. Now prints the concrete unit/job name and exact file path before writing it (e.g. `installing systemd service 'tetron' -> /etc/systemd/system/tetron.service`), and announces the enable/restart (or launchd load) step.
- **`tetron nuke`/`tetron kick`'s network positional renamed `net_id` -> `network_key` (CLI-VOCAB-005)**: `--help` showed `<NET_ID>` and `tetron status` (text) labeled the same value `id`, but `tetron status --json` already called it `network_key` -- the field name a user scripting against `tetron status --json | jq` actually has to know. Standardized on `network_key` everywhere: `--help` now shows `<NETWORK_KEY>`, and the status text line reads `network_key <short>` instead of `id <short>`. Behavior is unchanged (still accepts an unambiguous >=10-char prefix, or the full key).
- **`tetron kick`'s second positional renamed `peer` -> `endpoint_id` (CLI-VOCAB-005)**: same problem one level deeper -- `kick` resolves this argument only against a member's endpoint id, never a hostname, but `tetron status --json`'s `PeerStatus` exposes both `endpoint_id` and `hostname` fields side by side with no way to tell which one `kick` wants. `--help` now shows `<ENDPOINT_ID>`, matching the JSON field. `tetron admin add`'s `peer` argument is unchanged -- it genuinely accepts a hostname or short id.
- **Dropped the email from `Cargo.toml`/`tetron-proto/Cargo.toml`'s `authors` field, kept the name, added a co-author.** Was `["Dario <dario@rayfish.xyz>"]`, now `["Dario", "ErikAllanKincaid"]` — no email for either. An `authors` field is a published contact point (crates.io/docs.rs listings, security-scanner disclosure targets); leaving upstream's personal address there risked routing this fork's own traffic to someone unconnected to it, and the new co-author entry uses a GitHub username rather than an email for the same reason. No functional change.

### Removed

- **`StatusResponse.pending_networks` (dead field, never displayed)**: same shape as the already-removed `pending_requests` -- claimed to reflect a live-approval queue removed by `LIVE-001`, was always empty, and nothing ever read it in either text or `--json` output.

## [0.5.0] - 2026-07-19

### Changed

- **`tetron up`/`tetron down` renamed to `tetron resume`/`tetron standby` (CLI-VOCAB-004)**: `down`'s target state was already called "standby" everywhere (`tetron status`, daemon logs) — the verb never matched the noun. Full depth, not just CLI text: the wire protocol's `IpcMessage::Up`/`Down` are now `Resume`/`Standby` too. Hard cutover, no aliases — update scripts/muscle memory. `tetron status`'s daemon-wide summary now shows `active`/`standby` (was `up`/`standby`), matching the existing `active` field used everywhere else in the output.
- **`tetron resume`'s silent install-on-first-use is removed.** The old `tetron up` would silently install and start the system service (needing root) if no daemon was reachable — a hidden scope escalation baked into what looked like a routine activate command. `tetron resume` is now a stable, single-meaning operation: with no daemon reachable it always errors ("tetron service is not running. Install and start it with: `sudo tetron install`"), regardless of caller privilege. Use `sudo tetron install` explicitly for first-time setup — it already did the same install/start work, just without the redundant name.

### Fixed

- **`tetron resume`'s own response still said "up" in a few places after the CLI-VOCAB-004 rename**: found live-testing on real hardware (Linux + macOS) — replying "already up"/"'name' up"/"VPN up" while `tetron status` correctly said "active"/"standby". Renamed to match ("already active"/"'name' active"/"VPN active").

## [0.4.0] - 2026-07-18

### Added

- **`tetron status` shows each network's OS TUN interface name (STATUS-001)**: with a node joined to several networks, there was previously no way to tell which interface (`tun0`, `tun1`, ...) belongs to which network without guessing from `ip link show` order or daemon logs. Now printed as an `interface` line per network in both text and `--json` output.
- **`tetron leave` auto-promotes co-coordinators before stranding anyone (STRANDED-COORDINATOR-WARN)**: leaving a network as its only coordinator, while other members still exist, used to leave them permanently ungoverned (no one able to admit joiners, mint invites, or kick, ever again). Now `tetron leave` first grants the network key to every currently-connected member (the same effect as `tetron admin add`) before proceeding. Only refuses — naming exactly who's affected — if a member is offline right now and genuinely can't be reached; `--force` overrides.
- **Per-network standby (STANDBY-PER-NETWORK)**: `tetron up`/`tetron down` take a new optional `--network <name>` to bring just one joined network's data plane up or down instead of every one — e.g. take a "work" network offline at end of day while "home" stays up. Omit the flag for the original daemon-wide behavior, unchanged. `tetron status` shows a `·standby·` marker on any network currently down.

### Fixed

- **`tetron admin add` could resolve a peer's hostname on the wrong network (ADMIN-ADD-NETWORK-SCOPE)**: with two joined networks each having a same-named member (e.g. `alice`), `tetron admin <net-A> add alice` could resolve to network-B's `alice` instead of network-A's, since `resolve_peer_name` searched every joined network's roster instead of just the target one. Failed closed rather than granting the wrong peer (the resolved identity was then checked against the target network's own connection table), but produced a confusing error when the intended target was actually reachable. Fixed by scoping the hostname lookup to the target network.

## [0.3.0] - 2026-07-18

### Added

- **macOS support, live-verified on real Apple Silicon hardware**: tetron now runs on macOS as a real target, not an aspirational one — installed as a launchd service (`sudo tetron up`), joined to a live network, and confirmed working end to end over both IPv4 and IPv6 (ping and real multi-megabyte file transfers, both directions, both address families), including surviving a `tetron down`/`up` standby cycle. This is the first time the per-network IPv6 addressing shipped in 0.2.0 has been tested on macOS at all. Prebuilt macOS release binaries are not published yet (`build-macos` stays disabled in CI pending that separate step); build from source with `cargo build --release` in the meantime.

### Fixed

- **macOS installed the wrong IPv4 route for a network's peer range (MACOS-001)**: `route_peer_range`'s macOS variant hardcoded the pre-fork `100.64.0.0/10` CGNAT literal for the IPv4 route instead of the network's actual configured subnet, silently breaking IPv4 connectivity on every macOS-joined network by default (tetron's own default subnet is `10.88.0.0/24`, not `100.64.0.0/10`). Fixed by threading the network's real subnet through, the same pattern already used for MULTISEG-007.
- **A member's locally-tracked subnet could revert to the node-wide default on reconnect (MULTISEG-008)**: present since multi-segment TUN shipped in 0.2.0, not something introduced since. Rejoining or reconnecting to a network (including a `tetron down`/`up` cycle) rebuilt a member's in-memory state using the node's default subnet instead of that network's own — on Linux this had no visible effect (IPv4 routing there doesn't consult this value, and IPv6 routing derives its prefix elsewhere), which is why it shipped unnoticed; on macOS, where the route is installed explicitly, it meant connectivity could silently stop routing to the correct subnet after a standby cycle. Fixed by threading the network's already-correctly-resolved subnet through the member reconnect path, closing the one call site multi-segment TUN's original subnet-correctness sweep had missed.

## [0.2.0] - 2026-07-18

### Added

- **Per-network subnet field on local config (MULTISEG-001, internal groundwork)**: `NetworkConfig` now persists an optional per-network subnet, laying the groundwork for multi-segment TUN (one TUN device + subnet per network). Purely additive — `None` means "use the node-wide subnet" (today's actual behavior), nothing reads the field yet, and no user-facing behavior changes.
- **Multi-segment TUN (MULTISEG-002..007): every joined network now gets its own TUN device and its own subnet**, instead of one shared TUN/subnet across every network the node belongs to. Structurally, two tetron networks on one host are now two separate interfaces — reachability boundaries are a property of the interface, not just which peers you've dialed. `tetron create --subnet`/`tetron join` no longer require every joined network to agree on one node-wide subnet; a network's own subnet (from its signed roster, or `--subnet` at create time) is what its TUN is built in, applied immediately (no restart needed). Network teardown (`leave`/`nuke`/being kicked) now actually deletes that network's TUN device instead of relying on the kernel to reclaim it whenever the whole daemon process eventually exits — closes a real, previously-observed "stale TUN device survives a restart/crash" bug. IPv6 addressing was still global at the time this landed (see the IPV6-001..003 entry below for the follow-up that resolved it). A node belonging to two networks still does **not** route traffic between them — each network stays a fully isolated peer mesh even on a node that belongs to several; this is unlike two physical NICs, where the host's own routing would bridge them. See MULTISEG-003/MULTISEG-007 in `spec/design_spec.py` for full detail.
- **Per-network IPv6 addressing (IPV6-001..003): peer IPv6 addresses are now scoped per network, closing the "IPv6 works on one segment only" limitation multi-segment TUN shipped with.** `derive_ipv6` now takes the network's own public key alongside the identity, structurally splitting the address into a fixed tag, a 48-bit network-prefix (shared by every member of one network, giving it a real, routable `/56` block), and a 72-bit peer-part (unique per identity *and* per network, so the same identity gets an unrelated address in each network it joins). Every joined network now gets its own IPv6 route into its own TUN device simultaneously — no more picking one network to "win" the route. No collision-index is needed (a 1% accidental-collision probability needs ~3.1 billion members of one network at 72 bits); admission now additionally rejects a *deliberately grinded* IPv6 collision against a different identity (mirroring the existing IPv4 collision check), closing the narrower adversarial-grinding gap the accidental-collision math alone doesn't cover.
- **Nuke requires consensus on multi-coordinator networks (NUKE-CONSENSUS)**: `tetron nuke <net>` on a network with a single coordinator still destroys it immediately, unchanged. With two or more coordinators, a coordinator running `tetron nuke <net>` now proposes instead of nuking outright; the network is destroyed only once two distinct coordinators have proposed (a second coordinator running the same command seconds it) within a 24h window. `tetron nuke <net> --cancel` withdraws your own proposal; `tetron nuke <net> --second <short-id>` explicitly names the proposal being seconded. `tetron status` surfaces any pending proposal so members can see a nuke is being considered before it happens. Prevents a single compromised or reckless coordinator from unilaterally destroying a network nobody else agreed to lose.

### Fixed

- **A subnet-diverging joined network dropped 100% of real coordinator traffic as "spoofed" (MULTISEG-007)**: when a member joined a network whose subnet differed from the node's own default (e.g. `tetron create --subnet 10.77.0.0/16` on a node whose other networks use the default range), the join path derived both its own IP and the coordinator's IP from the wrong (node-wide default) subnet instead of the network's actual one. The coordinator's real, correctly-addressed packets then failed the per-packet anti-spoof check and were silently dropped — 100% data-plane loss on that network, while its control-plane connection looked healthy. Found via live 3-machine testing of multi-segment TUN (a node coordinating two networks on two different subnets at once); fixed by threading the already-correctly-derived per-network IP through instead of recomputing it, and by resolving the coordinator's IP from the just-admitted membership roster rather than re-deriving it. Verified fixed live, same topology, 0% loss both directions.
- **Nuke tombstones are now actually fetchable by remaining members**: `tetron nuke` previously only ever published the empty record's `(hash, generation)` pointer to the DHT, never the actual bytes anywhere fetchable — the executing coordinator calls `leave_network` (closing its connections) immediately after, so it was typically the only node that ever held the content, and everyone else's `member_removed` (CONVERGE-003) self-removal check never fired. Predates NUKE-CONSENSUS (the original single-coordinator nuke had the same gap) but only surfaced with other members actually present to notice. A nuke tombstone's content is fully deterministic given just its generation, so every node now reconstructs and verifies it locally instead of ever needing to fetch it from anyone. Found and fixed via live 3-machine testing.
- **Group poller no longer gets stuck forever on a coincidental generation tie**: a node's own unrelated local mutations (e.g. pruning a peer that gracefully left) could independently land its generation on the same number a different coordinator's mutations reached, purely by chance; the poller treated that tie as "nothing new" and silently stopped fetching for that network forever, even though the content genuinely differed. It now also fetches on an exact-generation tie when the hash differs, not just on a strictly newer generation. Found via live 3-machine testing.
- **A coordinator restart can no longer resurrect stale state (CONVERGE-008)**: a coordinator's very first publish attempt after restart used to bypass the DHT comparison entirely and publish unconditionally. Combined with falling back to stale local config when the DHT was unreachable at restart, this could republish superseded — or, worst case, already-destroyed — network state and have it unconditionally win over whatever was actually live. The bypass is removed; every publish, first or not, now compares against the real DHT state first. Found and fixed via live 3-machine testing (deliberately blocking a coordinator's DHT access across a restart, confirming it no longer overwrites the live record with its stale view).
- **Several `--help`/error strings promised behavior that didn't exist, found via a full CLI doc-comment-vs-handler audit**: `tetron admin add`'s help text and the daemon's own error message both claimed a mesh IP could identify a member — it never could (only hostname or short id resolve); dropped the false claim. `resolve_peer_name`'s internal doc comment claimed it backs `tetron kick` — it doesn't (`kick` intentionally resolves by short id only, since removing the wrong member needs a cryptographic identity, not a spoofable hostname); corrected to name `admin add` as its actual, and only, caller. `AGENTS.md` had the same "kick"/"hostname not accepted" claim backwards for `admin add`; corrected. `invite_create`'s internal doc comment said an invite never expires by default — it directly contradicted the actual (correct, intentional) 7-day default four lines below it. Three leftover references to the removed Magic DNS feature (MINIMAL-012) in `--help` text (`create --hostname`/`join --hostname`'s `.gaming.ray` example, `down`'s doc mentioning "Magic DNS") and one in code (`resolve_peer_name`'s now-permanently-dead hostname `.`-splitting) were also cleaned up. `tetron up` and `tetron install`'s `--help` text was nearly identical despite meaningfully different behavior (`up` tries an unprivileged activate first and only installs as a root fallback; `install` always requires root and unconditionally reinstalls/restarts) — reworded both to state the actual distinction.
- **`tetron kick`/`admin add` could silently resolve an ambiguous or too-short peer-id prefix**: `resolve_short_id_any_network` accepted a prefix of any length and returned the first endpoint id that matched, with no check for a second match. Harmless for the additive `admin add`, but a real correctness bug for the destructive `kick`: a short or colliding prefix could silently act on the wrong member. Now rejects prefixes under 10 characters (the length `tetron status` already displays) as too short, and errors as ambiguous if more than one distinct peer matches, instead of guessing. A full endpoint id was already inherently unambiguous and needed no change.
- **`tetron leave`'s positional argument renamed `name` → `network`**: matches `invite`/`admin`, which already used `network` for the identical lookup. Pure rename, no behavior change. (`nuke`/`kick` got their own rename once their mechanism changed — see the CLI flag/positional rename entry below.)

### Changed

- **`tetron nuke`/`tetron kick` now take a network's short id, not its local name**: both previously resolved "which network" through the same mutable, locally-chosen display name used by `leave`/`invite`/`admin` — unfit as the sole identifier for a destructive, hard-to-undo action (a stale or reused local name could point at the wrong network). Both now require the network's own short id instead (a prefix of its public key, no local-name fallback at all), matching the destructive-command discipline already applied to peer identifiers. `tetron status` gained a new `id <short>` line, printed unconditionally for every network, so the short id is always available to copy. `nuke`'s "have another coordinator run..." hint and `tetron status`'s pending-nuke-proposal hint were updated to suggest the short id instead of the now-unusable local name.
- **`--hostname` defaults to the machine's own hostname, not a random name**: `create`/`join`/`up` previously fell back to a random noun (e.g. `purple-otter`) when no hostname was given — meaningless in `tetron status`/`kick`/`admin add` and unrelated to the actual machine. Now defaults to the OS hostname (sanitized: lowercased, invalid characters collapsed, truncated to 63 chars), falling back to the old random generator only if the OS hostname is unavailable or unusable. This trades a small amount of information exposure (your hostname is now visible to every peer on every network you join) for immediately meaningful roster entries; `--hostname` still overrides it for anyone who'd rather not. Explicit `--hostname` input is now also lowercased instead of hard-rejected on mixed case (`--hostname MyLaptop` previously errored; now accepted as `mylaptop`) — other invalid characters are still a hard error.
- **CLI flag/positional rename pass, several breaking renames**: the CLI had settled on no consistent vocabulary for "which network"/"what to call it" across commands. `tetron create --name` → `--network-name` (the network's published name); `tetron join --name` → `--alias` (your local-only nickname for it); `tetron nuke`/`tetron kick`'s network positional (already a short id as of the previous entry above) is now named accordingly rather than left over from when it took the local name; `tetron admin add`'s peer positional is internally consistent with `kick`'s (`peer` in both, was `identity` on `admin add`). `tetron leave`/`tetron invite`/`tetron admin` are unchanged — they already agreed on `network` for the identical local-name lookup, which is also why no command anywhere uses "alias" as a lookup key, only `join --alias` as a way to set one.
- **`tetron status --json`'s `pending_requests` field removed**: always `0` since `LIVE-001` removed the live-approval queue it used to reflect, and nothing in the CLI ever read it. Its own doc comment claimed it was "retained for D1 compat" — incorrect; `NetworkStatus` is a local daemon-to-CLI IPC structure, never sent to or received from a peer, so there was no D1 concern to retain it for.

## [0.1.6] - 2026-07-16

### Added

- **Invite-in-blob (BLOB-001)**: invites now ride in the signed `GroupBlob` instead of machine-local files (`InviteStore` superseded). Any network-key holder can mint, list, and revoke invites. The invite code drops the pinned coordinator endpoint (48 B vs 80 B) since every coordinator validates from the blob. Validation happens against the in-memory invite table; on redemption the entry is removed and the blob republished immediately. A narrow replay race window (~30-60 s DHT poll) is accepted for the initial implementation.
- **Peer address cache (CACHE-001)**: persistent transport-address cache at `<config_dir>/peercache.msgpack` so the mesh can re-establish direct QUIC connections without DHT lookups after an all-offline gap. Loaded at startup, seeded from live connections, saved every 5 min and on shutdown. Entries older than 30 days are pruned.
- **Overlap guard**: instead of the upstream rayfish preflight that refused to start if anything used `100.64.0.0/10`, tetron refuses to start only if the *chosen* subnet overlaps an existing local network. This lets tetron run alongside Tailscale or any other overlay without hijacking routing.
- **`ray kick <network> <peer>`**: coordinators can now remove a member from a
  closed network. Identify the peer by hostname, mesh IP, or short id. The member
  is dropped from the network's roster, and every node disconnects from it: the
  kicked peer is severed mesh-wide, not just from the coordinator. It cannot
  re-join the closed network without a fresh invite or approval (to bar it
  permanently, also revoke its invite or reusable key). Kicking is refused on open
  networks (where the peer could immediately re-join) and against another
  coordinator or yourself.

### Changed

- **rayfish relay/discovery presets retained (CON-001)**: the `"rayfish"` config keyword and its preset URLs (`relay.iroh.rayfish.xyz`, `dns.iroh.rayfish.xyz`) are kept as-is — they are load-bearing infrastructure references that must match upstream. The default remains n0's neutral infrastructure, but the keyword survives for users who pin to rayfish's hosted services.
- **Crate identity renamed to tetron (RENAME-M01)**: the library crate is now
  `tetron` (`[package] name = "tetron"`), the helper crate is `tetron-proto`,
  all `use rayfish::…` paths are `use tetron::…`, and the tracing filter is
  `info,tetron=debug`. Internal only — wire format is untouched (D1 preserved,
  mixed tetron/full-torpedo networks still work). The binary, service, and paths
  remain `torpedo`.
- **Hostname is fixed at join (MINIMAL-014)**: a member's hostname is set once
  when it joins (the coordinator still resolves collisions by appending
  `-1`/`-2`/…), and a member adopts that authoritative name from the signed
  roster. There is no longer a way to rename a member after join.
- **Admission is approval-only (MINIMAL-013)**: `torpedo create` always makes a
  closed network — the `--open` (and explicit `--closed`) flag is gone. A
  joiner dials the room id, lands in the pending queue, and a coordinator (or
  any co-coordinator granted with `torpedo admin add`) admits it with
  `torpedo requests` → `torpedo accept`/`deny`. This is now the only way onto a
  tetron-coordinated network.
- **Phase 5 complete**: presentation and workspace cleanup. The CLI is now
  plain text (no ANSI colors or spinners; `--json` remains for machine
  output). The Android build (`ray-mobile`, `android/`) is removed, leaving
  a single-product workspace (binary `torpedo`, library `tetron`, helper
  `tetron-proto`). The `desktop` cargo feature is retired.
- **Bounded pending-join queue** — on a closed network, the coordinator's queue
  of join requests awaiting `ray accept` is now capped (oldest request evicted
  when full), so a peer churning fresh identities can no longer grow it without
  limit. Legitimate queues are far below the cap, so this is invisible in normal
  use.
- **Admission is invite-only (LIVE-001)**: supersedes the live-approval queue described above (MINIMAL-013's `torpedo requests`/`accept`/`deny` and the bounded pending-join queue entry above) -- both are gone. A bare room-id join is now always denied; the only way onto a tetron-coordinated network is an invite key minted by a coordinator (auto-minted on `tetron create`, or via `tetron invite <net> create`), which the joiner presents to be admitted directly. This was never logged as its own changelog entry when it landed; recorded now for accuracy.

### Removed

- **Workspace trimmed (MINIMAL-016)**: removed `ray-mobile` workspace member and
  `android/` directory, trimmed `justfile` to the surviving deploy recipes, and
  removed the `desktop` cargo feature (no longer needed without the Android
  library build). Only `tor` remains as an optional feature.
- **Plain CLI (MINIMAL-015)**: removed style.rs, layout.rs, progress.rs and the
  `indicatif`/`crossterm`/`unicode-width` dependencies. CLI output is plain text
  with no colors, spinners, or interactive pickers. `--json` output is
  unaffected and remains on every read command.
- **`torpedo hostname` and `torpedo ephemeral` (MINIMAL-014)**: hostname rename
  propagation (the durable pending-rename intent and its redelivery) and the
  per-network ephemeral auto-kick TTL (auto-removing members offline longer than
  a configured duration) are gone. Remove a stale member manually with
  `torpedo kick`.
- **Invite minting (MINIMAL-013)**: `torpedo invite` and all its subcommands
  (`create`/`list`/`revoke`, `--reusable`/`--hostname`/`--expires`/`--qr`) are
  gone, along with the single-use invite ledger (`invites/<network>.toml`) and
  reusable-key minting. A tetron node can still **join** a full-torpedo network
  by an invite code or reusable key (`torpedo join <code>`), and a tetron
  coordinator still validates a reusable key that rides a full-torpedo signed
  roster — it just never mints one. Invite-share gossip from a full-torpedo
  co-coordinator is accepted on the wire and ignored (wire compatibility).
- **Magic DNS and all OS DNS mutation (MINIMAL-012)**: the `.ray` name
  resolver, the in-daemon DNS responder + port-53 intercept, and every OS-DNS
  integration (systemd-resolved / NetworkManager / resolvconf / the
  `/etc/resolv.conf` takeover, its inotify re-assert, and the panic-hook
  restore) are gone, along with the `magic-dns` and `dns-upstreams` config
  keys. **Reach peers by mesh IP** from `torpedo status` (or `--json`); host
  naming is `/etc/hosts`' job. Hostnames still ride the roster and show in
  `torpedo status`. The daemon's host footprint shrinks to: TUN device, routes,
  config dir, log dir, unix socket. The `.100.53` resolver address stays
  reserved (never assigned to a member) for wire compatibility with a
  full-torpedo node on a shared network.
- **Userspace firewall (MINIMAL-010)**: the entire `torpedo firewall` command
  (add/remove/show/default/reject/on/off, coordinator `suggest`, and the
  `pending`/`accept`/`deny`/`auto-accept` review flow), the per-device
  `firewall.toml`, the `auto_accept_firewall` per-network setting, and the
  `--auto-accept-firewall` join flag are gone. **Packet filtering is now the
  host firewall's job**: within a shared network every peer reaches every port
  a local service binds (mesh membership still gates *who* can connect).
  Restrict ports with nftables/ufw on the `torpedo` TUN interface — e.g.
  `nft add rule inet filter input iifname "torpedo" tcp dport != 22 drop`. The
  daemon still drops inbound packets whose source IP is not the sending peer's
  assigned mesh address (anti-spoofing). A coordinator running full torpedo can
  still ship firewall suggestions in the signed group blob; a tetron node
  carries them through on republish but does not act on them.
- **Declarative apply layer and local aliases (MINIMAL-011)**: `torpedo apply`
  (with `--example`/`--dry-run`/`--prune`/`--invite-missing`), `torpedo alias`,
  and `torpedo identityof` are gone, along with the per-network `aliases`
  setting shown inline in `torpedo status`. Reconcile a fleet with a script
  over `torpedo status --json`.
- **File sharing and device pairing (MINIMAL-004)**: `torpedo send`/`files`
  (and file auto-accept + download-dir/download-user settings) and
  `torpedo pair`/`unpair` (multi-device identity, encrypted key backup, and
  `--1password` backup/restore) are gone, along with device certificates, the
  `_torpedo_certgen` revocation floor, and the file/pair ALPNs. The identity
  model collapses to one device = one user. Copy files with `scp`/`rsync` over
  the mesh IPs, and back up the identity key yourself (it is one `0600` file
  under the config dir). Nodes on a shared network with a full-torpedo peer
  still decode-and-ignore its pairing/cert control messages, so the mesh keeps
  working across both variants.
- **Direct connect (MINIMAL-005)**: `torpedo connect`, `torpedo connections`,
  and `torpedo contact` (the contact-id friend-request flow) are gone, along
  with the `_torpedo_contact` DHT record and the connect ALPN. A private
  2-peer link is a normal 2-member network: create it and approve the join.
- **Observability export (MINIMAL-009)**: the Prometheus metrics endpoint on
  `:9090` and the `torpedo report` diagnostic-bundle command are gone. The
  daemon still logs to rolling files under the log directory, and traffic
  counters still appear in `torpedo status`.
- **Peripheral surfaces (MINIMAL-008)**: the `otel` cargo feature (OTLP span
  export), `torpedo open` deep links (the `torpedo://` scheme), and the
  append-only peer audit log (`audit.log`) are gone.
- **`torpedo ping` and `torpedo netcheck` (MINIMAL-006)**: the mesh echo-probe
  and endpoint-diagnostics commands are gone. Probe reachability with the
  system `ping` against a peer's mesh IP from `torpedo status`. Nodes still
  answer mesh Ping probes from full-torpedo peers.
- **mDNS local discovery (MINIMAL-007)**: the `torpedo mdns` command, the
  `mdns_enabled` setting, and LAN mDNS advertising are gone. Peer discovery is
  relays + pkarr; LAN peers still connect directly once discovered.
- **Embedded mesh SSH server (MINIMAL-003)**: `torpedo firewall ssh` and the
  in-daemon SSH server (with its userspace port-22 NAT) are gone, along with
  the russh/pty-process/uzers/socket2 dependencies. Remote shells are the host
  sshd's job: it listens on the mesh IPs like any other interface, so
  `ssh user@<mesh-ip>` keeps working with your normal keys and config.
- **Self-update (MINIMAL-002)**: the `torpedo update` and `torpedo auto-update`
  commands, the `install --auto-update` flag, the daemon's periodic update
  task, and the `auto_update` status field are gone. Upstream torpedo shipped
  this machinery disabled; tetron deletes it outright (along with the
  reqwest/rustls/self-replace/sha2/semver dependencies). Upgrade by replacing
  the binary and running `sudo torpedo restart`.

### Fixed

- **IPv4 fragmentation for QUIC datagram size limits (FRAG-001)**: when Quinn's
   `max_datagram_size()` is below the TUN MTU (1280), IP packets larger than
   ~1192 bytes were silently dropped by `send_datagram` with a "datagram too
   large" error, stalling TCP connections (SSH key exchange failed at "expecting
   SSH2_MSG_KEX_ECDH_REPLY"). The forwarder now fragments oversize IPv4 packets
   into RFC 791-compliant IP fragments (each sent as a separate QUIC datagram)
   before the receiving kernel reassembles them. IPv6 fragmentation is not yet
   implemented; oversize IPv6 packets are dropped with a warning.
- **Coordinator restart no longer orphans control listeners (ADMIN-RECONNECT-CTRL)**:
   when the coordinator connection drops and the reconnect loop establishes a new
   one, a fresh control-listener task is now spawned on the new connection.
   Previously the listener was only spawned once at initial join, so
   AdminGrant (and other control messages) arriving on the re-established
   connection were silently lost. This fixes `tetron admin add` failing to
   promote a member after the coordinator daemon restarts.
- **`--tor` flag now actually enables Tor transport (TOR-M01)**: previously the
   `--tor` flag on `torpedo create` and `torpedo join` was accepted by the CLI but
   silently ignored — the `transport` field was never threaded through the IPC
   handler, create/join functions, or persisted to config. It is now threaded end
   to end: the flag reaches `create_network`/`join_network`, is saved to the
   per-network `networks/<name>.toml`, and is restored on daemon restart
   (`restore_coordinator_network`) so Tor transport survives restarts as intended.
- **`torpedo report` and the issue templates now identify as torpedo**: the
  diagnostic bundle (`/tmp/torpedo-report-*.tgz`), its sysinfo banner, and the
  pre-filled GitHub issue title/body said `rayfish`, so every bug report
  mislabeled itself as upstream. The bug-report template also pointed at the
  wrong log directory (`/var/log/rayfish`); it now names the real path
  (`/var/log/torpedo`, or `/Library/Logs/torpedo` on macOS). The changelog
  "Full Changelog" compare link now points at the torpedo repository instead of
  upstream.
- **`ray status` peer traffic counters now line up**: the per-peer up/down
  columns were packed into a single field, so the `↓` counter drifted from row to
  row and the block did not read as a table. Up and down are now their own
  right-aligned columns, so the arrows and digits line up down the list.
- **`ray firewall add --peer` now accepts any peer identifier**: previously it
  only matched a short id / endpoint-id prefix, so the natural things to type
  (`--peer alice`, `--peer alice.homenet.ray`, `--peer 100.x.y.z`) failed with
  "unknown peer". It now resolves a hostname, mesh IPv4/IPv6, short id, full
  endpoint id, or a paired user identity, the same way `ray ping`, `ray send`,
  and `ray firewall ssh allow` already do. It also fixes a case where an
  **inbound** rule scoped to a paired (multi-device) peer never matched: the rule
  is now keyed on the peer's user identity, so `allow in ... --peer alice` covers
  every one of that user's devices (an outbound rule stays scoped to the named
  device).
- **Member network vanished when the coordinator was offline at startup**: a
  member (non-coordinator) whose daemon restarted while its coordinator was
  unreachable would silently drop the network from its running state. `ray
  status` showed "no active networks" and the node rejected inbound mesh
  connections, and it stayed that way until it happened to restart again while
  the coordinator was online (its config was never lost). Restore now registers
  the network immediately from the verified group blob it already holds, whether
  or not the coordinator answers, and hands off to the reconnect loop to dial the
  coordinator back with backoff. The network stays visible in `ray status`
  (peers show offline) and reconnects on its own when the coordinator returns. As
  a side effect, a network no longer takes ~30s to appear in `ray status` after a
  member restart.
- **Mesh SSH host-key mismatch**: enabling `ray firewall ssh on` no longer makes
  `ssh <host>.ray` fail with a "REMOTE HOST IDENTIFICATION HAS CHANGED" warning.
  The embedded SSH server now presents the machine's existing OpenSSH ed25519
  host key (discovered via `sshd -T`) instead of a separate generated key, so
  clients that already trust the host keep matching the fingerprint pinned in
  their `known_hosts`. Hosts without a usable OpenSSH key fall back to a
  generated key as before.
- **Join with a mismatched overlay subnet no longer silently breaks the data plane (SUBNET-BUG-001)**: joining a network whose subnet differed from this node's configured subnet used to succeed at the control-plane level (a mesh IP was assigned) while the TUN device was still built with the *local* subnet -- packets to the correct mesh IP arrived over QUIC but were silently dropped by the kernel, with no error anywhere. `tetron join` now rejects the join up front with a clear error telling you to `sudo tetron config set subnet <cidr> && sudo tetron restart` first.
- **A member kicked or dropped from the roster no longer becomes a silent zombie (CONVERGE-003)**: a node removed from the signed roster (kicked, or a casualty of the publish race below) kept redialing coordinators that correctly denied it, in a tight ~5-6s loop, while its own `tetron status` kept reporting a healthy, fully-connected membership indefinitely -- no ping, no ssh, no traffic actually moved, and nothing surfaced an error anywhere. The node now leaves the network locally as soon as it detects the removal: background tasks stopped, config deleted, `tetron status` reflects reality.
- **Co-coordinator admissions could be permanently lost (CONVERGE-001 / CONVERGE-005)**: when a promoted co-coordinator admitted a new member, the original coordinator's own periodic republish could win the DHT write purely by landing later -- even though its content was objectively older -- permanently burying the newer admission. An earlier read-before-write guard could tell a write was unrecognized but not whether it was actually newer, so the underlying race was only narrowed, not closed. The signed `GroupBlob` and its DHT record now carry a monotonic generation number, so every publisher and poller can tell newer from stale regardless of write order.
- **Coordinator restart no longer risks losing recently admitted members (CONVERGE-002)**: resolved as a consequence of the generation counter above -- a restarting coordinator's blob fetch is now always monotonically correct instead of occasionally landing on a stale copy.
- **A member reconnecting at boot during a DHT/relay outage no longer drops its network (CONVERGE-006)**: if pkarr resolution failed or no seed peer answered at the exact moment a member's daemon restarted, the network vanished from that daemon's runtime entirely -- no retry, invisible in `tetron status`, until a lucky restart. It now falls back to the locally persisted roster and can fully reconnect to peers using it (DHT reachability is only needed to *resolve* a peer, not to talk to it once dialed).
- **A single unreachable roster member no longer stalls a join, reconnect, or restart (DIAL-001)**: member join/reconnect and the coordinator's full-mesh restore dial were both serial and unbounded -- one dead or offline peer in the roster could block the whole operation for iroh's uncapped internal handshake timeout, and a coordinator restart's `tetron status` reported no active networks at all until every member had been dialed. Roster dials are now concurrent and timeout-bounded (10-30s), and a coordinator's network registers in `tetron status` before dialing its members, not after.
- **A kick-coded connection close no longer risks evicting a valid member (CONVERGE-007)**: closing a connection with the kick code (sent whenever any node's local roster momentarily excludes a peer, not only on a real `tetron kick`) was treated the same as a deliberate `tetron leave` for the purpose of pruning the coordinator's roster -- a transient convergence hiccup could cause a false eviction. Only a genuine leave now prunes the roster; a kick-coded close is treated as ordinary transport teardown, and reconnection is decided by the already-correct, signed-roster-driven mechanism instead of the raw close code.

### Performance

- **Drop-newest under datagram backpressure** — when a peer's QUIC datagram send
  buffer is momentarily full, the new packet is dropped at the application
  boundary instead of letting QUIC evict an older already-queued one (drop-newest
  beats drop-oldest for a VPN), and the QUIC transport is tuned for the one
  datagram stream per peer shape. Keeps the send path non-blocking with no
  cross-peer head-of-line blocking.

## [0.1.4]

### Added

- **Mesh SSH (`ray firewall ssh`)**: Tailscale-style SSH with no SSH keys to
  manage. `ray firewall ssh on` runs an embedded SSH server on this node's mesh
  IPs (port 22); `ray firewall ssh allow <network> <peer>` authorizes a peer
  (hostname, mesh IP, short id, or `*` for any peer on the network) to log in.
  Connect with a stock client: `ssh user@host.ray`. The connecting peer is
  identified by its mesh identity (already proven by the encrypted mesh link), so
  there are no `authorized_keys` to distribute. Each grant restricts which local
  unix users the peer may log in as: `ray firewall ssh allow <net> <peer>` permits
  any **non-root** user by default, `--user alice,deploy` limits it to named
  accounts, and `--user '*'` permits any user including root. The check is by uid,
  so a uid-0 account under any name is blocked unless root is explicitly granted.
  `ray firewall ssh deny` revokes a peer; `ray firewall ssh show` lists state and
  per-network allow lists with their permitted users. As a security prerequisite,
  inbound mesh packets whose source IP is not the sending peer's assigned mesh
  address are now dropped (ingress anti-spoofing), so no peer can forge another's
  mesh IP.
- **Aliases and groups in `ray apply`**: a spec can now define optional
  top-level `aliases:` (a friendly name to a user's identity string) and
  `groups:` (a name to a list of aliases and/or hostnames), then reference them
  as firewall subjects or peers instead of listing every hostname. An alias
  names a person and expands to all of that person's currently-joined devices;
  a group expands to the union of its members. Expansion happens client-side at
  apply time, so the published rules are plain per-host suggestions. Aliases
  resolve only for members that have already joined (a `note:` is printed and
  the rule skipped until they do); literal hostnames still work before a host
  joins. `ray apply --dry-run` shows the fully expanded result.
- **`ray identityof <net> <host>`**: print a host's identity string (the value
  to paste into a spec's `aliases:`). Resolves to the user identity if the
  device is paired, else the device's transport identity. `--json` supported.

### Fixed

- **Accepted firewall suggestions no longer pile up duplicates.** Any change to a
  network's signed blob (a join, a rename, a new reusable key) re-materialized the
  whole suggested-firewall set and re-queued it for review, even the rules this
  node had already accepted. Accepting one of those repeats via the picker then
  appended a second identical rule. Already-installed suggestions are now kept out
  of the pending queue, and the picker merges by selector (newest wins), so a
  re-suggested rule replaces its predecessor instead of stacking.
- **`ray update` no longer bricks the system service.** After swapping its own
  binary, `ray update` rewrote the service unit using the path of the running
  executable, which Linux reports with a trailing `" (deleted)"` once the old
  binary is unlinked. The unit ended up as `ExecStart=/usr/local/bin/ray (deleted)
  daemon`, so the daemon crash-looped with `unrecognized subcommand '(deleted)'`
  and the node went offline until a manual reinstall. The path is now sanitized,
  making remote self-update safe.

## [0.1.3]

### Added

- **Custom relay, discovery, and DNS-upstream servers (`ray config`)**: override
  the default iroh relay and discovery servers, or the upstream resolvers used for
  non-`.ray` queries, with `ray config set relay|discovery-dns|dns-upstreams
  <value>`. Values are a comma list of presets (`rayfish`/`n0`), URLs, or IPv4s;
  the default augments the n0 defaults, `--replace` swaps them out, and `n0`/empty
  resets. `ray config get`/`unset` read and clear overrides. Applied on
  `sudo ray restart`.
- **`ray ping <peer>`**: active mesh diagnostics: sends live echo probes to a
  peer (by hostname, mesh IP, or short id) and reports per-probe round-trip
  latency, packet loss, and whether the path is direct or relayed. `-c/--count`
  and `-i/--interval` tune the probe run; `--json` emits the per-probe array.
  Unlike `ray status` (a passive snapshot), this verifies the round-trip works
  end to end.
- **`ray netcheck`**: local network diagnostics: bound UDP port (and whether
  it is the fixed forwardable port or an ephemeral fallback), home relay and its
  latency, public IPv4/IPv6 addresses, and whether UDP is working. `--json`
  supported.
- **Release notes on `ray update`**: before swapping the binary (and in
  `ray update --check` when behind), print what the update brings: the stable
  channel walks every release in `(current, latest]` newest-first, while
  `--nightly`/`--version` show the resolved release's notes. Best-effort, so a
  fetch failure never blocks the update.
- **Standby control plane (`ray up`/`down`)**: `ray down` now takes only the
  data plane offline (TUN, routes, Magic DNS, inbound forward gate) while staying
  connected to peers, so the node keeps receiving roster/blob/firewall updates and
  `ray up` is near-instant with no re-dial. `sudo ray start`/`stop` remain the
  fully-offline switch.
- **Fail-fast firewall REJECT mode**: `ray firewall reject on|off` (opt-in,
  default off): a denied packet gets a TCP RST / ICMP-unreachable reply in both
  directions so the initiator fails immediately ("connection refused") instead of
  hanging. Off keeps the stealthy silent-drop posture.
- **`ray start` / `ray stop`** service commands to bring the whole daemon online
  or fully offline.
- **Comma-list firewall ports + short CLI aliases**: `--port`/`-P` takes a
  single port, a `start-end` range, or a comma list (`80,443`, `22,8000-9000`)
  expanded to one rule per item.
- **Control-plane abuse defense**: per-connection token-bucket rate limiting that
  closes sustained flooders, with a per-network debounced reconverge worker so a
  trigger burst coalesces into a single pkarr resolve + reconverge.

### Changed

- **Richer daemon log files**: the rolling daily logs (bundled by `ray report`)
  now capture `debug`-level detail for Rayfish itself while the console stays at
  `info`, so diagnostics like hostname propagation are traceable in a report
  without re-running with `RUST_LOG`. Dependency logs stay at `info`; `RUST_LOG`
  still overrides everything.
- **Additive firewall suggestions**: each suggested token becomes one allow/deny
  rule with no synthesized catch-all (allow-list relies on the node's own inbound
  default-deny; denies-only = blacklist). `ray status` ends with a `pending`
  summary of things awaiting the user.

### Fixed

- **`ray hostname` rename now reliably propagates.** A member's rename is kept as
  a durable pending intent and re-delivered to a coordinator on every reconnect
  and reconverge until the signed roster confirms it, so the new name reaches the
  coordinator and all peers instead of sticking only on the renamed node. The
  renamed node keeps showing its new name across reconverges rather than briefly
  reverting to the old one.
- **`ray status` no longer shows `?` for a live connection's path.** A connection
  that is up but whose path iroh hasn't marked "selected" yet (during holepunch or
  migration) now reports its actual `direct`/`relay`/`tor` path instead of `?`.
- **`ray status` no longer glues a network's `join <room-id>` onto the last peer
  row.** The room-id line now prints on its own line.
- Publish the contact record regardless of data-plane state, so `ray connect`
  resolves a peer that is on standby (`ray down`).

## [0.1.2]

### Changed

- **Magic DNS reworked to TUN interception**: `.ray` queries are intercepted in
  the TUN read loop and answered in-daemon via the magic IP `100.100.100.53`, so
  the resolver never binds the host's port 53. Non-`.ray` queries forward to the
  captured upstreams.
- **Direct-mode DNS takeover (Tailscale-style)**: on hosts without split-DNS,
  take over `/etc/resolv.conf` with an inotify re-assert loop that repairs it in
  ~ms when NetworkManager/dhclient overwrites it, plus a `dns=none` NM drop-in so
  NM stops regenerating it. Both are marker-guarded and crash-safe (panic hook +
  next-start cleanup restore the host's DNS).
- **Sharded, atomic per-network config**: globals in `settings.toml`, each
  network in `networks/<name>.toml`, all written via temp-file + atomic rename.
  Replaces the single `networks.toml` whose non-atomic rewrites raced and silently
  dropped networks; legacy files auto-migrate on first load.
- Retain only the 7 most recent daily log files.
- Authenticate GitHub API calls in `ray update` with a `gh` token (lifts the
  anonymous rate limit).

### Fixed

- Scope suggested firewall rules to non-joined networks correctly, and default a
  suggestion's peer to "any" so rules propagate instantly.
- Point systemd-resolved (`SetLinkDNS`) at the magic IP; fix the NetworkManager
  mode read on Linux.

## [0.1.1]

### Added

- **Direct connections (`ray connect`)**: link two peers with no shared room id
  or invite via a rotatable, published **contact id**. `ray connect <contact-id>`
  sends a friend request; `ray connections [approve <id>]` reviews and admits it,
  minting a 2-peer network with the requester pre-approved. `ray contact
  [id|rotate]` prints or rotates the contact key.
- **Reusable invite keys**: `ray invite <net> --reusable [--expires]` mints a
  multi-use, expiring key that rides the signed `GroupBlob`, for unattended
  fleets (`ray join <key> --hostname H --auto-accept-firewall`). Revocation
  propagates via the blob.
- **Cross-coordinator invite gossip**: single-use invites are gossiped
  (`InviteShare`/`InviteUsed`) so any coordinator can validate and burn a
  cross-minted invite; combined with dial-fallback across the published
  coordinator set, fresh joins survive any single coordinator being offline.
- **Self-update (`ray update`)**: update from GitHub releases with SHA-256
  verification and atomic binary swap; `--check`, `--list`, `--force`,
  `--nightly` (rolling pre-release), and `--version V` (pinned, downgrades
  allowed). `ray version` / `--version` print the compiled version + git SHA.
- **Stable listen port**: the shared endpoint binds a fixed UDP port (41383) so
  it survives restarts and can be manually port-forwarded for guaranteed direct
  reachability, falling back to an ephemeral port if the port is in use.
- **CLI polish**: ANSI-aligned tables, progress spinners, an interactive
  `ray firewall pending` picker, and a global `--json` flag for machine-readable
  output.
- **Per-node firewall auto-accept**: `ray join --auto-accept-firewall` /
  `ray firewall auto-accept <net> on|off` to auto-install suggested rules.
- **IPv4 collision handling**: per-member `collision_index` with `assign_ip`
  rotation, index-aware validation, duplicate-IP rejection, and a deterministic
  reconverge tiebreak.
- **Opt-in QR invites**: `ray invite --qr` prints a scannable code.

### Changed

- **Secure-by-default inbound firewall**: unsolicited inbound TCP/UDP is now
  denied by default (inbound ICMP allowed, outbound allowed), with a stateful
  conntrack letting return traffic pass. `ray firewall default allow|deny` flips
  the inbound default.
- **Removed `trusted` networks** in favor of per-device, per-network firewall
  auto-accept; coordinators suggest rules on any network and nodes consent
  per-node (auto-accept or manual `ray firewall accept`/`deny`).
- **`ray apply` is YAML-only** (previously YAML/TOML/JSON), with each network
  mapping directly to its firewall subjects.
- **Mesh ALPN is versioned as the protocol-compatibility gate**: peers on
  different mesh versions share no common ALPN and can't connect. `ray join`
  pre-checks the coordinator's signed mesh version and dials surface an
  incompatible-version hint suggesting `ray update`.
- Roster and firewall state reconverge from the network-key-signed pkarr record,
  not from peer control messages (which are payload-free triggers).

### Fixed

- **ICMP conntrack** is now echo-type-aware, closing an inbound leak where reply
  packets could be treated as solicited.
- macOS routing: assert the IPv4 `100.64.0.0/10` route on activate, and install
  a loopback self-route so you can ping your own `*.ray` IP.
- Flush control-protocol QUIC streams and the pairing device-cert response so
  messages always reach the peer before the connection drops.
- `AdminGrant` keys are self-authenticated against the network public key.

### Performance

- Zero-copy TUN read and datagram forwarding path, with Criterion microbenchmarks
  (`benches/forward.rs`) over the per-packet data path.

## [0.1.0]

First public release.

### Added

- **P2P mesh VPN** over [iroh](https://iroh.computer): peers connect by
  cryptographic identity (EndpointId), not IP. NAT traversal, hole-punching, and
  end-to-end encryption are handled by iroh, with encrypted relay fallback.
- **Dual-stack addressing** derived from identity: stable IPv4 in `100.64.0.0/10`
  (FNV-1a) and stable IPv6 in `200::/7` (blake3, 120-bit, never rotates).
- **Networks & access modes**: closed by default; `--open` for public networks.
  Closed networks admit via one-time **invite codes** (`ray invite`) or **live
  approval** (`ray requests` / `ray accept` / `ray deny`). The room id is a
  discovery key, never an admission credential.
- **Coordinator / membership model**: single signed `GroupBlob` per network
  published to a per-network pkarr record; gatekeeper admission, member roster,
  and `MemberApproved` broadcast so the coordinator need not be online for a
  member's later reconnects.
- **Co-coordinators**: `ray admin add` grants the network key over the
  authenticated mesh, enabling multiple machines to publish the signed blob.
- **Magic DNS**: reach peers at `name.network.ray` (A/AAAA/PTR/SOA), rebuilt
  from the roster on every membership change.
- **Per-device firewall**: directional, protocol-, port-, and network-scoped
  rules with a stateful conntrack; `firewall.toml`.
- **Trusted networks**: coordinators can suggest firewall rules that ride the
  signed blob; nodes auto-take (`--allow-trusted`) or queue them for manual
  `ray firewall accept` / `deny`.
- **Declarative provisioning**: `ray apply <spec>` reconciles trusted networks +
  suggested firewalls from a YAML/TOML/JSON spec, with `--prune`, `--dry-run`,
  `--invite-missing`, and `--example`.
- **Multi-device identity**: `ray pair` (ticket-based), plus encrypted
  backup/restore, including optional 1Password storage of the encrypted blob via
  the `op` CLI (`ray pair backup --1password` / `ray pair restore --1password`).
- **File sharing**: `ray send` / `ray files accept` over iroh-blobs.
- **mDNS local discovery** (`ray mdns on|off`, default on).
- **Service management**: `ray up`/`down`, `ray install`/`restart`/`uninstall`,
  and the Tailscale-style operator model (`ray set-operator`).
- **Audit log**: append-only peer connect/disconnect events at
  `~/.config/rayfish/audit.log`.
- **Diagnostics**: Prometheus metrics on `:9090`, rolling daily logs, and
  `ray report` to bundle logs + metrics + sanitized status.
- **Optional transports / export**: `--features tor` (Tor transport) and
  `--features otel` (OTLP span export).

[Unreleased]: https://github.com/ErikAllanKincaid/tetron/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/ErikAllanKincaid/tetron/compare/v0.1.4...v0.1.6
[0.1.4]: https://github.com/rayfish/rayfish/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/rayfish/rayfish/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/rayfish/rayfish/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/rayfish/rayfish/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rayfish/rayfish/releases/tag/v0.1.0
