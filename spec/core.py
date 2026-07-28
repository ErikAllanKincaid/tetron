from libspec import Requirement, Constraint, UserStory

class ForkIntent(UserStory):
    """USER-STORY: FORK-INTENT

    Fork rayfish so its overlay IPv4 subnet is configurable at network-creation
    time, instead of hardcoded to 100.64.0.0/10, so it can run alongside an
    already-active Tailscale client on the same host.

    Priority: high.
    User journey: create a network with a custom --subnet -> join it from a
    second machine also running Tailscale -> both machines reach each other over
    the fork's mesh while Tailscale keeps working unaffected on both.
    Acceptance: `torpedo create --subnet <cidr>` succeeds on a host with an
    active Tailscale client; a second host joins successfully; `torpedo status`
    on both shows a live peer; Tailscale connectivity is unaffected throughout.
    """
    brief_title = "Configurable overlay subnet"
    priority = "high"


# ==========================================================================
# tetron: the minimal variant (MINIMAL-*, CON-M*)
#
# This repository is tetron, a stripped-down P2P mesh VPN. The original
# docs/PROPOSAL.md (rationale/design decisions) and docs/PLAN.md
# (commit-by-commit execution order) were retired 2026-07-27, migration long
# complete; see AGENTS.md for the current canonical description. Inherited
# SUBNET-*/RENAME-*/CON-* specs above remain
# valid until a MINIMAL removal commit retires them explicitly. New
# constraints use the CON-M* namespace so future full-torpedo CON-0xx
# numbers never collide on cherry-pick.
# ==========================================================================


class MinimalIntent(UserStory):
    """USER-STORY: MINIMAL-INTENT

    Strip torpedo to a single-purpose tool that connects machines into a
    private mesh network, delegating firewalling, name resolution, file
    transfer, remote shells, and updates to the host tools that already do
    those jobs well, and rename the product identity to tetron.

    Priority: high.
    User journey: install tetron on two machines -> create a network on
    one -> join from the other -> approve the join -> reach the peer by its
    mesh IP from `torpedo status` -> filter traffic with nftables on the TUN
    interface if desired.
    Acceptance: the CLI exposes exactly the surface in docs/PROPOSAL.md; the main
    crate is roughly 15k lines; a tetron node and a full torpedo node
    interoperate on one network; the trimmed e2e harness is green.
    """
    brief_title = "Minimal connect-only variant"
    priority = "high"


# --------------------------------------------------------------------------
# Requirements: scope and removals (MINIMAL-*)
# --------------------------------------------------------------------------

class MinimalScope(Requirement):
    """REQUIREMENT-ID: MINIMAL-001

    tetron provides exactly: identity, membership, mesh transport, TUN
    forwarding, closed-network admission with live approval, and a plain CLI
    (create/join/leave/nuke/requests/accept/deny/admin/kick/status/up/down/
    config/completions/version plus the sudo service verbs). Policy
    enforcement, naming, file transfer, remote shells, diagnostics probes,
    self-update, and multi-device identity are out of scope. Wire
    compatibility with full torpedo was preserved until RENAME-M02 severed
    it by changing the ALPN prefix; prior to that commit, protocol version 1
    and unchanged ALPNs allowed mixed networks.
    """
    req_id = "MINIMAL-001"


class RemoveSelfUpdate(Requirement):
    """REQUIREMENT-ID: MINIMAL-002

    Remove the self-update machinery entirely: src/update.rs,
    src/cli/update.rs, the `update`/`auto-update` CLI and the
    `install --auto-update` flag, and the deps it alone pulls (reqwest, the
    direct rustls handle, self-replace, sha2, semver). Full torpedo already
    ships it disabled (CON-006); in tetron absence replaces the gate,
    so CON-006 and reconcile.py's `self_update` value check retire in the
    same commit (replaced by the CON-M01 dependency-absence gate).
    """
    req_id = "MINIMAL-002"


class RemoveEmbeddedSsh(Requirement):
    """REQUIREMENT-ID: MINIMAL-003

    Remove the embedded mesh SSH server: src/ssh.rs, the userspace
    22<->30022 NAT in src/forward.rs, the `firewall ssh` CLI surface, the
    ssh_enabled/ssh_allow config keys, deps russh/pty-process/uzers, and
    tests/e2e/ssh. Remote shells are the host sshd's job, reached over the
    mesh IPs.
    """
    req_id = "MINIMAL-003"


