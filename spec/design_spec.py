# spec/design_spec.py
#
# Specification for the `torpedo` fork of rayfish: make the overlay IPv4 subnet
# configurable at network-creation time instead of hardcoded to 100.64.0.0/10,
# and rebrand this fork's own identity (binary/service/paths/ALPN) away from
# rayfish so its traffic can never be confused with genuine rayfish traffic.
#
# libspec v9 note: each class's OWN docstring is what gets compiled into a
# stored component (base-class Jinja templates such as {{req_id}} are not
# inherited into a subclass docstring, because Python docstrings do not
# inherit). So each requirement/constraint ID is embedded literally in the
# first line of its docstring to stay visible and code-cross-referenceable,
# while req_id/constraint_id/enforcement_logic are also kept as class
# attributes for programmatic access (e.g. reconcile.py documentation).
from libspec import Requirement, Constraint, UserStory


# --------------------------------------------------------------------------
# User story: the intent behind the fork
# --------------------------------------------------------------------------

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


# --------------------------------------------------------------------------
# Requirements: subnet configurability (SUBNET-*)
# --------------------------------------------------------------------------

class SubnetField(Requirement):
    """REQUIREMENT-ID: SUBNET-001

    GroupBlob (src/membership.rs) gains `subnet: Option<(Ipv4Addr, u8)>`,
    following the existing `name: Option<String>` field's serde pattern
    (#[serde(default, skip_serializing_if = "Option::is_none")]). This is the
    network-wide signed source of truth every peer derives addresses against.
    """
    req_id = "SUBNET-001"


class SubnetCliFlag(Requirement):
    """REQUIREMENT-ID: SUBNET-002

    `torpedo create` gains `--subnet <CIDR>` (parsed to Ipv4Addr + prefix len).
    Omitting it falls back to the built-in default subnet (see SUBNET-011). The
    no-flag path keeps working; only the default value changes.
    """
    req_id = "SUBNET-002"


class DeriveIpParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-003

    derive_ip_with_index() (src/membership.rs) takes the network's subnet as
    a parameter instead of the hardcoded 0x6440_0000 base and fixed 22-bit host
    mask. Host-bit width is computed as 32 - prefix_len at call time. The mask
    computation, the netmask (SUBNET-005), and the gateway must all agree on
    the same prefix length or peers derive inconsistent addresses.
    """
    req_id = "SUBNET-003"


class RangeValidationParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-004

    ensure_in_cgnat_range() (src/membership.rs) validates a candidate IP
    against the network's own configured subnet (read from GroupBlob), not a
    single hardcoded 100.64.0.0/10 constant.
    """
    req_id = "SUBNET-004"


class TunCreateParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-005

    tun::create() (src/tun.rs) computes its netmask from the configured
    prefix length and its gateway as (base + 1), instead of the hardcoded
    (255, 192, 0, 0) netmask and 100.64.0.1 gateway.
    """
    req_id = "SUBNET-005"


class ConflictCheckRemoved(Requirement):
    """REQUIREMENT-ID: SUBNET-006

    check_cgnat_conflict() (src/tun.rs) and its call site are removed. This
    fork deliberately uses a subnet outside 100.64.0.0/10, so there is nothing
    for this check to protect against, and it is what currently blocks startup
    next to Tailscale.
    """
    req_id = "SUBNET-006"


class MagicDnsRelocated(Requirement):
    """REQUIREMENT-ID: SUBNET-007

    MAGIC_DNS_V4 (src/dns.rs) is computed as an offset within the configured
    subnet instead of the fixed 100.100.100.53 literal. Assumes the configured
    subnet is /24 or larger.
    """
    req_id = "SUBNET-007"


class PtrHandlerParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-008

    The PTR/reverse-lookup NXDOMAIN range check in src/dns.rs (~line 246 as
    of commit 9e142411) mirrors whichever range check
    RangeValidationParameterized (SUBNET-004) implements, so both stay
    consistent.
    """
    req_id = "SUBNET-008"