class RemoveFilesAndPairing(Requirement):
    """REQUIREMENT-ID: MINIMAL-004

    Remove file transfer and multi-device pairing: daemon/mesh/files.rs,
    daemon/file_service.rs, cli/files.rs, cli/pair.rs, onepassword.rs,
    revocation.rs, the FILES_ALPN/PAIR_ALPN accept arms, the _torpedo_certgen
    pkarr record, and DeviceUserMap (identity model collapses to one device =
    one user). iroh-blobs STAYS: it transports the signed GroupBlob
    (reconverge.rs fetches it by hash over the blobs ALPN) and is core
    infrastructure, not a file-sharing extra. File copying is scp/rsync's
    job; key backup is the operator's job (the key is one file).
    """
    req_id = "MINIMAL-004"


class RemoveDirectConnect(Requirement):
    """REQUIREMENT-ID: MINIMAL-005

    Remove the direct-connect (friend request) flow: daemon/connect_service.rs,
    daemon/mesh/connect.rs, cli/connect.rs, CONNECT_ALPN, the _torpedo_contact
    pkarr publisher, and contact_secret_key. A 2-peer link is a 2-member
    network created and approved the normal way.
    """
    req_id = "MINIMAL-005"


class RemoveDiagnostics(Requirement):
    """REQUIREMENT-ID: MINIMAL-006

    Remove `torpedo ping` and `torpedo netcheck` plus
    daemon/mesh/diagnostics.rs. Reachability probing is ping/mtr's job over
    the mesh IPs. For wire compat (D1) a min node keeps a passive
    ControlMsg::Ping -> Pong responder so probes from full nodes still work.
    """
    req_id = "MINIMAL-006"


class RemoveMdns(Requirement):
    """REQUIREMENT-ID: MINIMAL-007

    Remove mDNS local discovery: spawn_mdns_discovery, the `torpedo mdns`
    CLI, the mdns_enabled config key, and the iroh-mdns-address-lookup dep.
    Discovery is relays + pkarr.
    """
    req_id = "MINIMAL-007"


class RemovePeripherals(Requirement):
    """REQUIREMENT-ID: MINIMAL-008

    Remove peripheral surfaces: the `otel` cargo feature and its optional
    deps, deep links (deeplink.rs, cli/open.rs, the torpedo:// scheme), and
    the audit log (audit.rs).

    The `tor` cargo feature is explicitly KEPT (see TOR-M01 for why and for
    the flexible per-network policy roadmap): Tor carries only TCP streams,
    so an iroh QUIC/UDP mesh can not be torified externally (torsocks,
    TransPort redirection, and gateway setups all drop UDP); the in-endpoint
    iroh-tor-transport glue is the only working integration, and it already
    delegates onion routing to the system Tor daemon (ControlPort 9051).
    It stays compile-time gated and off by default, so default builds carry
    zero Tor code. The existing per-network `--tor` flag and its semantics
    (endpoint-wide additive transport, effective after daemon restart) are
    kept unchanged through the MINIMAL phases.
    """
    req_id = "MINIMAL-008"


class RemoveObservabilityExport(Requirement):
    """REQUIREMENT-ID: MINIMAL-009

    Remove the observability export surface: the stats.rs Prometheus
    exporter on :9090 and `torpedo report` (build_report, the .tgz bundle,
    the pre-filled GitHub issue). Per-peer counters that status display or
    forward.rs logging still need are kept as plain fields. Logs stay
    (logdir.rs, rolling files); shipping them anywhere is out of scope.
    """
    req_id = "MINIMAL-009"


class RemoveFirewall(Requirement):
    """REQUIREMENT-ID: MINIMAL-010

    Remove the userspace firewall: firewall.rs, cli/firewall.rs,
    daemon/mesh/firewall.rs, reject.rs, picker.rs, firewall.toml, the
    auto_accept_firewall config key, the firewall benches, and
    tests/e2e/firewall. forward.rs keeps only the upstream anti-spoof
    ingress check. The IP-header parser the forwarder still needs
    (PacketInfo/parse_packet_info, for peer routing, anti-spoof, and the
    port-53 Magic-DNS intercept) is relocated out of firewall.rs into a new
    neutral src/packet.rs — it is packet parsing, not firewall logic.
    Packet filtering is nftables/ufw's job on the TUN interface; README
    states the posture change (every mesh peer reaches every port) loudly,
    with the nftables equivalent. Wire compat (D1): GroupBlob keeps its
    suggested_firewall field; reconverge ignores it and coordinator republish
    preserves it verbatim; ray-proto policy.rs/firewall.rs wire types stay.

    **Follow-up, 2026-07-17:** `GroupBlob.suggested_firewall` and
    `tetron-proto`'s `policy.rs` (`SuggestedFirewall`/`HostSuggestions`) were
    kept at the time for D1 wire compat -- a full-torpedo coordinator's
    suggestions carried through the blob verbatim, never acted on. RENAME-M02
    subsequently severed D1 (see that requirement's own addendum), which
    initially got this pair classified as "lower-confidence, lower-urgency"
    to remove (their justification rested on a weaker, contrived
    cross-product-key-migration scenario rather than RENAME-M02's flat ALPN
    impossibility). On reflection that distinction didn't hold up: the
    feature these fields served was already fully gone by this requirement,
    so neither one did anything in tetron regardless of D1 -- keeping them
    only added a wire-format field and a whole crate module for no purpose
    tetron itself has. Removed as part of the same follow-up pass as
    RENAME-M02's D1 cleanup: `GroupBlob.suggested_firewall` (and its
    threading through `canonical_group_bytes`/`group_blob_hash`,
    `NetworkState`, `JoinParams`, `RestoredRoster`, restore/reconverge
    adoption), plus `tetron-proto/src/policy.rs` in its entirety (deleted --
    nothing else in the workspace consumed `SuggestedFirewall`/
    `HostSuggestions`) and its `lib.rs` re-export.

    `tetron-proto/src/firewall.rs` (`Action`/`Direction`/`Protocol` enums)
    was audited at the same time and found to be a *separate*, independently
    dead remnant of this same requirement -- its own doc comment names the
    firewall IPC types (`FirewallState`, `FirewallRuleView`, `FirewallAdd`,
    `FirewallDefault`) this requirement already removed, and `policy.rs`
    never actually imported from it (`HostSuggestions.allows` used raw
    strings, not these enums). Flagged but deliberately not removed in this
    pass, matching the same scope discipline applied to `membership.rs`'s
    already-`#[allow(dead_code)]`-marked `policy_for_mode`/`OpenPolicy` --
    both are pre-existing, unrelated dead code discovered as a side effect,
    not part of what was being cleaned up.
    """
    req_id = "MINIMAL-010"


class RemoveApplyLayer(Requirement):
    """REQUIREMENT-ID: MINIMAL-011

    Remove the declarative apply layer (which exists to push firewall specs
    and dies with MINIMAL-010): apply.rs, cli/alias.rs, daemon/mesh/alias.rs,
    the `torpedo apply` / `torpedo alias` / `torpedo identityof` CLI (and their
    orchestrators, previously co-located in cli/firewall.rs), EXAMPLE_SPEC, the
    `Alias{Set,Remove,List,ListResponse}` IPC ops, the per-network `aliases`
    config field + its `NetworkStatus.aliases` projection + the inline
    `[alias]` status display, and the tests/e2e/apply scenario. Fleet
    reconciliation is a script over `torpedo status --json`.

    Sequencing (see PROPOSAL/PLAN): this lands BEFORE MINIMAL-010 even though
    the numeric order is the reverse. `apply`/`identityof` code lived in
    cli/firewall.rs and consumed the firewall-suggest IPC, so removing the
    consumer first keeps every commit compiling AND behaviorally coherent (the
    firewall is still fully present after this commit; a broken intermediate is
    avoided). The GroupBlob `suggested_firewall` field and ray-proto
    policy.rs/firewall.rs wire types are untouched here (D1).

    Follow-up (D-01, 2026-07-23): the removed `apply.rs` was the sole consumer
    of the external `config` crate (Cargo.toml's `config = { version = "0.15",
    ... }`, not this crate's own `src/config.rs` module) — confirmed via a full
    workspace search (no `use config::...`, `config::Config::builder()`, or any
    other symbol from the external crate anywhere in `src/`/`tetron-proto/src`,
    and `Cargo.lock` shows it resolved only as a direct dependency of the root
    package). Removed from `Cargo.toml`; added to `CON-M01`'s banned-dependency
    list (`reconcile.py`) so it cannot silently creep back in.
    """
    req_id = "MINIMAL-011"