# --------------------------------------------------------------------------
# Requirements: rebrand rayfish -> torpedo (RENAME-*)
# --------------------------------------------------------------------------

class BinaryRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-001

    The `ray` binary is renamed `torpedo` (Cargo.toml [[bin]], build output,
    contrib/rayfish.service's ExecStart path).
    """
    req_id = "RENAME-001"


class ServiceRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-002

    systemd service, unit file, and all systemctl invocations referring to
    "rayfish" are renamed to "torpedo" (src/cli/service.rs, src/cli/update.rs,
    src/update.rs, contrib/rayfish.service renamed to contrib/torpedo.service).
    """
    req_id = "RENAME-002"


class PathsRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-003

    Config dir (/etc/rayfish -> /etc/torpedo, src/config.rs), log dir
    (/var/log/rayfish -> /var/log/torpedo, src/logdir.rs), socket path
    (/var/run/rayfish/rayfish.sock -> /var/run/torpedo/torpedo.sock,
    ray-proto/src/ipc.rs), and the Unix group name (rayfish -> torpedo,
    src/cli/service.rs) are all updated consistently.
    """
    req_id = "RENAME-003"


class AlpnRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-004

    The mesh ALPN protocol prefix (rayfish/net/<version>/...) is changed to
    torpedo/net/<version>/... so this fork's wire traffic can never be confused
    with genuine rayfish traffic.
    """
    req_id = "RENAME-004"


# --------------------------------------------------------------------------
# Constraints: enforced by reconcile.py (CON-*)
# --------------------------------------------------------------------------

class RelayPresetUntouched(Constraint):
    """CONSTRAINT-ID: CON-001

    The "rayfish" relay preset name in src/config.rs (used by `torpedo config
    set relay rayfish`) must NOT be renamed. It refers to upstream's own hosted
    relay infrastructure, an external service name, not this fork's identity.
    Renaming it would silently break that feature.

    ENFORCEMENT (reconcile.py): relay_preset_untouched.value equals 'rayfish'.
    """
    constraint_id = "CON-001"
    enforcement_logic = "{{ relay_preset_untouched.value == 'rayfish' }}"


class NoLeftoverHardcodedCgnat(Constraint):
    """CONSTRAINT-ID: CON-002

    No remaining hardcoded 100.64.0.0/10-family literals in the touched
    files, other than the CLI default fallback value itself (which is an
    intentional, explicit default, not a hidden hardcode).

    ENFORCEMENT (reconcile.py): grep_hardcoded_cgnat.unexpected_count equals 0.
    """
    constraint_id = "CON-002"
    enforcement_logic = "{{ grep_hardcoded_cgnat.unexpected_count == 0 }}"


class BuildPasses(Constraint):
    """CONSTRAINT-ID: CON-003

    cargo build succeeds.

    ENFORCEMENT (reconcile.py): build.success is true.
    """
    constraint_id = "CON-003"
    enforcement_logic = "{{ build.success }}"


class ClippyClean(Constraint):
    """CONSTRAINT-ID: CON-004

    cargo clippy --all-targets is warning-free, per this repo's own
    CONTRIBUTING.md convention.

    ENFORCEMENT (reconcile.py): clippy.warnings equals 0.
    """
    constraint_id = "CON-004"
    enforcement_logic = "{{ clippy.warnings == 0 }}"


class TestsPass(Constraint):
    """CONSTRAINT-ID: CON-005

    cargo test passes.

    ENFORCEMENT (reconcile.py): test.pass is true.
    """
    constraint_id = "CON-005"
    enforcement_logic = "{{ test.pass }}"


# --------------------------------------------------------------------------
# Follow-up round: node subnet at boot (SUBNET-009/010) and self-update
# neutralization (UPGRADE-001 / CON-006).
# --------------------------------------------------------------------------

class ConfigSetSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-009

    `torpedo config set subnet <CIDR>` (plus `config get subnet` / `config unset
    subnet`) persists the node's operative overlay subnet in AppConfig.subnet,
    mirroring the existing relay / discovery-dns / dns-upstreams config keys. The
    value is validated as a CIDR (via membership::parse_cidr) before persisting;
    `unset` (or empty) restores the built-in default subnet (SUBNET-011). Like
    the other config keys it takes effect at the next daemon restart (`sudo
    torpedo restart`),
    when the daemon builds its single TUN device and identity in that subnet.
    This removes the need to hand-edit settings.toml or rely on a create-time
    value to move the node's TUN off 100.64.0.0/10.
    """
    req_id = "SUBNET-009"


class CreateUsesNodeSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-010

    `torpedo create` with no `--subnet` uses the persisted node subnet
    (AppConfig.subnet) as the new network's GroupBlob.subnet, so the node's TUN
    and the network agree without specifying the subnet twice. `create --subnet
    <CIDR>` still works and also persists the node subnet, keeping a single
    source of truth for the node's one TUN. On a node with no persisted subnet
    yet, `create --subnet` sets it. If `--subnet` disagrees with an
    already-persisted node subnet it is rejected with a clear error ("node
    subnet is <Y>; change it with `torpedo config set subnet` + restart first"),
    never silently producing a network the node's single TUN cannot carry.
    """
    req_id = "SUBNET-010"


class SelfUpdateNeutralized(Requirement):
    """REQUIREMENT-ID: UPGRADE-001

    The self-update path is neutralized, not deleted (keeps the diff small and
    reversible for upstream rebases). Gated on a single switch
    `update::SELF_UPDATE_ENABLED = false`: the daemon never spawns the
    auto-update task, and `torpedo update`, `torpedo auto-update on`, and
    `torpedo install --auto-update` refuse with a message pointing at manual
    binary replacement — the refusal happens before any network call, so no
    binary is ever fetched from the (upstream) REPO_SLUG. `torpedo version`
    stays fully functional (offline). Upgrades are done by replacing
    /usr/local/bin/torpedo and running `sudo torpedo restart`.
    """
    req_id = "UPGRADE-001"


class SelfUpdateDisabled(Constraint):
    """CONSTRAINT-ID: CON-006

    The self-update kill switch stays off: update::SELF_UPDATE_ENABLED is
    false, so no code path fetches or installs a binary from the upstream
    release repo. Prevents an accidental re-enable that would overwrite the
    torpedo binary with an upstream rayfish build.

    ENFORCEMENT (reconcile.py): self_update.enabled is false.
    """
    constraint_id = "CON-006"
    enforcement_logic = "{{ self_update.enabled == false }}"


class DefaultSubnetSafe(Requirement):
    """REQUIREMENT-ID: SUBNET-011

    The built-in default overlay subnet (membership::default_subnet, used when a
    GroupBlob's / config's subnet is None) changes from 100.64.0.0/10 to
    10.88.0.0/16 — an uncommon 10.x slice deliberately chosen NOT to collide
    with Tailscale's 100.64.0.0/10, so a no-flag `torpedo create` coexists with
    Tailscale out of the box. `--subnet` / `config set subnet` still override it.
    A /16 gives ample host space (~65k). reconcile.py's CON-002 allowed-default
    substring is updated accordingly, and the membership Magic-DNS test that
    checked the historical 100.100.100.53 address is re-pinned to an explicit
    100.64.0.0/10 subnet (that back-compat property holds for the /10 range
    regardless of what the default is).
    """
    req_id = "SUBNET-011"


class SubnetOverlapGuard(Requirement):
    """REQUIREMENT-ID: SUBNET-012

    At daemon startup the node rejects (refuses to start the data plane) if its
    configured overlay subnet overlaps an existing local interface / route, with
    a clear error telling the user to pick another via `torpedo config set
    subnet`. This is a NEW, subnet-aware guard — NOT a revival of the removed
    hardcoded check_cgnat_conflict (SUBNET-006): that one refused whenever any
    100.64.0.0/10 address was present (i.e. whenever Tailscale ran); this one
    only refuses on a genuine overlap between the *chosen* overlay subnet and a
    real local network, so it protects the host's routing without blocking the
    Tailscale-coexistence case (10.88.0.0/16 vs Tailscale's 100.64.0.0/10 do not
    overlap). Pairs with SUBNET-011: the safe default plus this guard mean a
    bad range fails loudly instead of hijacking the host's routes.
    """
    req_id = "SUBNET-012"


class ListenPortDistinct(Requirement):
    """REQUIREMENT-ID: RENAME-005

    The fixed UDP listen port constant is renamed RAYFISH_LISTEN_PORT ->
    TORPEDO_LISTEN_PORT (src/transport.rs) and its value changed 41383 -> 43737,
    so torpedo and a genuine rayfish daemon can bind their forwardable ports on
    the same host without collision (completes the wire/host isolation of
    RENAME-004). The port is a per-node local bind (peers discover each other's
    actual endpoint), so no cross-machine coordination is needed; 43737 avoids
    Tailscale (41641) and WireGuard (51820).
    """
    req_id = "RENAME-005"


class DefaultSubnetDocsAccurate(Requirement):
    """REQUIREMENT-ID: SUBNET-013

    User-facing help text and doc-strings state the ACTUAL default overlay
    subnet (10.88.0.0/16), not the old 100.64.0.0/10 that SUBNET-011 replaced:
    - `torpedo create --subnet` CLI help (src/main.rs) says the default is
      10.88.0.0/16.
    - The GroupBlob.subnet (src/membership.rs) and AppConfig.subnet
      (src/config.rs) field docs, and the IPC Create.subnet doc
      (ray-proto/src/ipc.rs), describe `None` as the 10.88.0.0/16 default.
    - The service startup-failure message (src/cli/service.rs) no longer claims
      a foreign VPN on 100.64.0.0/10 (Tailscale) is a likely cause — that
      conflict was intentionally removed — and instead points at the SUBNET-012
      overlay-overlap guard / DNS port 53 / a conflicting route.

    Explicitly OUT OF SCOPE (documented deferrals, not the fork's Linux path,
    decision left for later): the macOS `route_peer_range` branch (src/tun.rs),
    the Android VpnService (android/), and the upstream e2e/bench shell harnesses
    (tests/) still assume 100.64.0.0/10. They are adapted or removed in a future
    project, not here.
    """
    req_id = "SUBNET-013"


# --------------------------------------------------------------------------
# Thorough-fork round: purge residual `rayfish` identity from host-visible
# artifacts and cosmetics (RENAME-006..009 / CON-007). Distinct from the
# KEEP-ON-PURPOSE names (upstream relay/discovery presets, REPO_SLUG, the
# `.ray` TLD, the internal Cargo crate name `rayfish`), which CON-001 and the
# honesty rationale explicitly protect and which this round must NOT touch.
# --------------------------------------------------------------------------

class HostDnsArtifactsRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-006

    The host-filesystem artifacts the DNS layer writes (src/dns_config.rs) carry
    the `torpedo` identity, not `rayfish`, so torpedo and a genuine rayfish
    daemon on the same host never read or clobber each other's files — the
    coexistence guarantee RENAME-004/005 established for the wire and ports,
    extended to disk. Rename, consistently across writers, marker-guards, backup
    /restore, the panic-time emergency restore, and tests:
    - NetworkManager drop-in `/etc/NetworkManager/conf.d/rayfish-dns.conf` ->
      `torpedo-dns.conf` (NM_DROPIN).
    - resolv.conf takeover marker `# Added by rayfish - do not edit` ->
      `# Added by torpedo - do not edit` (HEADER_COMMENT); the "ours?" marker
      check and re-assert log follow the new marker.
    - resolv.conf backup suffix `.before-rayfish` -> `.before-torpedo`
      (BACKUP_SUFFIX) and the macOS `/etc/resolver/<tld>.before-rayfish` backup.
    - resolvconf interface tags `tun-rayfish`/`tun-rayfish.inet` ->
      `tun-torpedo`/`tun-torpedo.inet`.
    - macOS SCDynamicStore service key `State:/Network/Service/rayfish/DNS` and
      the `SCDynamicStoreBuilder::new("rayfish")` client name -> `torpedo`.
    Because the marker guard keys on our own marker, only a file torpedo itself
    wrote is ever modified; the fork is pre-deployment so there is no old-marker
    migration to carry. The upstream `relay.iroh.rayfish.xyz` /
    `dns.iroh.rayfish.xyz` preset URLs are NOT touched (CON-001) — those name
    upstream's servers, not a host artifact.
    """
    req_id = "RENAME-006"


class UserIdentifiersRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-007

    The remaining user-typed / user-visible identifiers carry the `torpedo`
    identity:
    - Deep-link URI scheme `rayfish://<verb>/<code>` -> `torpedo://<verb>/<code>`
      (src/deeplink.rs), including the module's public symbols `RayfishLink` ->
      `TorpedoLink` and `parse_rayfish_uri` -> `parse_torpedo_uri` and every
      caller, so a scanned/pasted invite link is unambiguously this fork's.
    - Config-dir override env var `RAYFISH_CONFIG_DIR` -> `TORPEDO_CONFIG_DIR`
      (src/config.rs and the test-serialization lock doc/callers), so it cannot
      collide with a genuine rayfish process's own override on the same host.
    """
    req_id = "RENAME-007"


class MacosServiceIdentityRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-008

    The macOS service identity is rebranded and a stale binary-path bug is fixed
    (src/cli/service.rs and contrib/):
    - launchd label / plist `com.rayfish.vpn` -> `com.torpedo.vpn`
      (contrib/com.rayfish.vpn.plist renamed to contrib/com.torpedo.vpn.plist;
      the include_str! path, the /Library/LaunchDaemons plist path, and the
      launchctl load/unload/kickstart invocations follow).
    - BUG FIX: the plist install currently replaces `/usr/local/bin/ray` (the
      pre-fork binary name) instead of `/usr/local/bin/torpedo`, so the macOS
      ExecStart never points at the real binary; corrected to `torpedo`.
    NOTE: the macOS platform's ultimate fate (fully implement vs. rip out, see
    SUBNET-013 deferrals) is still undecided; this change keeps the macOS path
    internally consistent and collision-free in the meantime so that decision is
    not pre-empted by leftover `rayfish` identifiers.
    """
    req_id = "RENAME-008"


class CosmeticIdentitySweep(Requirement):
    """REQUIREMENT-ID: RENAME-009

    Non-functional cosmetic cleanup (Bucket 3): source comments, doc-strings, and
    local variable names that still say "rayfish" but describe THIS fork are
    reworded to "torpedo" (e.g. dns_config.rs `rayfish_domains` locals, "routes
    queries to rayfish" comments; main.rs `/usr/local/bin/ray` test fixtures).
    Also the crate/bug-report metadata that describes THIS package points at the
    fork (github.com/ErikAllanKincaid/torpedo): Cargo.toml +
    ray-proto/Cargo.toml `repository`/`homepage`, the ray-proto `description`,
    and REPORT_REPO_URL (src/cli/status.rs) so `torpedo report` opens an issue on
    the fork's tracker, not upstream's. No behavioral effect on the mesh; done
    opportunistically in files already edited by RENAME-006..008.

    Deliberately EXCLUDED (KEEP-ON-PURPOSE, not cosmetic churn): the internal
    Cargo crate/lib name `rayfish` and all `use rayfish::` references (renaming is
    large internal churn with no user-visible or coexistence benefit); the
    `authors = Dario <dario@rayfish.xyz>` attribution (honest credit);
    `REPO_SLUG = rayfish/rayfish` (names upstream's real release repo, only used
    by the now-disabled self-updater); the `"rayfish"` relay/discovery preset
    keyword and URLs (CON-001); and the `.ray` Magic-DNS TLD.
    """
    req_id = "RENAME-009"


class NoResidualHostIdentityLeak(Constraint):
    """CONSTRAINT-ID: CON-007

    After RENAME-006..008, none of the collision-prone `rayfish` host-artifact /
    user-identifier tokens remain in src/: the curated set is `rayfish-dns.conf`,
    `.before-rayfish`, `# Added by rayfish`, `tun-rayfish`, `com.rayfish.vpn`,
    `rayfish://`, `RAYFISH_CONFIG_DIR`, and the SCDynamicStore `rayfish` service
    key/client name. This is a completeness + anti-regression gate; it targets
    those specific tokens only, so it never trips on the KEEP-ON-PURPOSE
    `rayfish` names (relay/discovery preset URLs, REPO_SLUG, crate name, author
    attribution), which are allowed to remain.

    ENFORCEMENT (reconcile.py): host_identity.leak_count equals 0.
    """
    constraint_id = "CON-007"
    enforcement_logic = "{{ host_identity.leak_count == 0 }}"


class SurfaceDnsTakeoverWarning(Requirement):
    """REQUIREMENT-ID: DNS-001

    When torpedo has to manage /etc/resolv.conf directly (the tier-5
    `DirectResolvConf` takeover, reached only when no systemd-resolved,
    NetworkManager, resolvectl, or resolvconf backend is present — the common
    case on a default Debian trixie server / minimal install), `sudo torpedo up`
    MUST surface a user-visible warning, not merely a daemon-side log line. The
    prior behavior logged the takeover at INFO to /var/log/torpedo, so the user
    discovered the change only by noticing that their resolv.conf had been
    rewritten (reported in the field against upstream).

    The warning rides the EXISTING `activate()` `warnings` channel (returned in
    the `Up` IPC Ok message and rendered by `torpedo up`), so no IPC wire change
    is needed. Its text names the backup path (/etc/resolv.conf.before-torpedo)
    and the restore path (`torpedo down` / `sudo torpedo uninstall`) so the
    notice is actionable, not alarming. Implemented as a
    `DnsConfigurator::user_warning()` trait method: default None (split-DNS
    backends leave resolv.conf untouched), overridden to Some(..) by
    DirectResolvConf; `DnsManager::configure` pushes it into `warnings`. The
    takeover daemon log is also raised INFO -> WARN so it appears in
    `torpedo report` bundles for the non-interactive (reboot / auto-activate)
    path where there is no CLI to print to.

    Scope: covers the interactive `torpedo up`. Surfacing the active DNS mode in
    `torpedo status` for the non-interactive path is a separate later item.

    ENFORCEMENT: unit test (run by reconcile.py's `test` check) asserts
    DirectResolvConf::user_warning() is Some and names the backup file, and that
    a split-DNS configurator returns None.
    """
    req_id = "DNS-001"


class SubnetChangeObservableAndAnnounced(Requirement):
    """REQUIREMENT-ID: SUBNET-014

    Two subnet-UX fixes found in Phase-7 live testing.

    (1) `create --subnet X` / `join` onto a network whose subnet differs from this
    node's live TUN persist the subnet but only apply it to the TUN at the next
    (re)start. Previously silent, so the node kept its old subnet while the roster
    advertised the new one and NO IP forwarding worked until a manual restart. The
    `Created`/`Joined` IPC responses now carry an optional `warning`; the CLI
    prints it when the chosen subnet != the live TUN subnet ("subnet B/P takes
    effect after `sudo torpedo restart`"). The pure helper is
    `membership::subnet_change_warning`.

    (2) `config get` as a non-root user cannot read the 0600 root:root
    settings.toml (it holds contact_secret_key, so its perms must NOT be relaxed),
    so config::load() silently returned defaults and misreported e.g. `subnet` as
    <default> while the node ran on 10.99. `config get` now detects the unreadable
    file and errors with a "re-run with sudo" hint instead of a wrong value;
    `sudo torpedo config get` shows the real value. Full read-via-daemon IPC is a
    deferred follow-up.

    ENFORCEMENT: unit test on subnet_change_warning (reconcile's `test` check).
    """
    req_id = "SUBNET-014"