class RemoveMagicDns(Requirement):
    """REQUIREMENT-ID: MINIMAL-012

    Remove Magic DNS and all OS DNS mutation: dns.rs, dns_config.rs,
    dns_resolver.rs, dns_packet.rs, daemon/dns_manager.rs, the port-53
    intercept in forward.rs, the magic-dns/dns-upstreams config keys, deps
    zbus/inotify, the panic-hook resolv.conf restore, and tests/e2e/dns.
    Peers are reached by mesh IP from `torpedo status`; naming is
    /etc/hosts' job (or a script over `status --json`). Hostnames remain in
    the roster (wire compat, status display). The daemon's host footprint
    shrinks to: TUN device, routes, config dir, log dir, unix socket.

    **Follow-up, 2026-07-17:** at the time this shipped, `membership::
    magic_dns_v4(subnet)` and the `is_reserved_ipv4` check that kept it out
    of the member IP pool were deliberately retained for D1 wire compat (a
    full-torpedo node on a shared network routes that address to its own
    resolver). RENAME-M02 subsequently severed D1 -- see that requirement's
    own addendum -- making the reservation's justification moot the same way
    it made the other D1-compat branches moot. Removed as part of the same
    cleanup pass: `magic_dns_v4`, `is_reserved_ipv4`, the skip-logic in
    `assign_ip`, and the reserved-IP check in `validate_member`, plus their
    three dedicated tests. `assign_ip` now only avoids IPs already held by a
    different member; `validate_member` only checks the CGNAT range and the
    network/gateway reservations.

    **Follow-up, 2026-07-17:** a CLI doc-comment-vs-handler audit found three
    remaining `--help` references to the removed feature: `Create.hostname`
    and `Join.hostname` both illustrated the hostname example as `"alice" ->
    alice.gaming.ray` (a Magic DNS `.ray`-domain label that has not existed
    since this requirement shipped); `Down`'s doc comment still said "take the
    data plane (TUN + Magic DNS) offline." Fixed all three (`main.rs`). Also
    found the same dead pattern in code, not just help text:
    `resolve_peer_name` (`runtime.rs`) still split its argument on `.` to
    accept a bare-or-qualified `alice.net.ray` hostname; since valid hostnames
    can never contain a `.` (`is_valid_hostname` is letters/digits/hyphens
    only), the split was permanently a no-op. Removed alongside the doc-
    comment fix documented in ADMIN-ADD-EASY-ID's own addendum.
    """
    req_id = "MINIMAL-012"


class ApprovalOnlyAdmission(Requirement):
    """REQUIREMENT-ID: MINIMAL-013  [PARTIALLY SUPERSEDED]

    NOTE 2026-07-14: The invite-removal part of MINIMAL-013 was applied
    (commit history shows the invite-free period) and then REVERSED when
    invite keys were brought back as the primary enrollment method. The
    INVITE-* requirements below document the restored invite system. The
    parts of MINIMAL-013 that still hold:
      - `tetron create` always makes a Restricted network
        (`--open`/`--closed` removed from CLI).
      - `GroupMode::Open` is still understood for D1 compat (auto-admit on
        full-tetron open networks), but tetron never creates one.
      - Joiner-side invite-code redemption (decoding an invite minted by a
        full-tetron coordinator) still works unchanged.
      - Reusable-key validation in membership.rs is kept as D1 compat.
      - `InviteShare`/`InviteUsed` from full co-coordinators are decoded
        and ignored on receipt (D1 compat).

    What was REMOVED and stayed removed:
      - `--open`/`--closed` flags on `tetron create`.
      - Reusable-key minting (validation-only survives).

    What was APPLIED and then REVERSED (invites are now fully present):
      - The single-use invite store (InviteStore, TOML files).
      - `tetron invite` create/list/revoke CLI.
      - InviteCreate/InviteList/InviteRevoke IPC ops.
      - `invite_create`/`invite_list`/`invite_revoke` daemon handlers.
      - The per-network `invite_lock` mutex was restored in the accept/join
        machinery.
      - The `initial_invite_key` auto-mint on create.
      - `redeem_invite_and_admit` as the primary admission gate.

    See INVITE-001 through INVITE-008 for the current design.
    """
    req_id = "MINIMAL-013"


class FixedHostnameNoEphemeral(Requirement):
    """REQUIREMENT-ID: MINIMAL-014

    Remove hostname rename propagation and the ephemeral auto-kick TTL.
    Deleted: the `torpedo hostname`/`torpedo ephemeral` CLI, the
    `SetHostname`/`SetEphemeral`/`GetEphemeral`(+`EphemeralStatus`) IPC ops,
    `MeshManager::set_hostname`/`announce_rename_to_peers`/`set_ephemeral`/
    `get_ephemeral`, the whole `src/daemon/mesh/rename.rs` (`pending_hostname`
    drain, `rename_satisfied`, `has_pending_hostname`),
    `spawn_stale_member_pruner`/`should_prune`, the `pending_hostname` and
    `ephemeral_ttl_secs` `NetworkConfig` fields, the status `ephemeral_ttl_secs`
    field + its status-line render, and the reconverge worker's 30s
    rename-backstop tick (now purely trigger-driven).

    Hostname is fixed at join: it is set once from the joiner's
    `JoinRequest`/`MeshHello`, the coordinator still resolves collisions
    authoritatively at admission (`admit_peer` -> `resolve_collision`), and a
    member adopts that authoritative name from the signed roster on reconverge
    via the trimmed `reconcile_local_hostname` (now adopt-blob-name only). The
    coordinator control reader no longer acts on a `MeshHello` hostname but
    still captures a full-torpedo peer's `device_cert` off it (D1).
    `outgoing_hostname` (announce the fixed name on reconnect) survives, moved
    from the deleted rename.rs into join.rs. `reconverge_and_apply` keeps its
    now-unused `alpn`/`my_ip` params (prefixed `_`) for call-site stability with
    torpedo. Manual `kick` remains the remediation tool for stale members.
    """
    req_id = "MINIMAL-014"


class PlainCliPresentation(Requirement):
    """REQUIREMENT-ID: MINIMAL-015

    Plain-text CLI output: remove style.rs, layout.rs, progress.rs and deps
    indicatif/crossterm/unicode-width/humansize/mime_guess. `--json` stays
    on every read command (the composable Unix interface). No colors,
    spinners, glyphs, or interactive pickers.
    """
    req_id = "MINIMAL-015"


class WorkspaceTrim(Requirement):
    """REQUIREMENT-ID: MINIMAL-016

    Trim the workspace to the one product: remove the ray-mobile member and
    android/ (the Android build reuses subsystems MINIMAL removes), reduce
    benches/ to the surviving forward path, prune cargo features to the
    default set, and sweep justfile/cliff.toml targets that reference
    removed surfaces.

    **Follow-up, 2026-07-17:** the ray-mobile *member crate* was removed, but
    15 doc comments across 6 files (`daemon/mod.rs`, `daemon/mesh/runtime.rs`,
    `daemon/mesh/bootstrap.rs`, `daemon/mesh/create_join.rs`,
    `daemon/mesh/diagnostics.rs`, `config.rs`) still named `ray-mobile` as a
    current consumer of the embedding API (`MeshManager::activate`/
    `attach_tun`/`detach_tun`/`shutdown_and_close`/`create_network`/
    `join_network`/`status`, the `DaemonState` legacy alias, and the
    `TETRON_CONFIG_DIR` Android override). The embedding API itself is not
    dead -- the `#[cfg(not(target_os = "android"))]` gates it exists for are
    still live, compiled code -- only the specific named example is gone.
    Reworded all 15 to describe the embedding API generically ("Part of the
    embedding API", "an embedder", "a mobile embedder") instead of citing a
    member crate that no longer exists in this workspace. Also fixed two
    unrelated staleness bits found in the same pass: `MeshManager::activate`'s
    doc comment still said it "configure[s] system DNS" / "configure[s] Magic
    DNS" (removed by `MINIMAL-012`; `activate`'s body has had zero DNS-related
    code since), and `bootstrap.rs`'s module doc said `handle_ipc_client`
    answers "`ray` CLI requests" (the binary is `tetron`; this particular
    phrasing didn't match `CON-010`'s `cli_reference_identity` regex because
    the character after "ray " was uppercase, so the automated gate never
    caught it).
    """
    req_id = "MINIMAL-016"


class TorPerNetworkPolicy(Requirement):
    """REQUIREMENT-ID: TOR-M01  (post-MINIMAL, deferred)

    Flexible per-network Tor routing, as a per-network transport policy in
    networks/<name>.toml with three tiers of increasing isolation and cost:

    - `any` (default): clearnet UDP with relay fallback; current behavior.
    - `tor` (what `--tor` maps to today): the shared endpoint gains the Tor
      custom transport and dials for this network prefer onion addresses.
      Traffic-level Tor only: the shared endpoint still publishes clearnet
      addresses under the same endpoint id for its other networks, so a peer
      in the tor network can resolve our id to a real IP. This tier is
      censorship resistance / reachability, NOT anonymity, and the docs must
      say so.
    - `tor-isolated` (the new work): networks with this policy live on a
      SECOND iroh endpoint owned by the same daemon, with its own secret key
      (hence its own mesh identity and derived IPs), RelayMode disabled, no
      UDP address publishing, and onion-only discovery via the tor
      transport's address lookup. No clearnet address is ever published for
      that identity; this is the only leak-free per-network Tor. All
      tor-isolated networks share the one tor endpoint/identity (linkage
      among them is accepted and documented). MeshManager routes per-network
      ALPNs to the owning endpoint; the TUN stays shared.

    Deferred until after Phase 6: tier 3 touches bootstrap, MeshManager,
    create/join, and status, and must not ride along with the removal
    phases. Tiers 1-2 already exist upstream and are kept by MINIMAL-008.
    Policy is node-local routing, never a blob/protocol change (D1 was
    severed by RENAME-M02, but routing policy is inherently local).
    """
    req_id = "TOR-M01"


# --------------------------------------------------------------------------
# Invite-key admission (INVITE-*)
#
# MINIMAL-013 originally removed invite minting (approval-only admission).
# That removal was applied (committed) and then REVERSED: invite keys are
# restored as the primary enrollment method. The room id is discovery-only;
# an invite key is required to join (with the pending-queue fallback still
# present but secondary). See INVITE-007 for the current admission priority
# and the planned removal of the live-approval fallback.
#
# Reversal history: the INVITE-* requirements were applied on top of the
# invite-free state, restoring the InviteStore, invite CLI/IPC/daemon
# handlers, initial_invite_key on create, and redeem_invite_and_admit.
# The MINIMAL-013 requirement class above is marked PARTIALLY SUPERSEDED.
# --------------------------------------------------------------------------

class InviteKeyIntent(UserStory):
    """USER-STORY: INVITE-INTENT

    Replace live-approval admission with single-use invite keys as the
    primary way onto a network. A coordinator mints an invite key (a
    printable string), shares it out-of-band with whoever should join, and
    the bearer is auto-admitted on presentation -- no approval queue, no
    coordinator attendance required beyond minting.

    Priority: high.
    User journey: create a network -> mint an invite key -> share it with a
    collaborator -> they run `tetron join <key>` and connect immediately.
    Acceptance: `tetron invite <net> create` prints a usable key; joining
    with it succeeds without `tetron accept`; the invite is single-use
    (re-joining with the same key is denied). `tetron join <room-id>` alone
    fails with a message telling the user to obtain an invite key.
    """
    brief_title = "Single-use invite key admission"
    priority = "high"


# --------------------------------------------------------------------------
# Laptop fleet: making tetron work without an always-on member
#
# The three laptop fleet changes (CACHE-001, BLOB-001, COORD-001) let a
# network of laptop users who come and go operate reliably without an
# always-on member. The two-tier model (coordinator / member) is sufficient;
# no new roles are added.
#
# Implementation order:
#   1. CACHE-001 (peer address cache) -- standalone, quick win
#   2. BLOB-001 (invite in blob) -- core change, enables cross-machine invites
#   3. COORD-001 (multi-coordinator docs) -- already works, just document
# --------------------------------------------------------------------------

class LaptopFleetIntent(UserStory):
    """USER-STORY: LAPTOP-FLEET-INTENT

    Make tetron work for a network of laptop users who come and go with no
    always-on member. A member should be able to rejoin after an all-offline
    gap, join a network using an invite minted from a machine that is now
    asleep, and kick a departed member when the network creator is offline.

    Priority: high.
    User journey: Alice creates a network, mints an invite, grants Bob the
    network key via admin add. Everyone goes home for the night. Next morning
    Bob comes online first, can admit Carol (who has an invite from Alice)
    because the invite is in the blob, can reconnect without DHT because
    peers are cached, and can kick a stale member.
    Acceptance: `tetron join <invite>` works when the minting coordinator is
    offline but another coordinator is online; `tetron status` shows peers
    immediately after an all-offline restart; `tetron kick` works when any
    coordinator is online.
    """
    brief_title = "Laptop fleet operation"
    priority = "high"
