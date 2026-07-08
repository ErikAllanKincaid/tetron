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


class UserFacingCommandNameRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-011

    RENAME-006..009 renamed host artifacts, wire identifiers, and doc-comment/
    metadata cosmetics, but missed the pre-fork upstream binary's own short
    name, `ray`, hardcoded directly into ~40 LIVE, reachable, user-facing
    strings: CLI hint text, error messages, an IPC response message, a printed
    YAML example, the `torpedo version` banner, and shell-completion
    registration. A user following any of these would try to run a binary that
    does not exist on a torpedo install. Found via live two-machine testing
    (`torpedo version` was directly observed printing `ray 0.1.5 (...)` on the
    first line, `torpedo --version` printing `torpedo 0.1.5 (...)` on the
    second — the same binary, two different self-identifications).

    Renamed (literal `ray` -> `torpedo` in each string, no behavior change):
    - `src/main.rs`: the `clap_complete::generate(shell, ..., "ray", ...)` call
      (so `torpedo completions <shell>` registers completions for a command
      name that actually exists); the `Command::Version` println (the
      `ray {FULL_VERSION}` banner); both `config set`/`unset` "restart" hints.
    - `src/cli/status.rs`: `infer_hint`'s three hints (daemon-not-running,
      expired-invite, needs-operator); the inactive-data-plane hint; the
      version-skew hint; all four `print_pending_summary` command hints
      (`firewall pending`, `requests`, `files`, `connections`).
    - `src/cli/network.rs`: the post-`create` invite hint and both `print_next`
      command tables (`ray status`/`ray up`).
    - `src/cli/invite.rs` (join hint, reusable-key hint, admit hint),
      `src/cli/pair.rs` (unpair hint), `src/cli/connect.rs` (approve hint,
      share hint, incompatible-version hint), `src/cli/alias.rs` (identity hint),
      `src/cli/service.rs` (sudo re-run hint), `src/cli/files.rs` (accept hint),
      `src/cli/firewall.rs` (disabled-state hint, invite-missing suggested
      command, alias-identity hint).
    - `src/apply.rs`: the non-YAML error message, and the entire `EXAMPLE_SPEC`
      constant printed by `torpedo apply --example` (also fixes a stray
      "Rayfish deploy spec" mention).
    - `src/onepassword.rs`: the backup item's stored `value` text — this one
      is written verbatim into the user's own 1Password vault item by
      `torpedo pair backup --1password`, so the leak is persisted outside the
      repo entirely until fixed. Also `src/main.rs`'s `pair backup`/`pair
      restore --1password` item **title** default, `"Rayfish Identity"` ->
      `"Torpedo Identity"` (both subcommands, kept identical since restore
      looks up by this default title). This fork is pre-release with no real
      users, so there is no existing backup stored under the old title to
      break; a back-compat lookup is unneeded and was not added.
    - `src/daemon/mod.rs` (operator-grant hint + confirmation message),
      `src/daemon/mesh/runtime.rs` (kick-yourself error), `src/daemon/mesh/
      create_join.rs` (pending-approval message, version-mismatch message),
      `src/daemon/mesh/files.rs` (auto-accept warning, not-your-device error),
      `src/daemon/mesh/firewall.rs` (mesh-SSH no-peer-authorized nudge).
    - `src/lib.rs`: `APP_NAME` corrected from `"ray"` to `"torpedo"`. Dormant
      (grep confirms nothing references this constant), but an exported wrong
      value is exactly the residual-identity class this series targets, and
      the fix is zero-risk since nothing consumes it today.

    Deliberately EXCLUDED (false positives / different `ray` / out of scope):
    - `src/lib.rs`'s `DNS_DOMAIN = "ray"` and every `.ray`-suffixed hostname in
      `src/dns.rs`, `src/dns_resolver.rs`, `src/dns_config.rs` (tests and
      domain-suffix logic) — this is the KEEP-ON-PURPOSE `.ray` Magic-DNS TLD,
      an unrelated "ray".
    - `src/network_name.rs`'s hostname-generator wordlist entry `"ray"` —
      the English word (as in stingray), coincidental, part of a list with
      "reed", "pond", "quay".
    - `src/update.rs`'s `release_asset_name` (`ray-{os}-{arch}`) and the
      matching literals in `src/main.rs` (`ray-linux-x86_64` etc.) — these name
      **upstream's own** release asset filenames (self-update, gated off by
      CON-006, still points `REPO_SLUG` at `rayfish/rayfish` on purpose);
      renaming would make a hypothetical re-enabled updater look for an asset
      that does not exist in upstream's releases.
    - Every other user-facing string inside `cli/update.rs` past its
      `SELF_UPDATE_ENABLED` early-return (confirmed unreachable in this fork's
      shipped behavior — `cmd_update` returns before reaching any of them).
    - Source comments and doc-comments (`//`, `///`, `//!`) mentioning `ray
      <verb>` — not user-facing, matches the cosmetic carve-out RENAME-009
      already established; left for a later opportunistic pass, not this one.

    No new Constraint: unlike CON-007's curated host-artifact tokens (which
    never appear in comments or dead code), a token-count gate here would
    false-fail on the deliberately-untouched comments and the dead
    `cli/update.rs` tail, which still contain `ray <verb>` after this change.
    Verified by reading the diff, same as RENAME-006..009.
    """
    req_id = "RENAME-011"


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


class NoMutualDnsForwardingLoopWithTailscale(Requirement):
    """REQUIREMENT-ID: DNS-003

    CRITICAL, TOP PRIORITY — found live in Phase-7 two-machine testing
    (2026-07-08, xps-17-9720). On a tier-5 host (no systemd-resolved,
    NetworkManager, resolvectl, or resolvconf — DirectResolvConf takeover,
    same class of host DNS-001 covers) running Tailscale, torpedo and
    Tailscale form a MUTUAL DNS FORWARDING LOOP that breaks ALL DNS
    resolution system-wide — not just `.ray` names, ALL of it, including the
    torpedo daemon's own outbound HTTP (pkarr discovery). This directly
    defeats the fork's entire reason to exist: coexisting with Tailscale.

    SYMPTOMS (all observed live on xps-17-9720, a minimal Debian-trixie-family
    host, LMDE, with Tailscale active):
    - `torpedo join <invite>` fails immediately: "failed to resolve network
      record: failed to resolve network record: Service 'pkarr' failed".
    - Direct queries to EITHER resolver hang for the full timeout and return
      nothing: `dig @100.100.100.100 github.com` (Tailscale's quad-100) and
      `dig @10.99.100.53 github.com` (torpedo's magic resolver, subnet-derived)
      both time out. Critically, `dig @10.99.100.53 <anything>.ray` answers
      correctly and instantly (NXDOMAIN + SOA, 0ms) — torpedo's local `.ray`
      answering path and its TUN-interception plumbing are NOT the bug.
    - `ping -c1 github.com` on the host: "Temporary failure in name
      resolution" — total outbound DNS failure, confirmed independent of any
      particular application.
    - Raw ICMP to both `100.100.100.100` and `10.99.100.53` succeeds fine
      (sub-millisecond) as both root and non-root — the network/routing path
      is healthy; this is a DNS-application-layer bug, not connectivity.
    - `journalctl -u tailscaled` shows, at the exact moment of failure:
      `dns udp query: waiting for response or error from [10.99.100.53]:
      context deadline exceeded` and `dns udp query: request queue full`
      (with hundreds of queries dropped under `[RATELIMIT]`) — i.e.
      Tailscale's own DNS proxy is ALSO stuck waiting on torpedo's resolver.

    DIAGNOSIS (root cause, confirmed via `/etc/resolv.conf.before-torpedo`,
    the daemon's own capture log, and the interleaved torpedo/tailscaled
    journal):
    1. Before torpedo ran, `/etc/resolv.conf` was Tailscale's own file:
       `nameserver 100.100.100.100` (this is the literal content of the
       `.before-torpedo` backup — confirmed).
    2. `DirectResolvConf::new()` (src/dns_config.rs) reads that file BEFORE
       overwriting it and correctly captures `100.100.100.100` as the sole
       upstream via `parse_resolv_nameservers` (also confirmed via the
       daemon's own `took over /etc/resolv.conf directly … upstreams=
       [100.100.100.100]` log line, present at every one of several restarts
       in this test run). torpedo's capture step is NOT the bug — it also
       already excludes its own magic IP from a captured upstream list
       (`parse_resolv_nameservers` filters `crate::dns::magic_dns_v4_node()`),
       so torpedo correctly guards against looping to itself on re-takeover.
    3. `DirectResolvConf::apply()` then overwrites `/etc/resolv.conf` to point
       solely at torpedo's own magic resolver IP (`10.99.100.53`, subnet-
       derived).
    4. `tailscaled` ITSELF watches `/etc/resolv.conf` to learn where to
       forward queries its own quad-100 resolver can't answer (the same
       "watch resolv.conf for the real upstream" design torpedo uses,
       independently implemented). Once torpedo rewrites the file, tailscaled
       adopts torpedo's magic IP (`10.99.100.53`) as ITS OWN upstream —
       tailscaled has no way to know that IP belongs to another VPN's
       self-referential resolver, so it has no equivalent guard.
    5. Net effect, a perfect two-hop loop with no real exit: an app's query
       hits `10.99.100.53` (torpedo, the OS's sole nameserver) -> torpedo
       does not recognize the name as `.ray` -> forwards to its captured
       upstream `100.100.100.100` (Tailscale) -> Tailscale's quad-100 also
       does not recognize the name -> forwards to ITS captured upstream,
       `10.99.100.53` -> back to torpedo. Neither side ever reaches the real
       internet; each side's own timeout eventually fires (torpedo's
       `forward_once`, 3s; tailscaled's own deadline), which is exactly the
       observed hang-then-timeout behavior, not an instant error.

    WHY THIS WAS NOT CAUGHT ON THE COORDINATOR (AORUS, tier-1): AORUS has
    systemd-resolved, so torpedo took the D-Bus split-DNS registration path
    (`configured systemd-resolved via D-Bus for .ray`) instead of touching
    `/etc/resolv.conf` at all, and Tailscale itself very likely also registers
    with systemd-resolved there rather than writing the file directly — so
    there is no file for the two to collide over. This loop is specific to
    hosts where BOTH torpedo and Tailscale independently fall back to direct
    `/etc/resolv.conf` management (tier-5, this fork's own DNS-001 scenario) —
    which, per DNS-001's own framing, is not a rare edge case: it is "the
    common case on a default Debian trixie server / minimal install", i.e.
    exactly the kind of host an operator would run a lean VPN mesh on.

    IMPACT: on any tier-5 host running Tailscale, `sudo torpedo up` silently
    breaks ALL system DNS (not just `.ray`), including torpedo's own ability
    to join a network (pkarr discovery needs working DNS to resolve
    `dns.iroh.link`). This is a total, silent failure of the fork's headline
    coexistence promise on a documented-common host class, not a cosmetic bug.

    NOT YET FIXED — left open deliberately for focused design work (this
    docstring is diagnostic, not prescriptive). Candidate directions worth
    weighing, none chosen yet: detect that Tailscale is active (e.g. a
    `tailscale0` interface or `100.100.100.100` present in the pre-takeover
    resolv.conf) and refuse the direct takeover / warn loudly instead of
    proceeding blind; special-case known other-VPN quad-resolver IPs
    (`100.100.100.100`) by never handing them out as a captured upstream
    that could loop back; or (a smaller, more surgical option) detect the
    specific loop pattern at forward time (a query that bounces back to
    ourselves) and fail fast with a clear error instead of a silent hang.

    ENFORCEMENT: none yet (no fix designed or landed). Add a unit test once a
    fix direction is chosen; this is exactly the kind of interaction bug a
    single-process unit test cannot catch on its own (it requires a second,
    real DNS-proxying daemon), so integration-level verification (a live
    two-machine re-test with Tailscale + a tier-5 host) is the real gate.
    """
    req_id = "DNS-003"


class ForeignOverlayUpstreamLoopBreaker(Requirement):
    """REQUIREMENT-ID: DNS-004

    The SAFETY NET for the opt-in `magic-dns direct` takeover. The primary
    resolution of DNS-003 is DNS-005, which makes the /etc/resolv.conf takeover
    opt-in so the loop cannot occur by default. This requirement hardens the
    remaining case: a user who explicitly runs `torpedo config set magic-dns
    direct` on a tier-5 host that ALSO runs Tailscale. Without it, that opt-in
    would re-expose the mutual torpedo<->Tailscale DNS forwarding loop of
    DNS-003. Two decoupled mechanisms plus housekeeping.

    (1) LOOP-BREAKER — never adopt a foreign overlay VPN's resolver as our
    upstream. `parse_resolv_nameservers` (src/dns_config.rs) already drops our
    OWN magic IP; extend the same filter to drop EVERY address in the whole
    CGNAT / shared range 100.64.0.0/10 (RFC 6598). That range is where overlay
    VPNs park self-referential stub resolvers (Tailscale's 100.100.100.100, and
    the LEGACY rayfish magic 100.100.100.53 from before this fork moved the
    default subnet to 10.88.0.0/16) — a genuine recursive resolver essentially
    never lives there. A captured 100.64/10 address is precisely the poison that
    forms the loop, so refusing to forward to it breaks the loop at the source.

    This is deliberately NOT gated on "NetworkManager in default DNS mode" (an
    early candidate). That signal is wrong twice over: it MISSES the pure no-NM
    tier-5 host (minimal Debian netinst on ifupdown+dhclient or systemd-networkd
    without resolved), which DNS-001 itself calls "the common case" and which
    loops identically; and it is unnecessary, because the filter is already
    scoped to the only risky path (captured_upstreams() is non-empty ONLY in
    DirectResolvConf — every split-DNS backend returns empty) AND self-gates by
    content (it does nothing unless a 100.64/10 address is actually present,
    which only happens when a foreign overlay poisoned resolv.conf). On a normal
    tier-5 host with a real router (e.g. 192.168.1.1) and no Tailscale it is a
    no-op. Every drop is logged so an operator can see it happened.

    (2) REAL-UPSTREAM RECOVERY — the loop-breaker alone converts a hang into a
    failure (on a Tailscale-first-then-torpedo host the captured set was ONLY
    100.100.100.100, so after the filter it is empty and non-`.ray` DNS dies).
    To restore working internet DNS, recover the genuine upstream, in priority:
      (a) config `dns_upstreams` if the operator set it (already honored via
          config::resolve_upstreams);
      (b) NetworkManager's DHCP-learned nameservers via D-Bus (the physical NIC's
          Ip4Config.Nameservers — unpoisoned even while Tailscale owns
          resolv.conf, since tailscale0 is unmanaged; this is where NM detection
          EARNS its keep, as a SOURCE, not as a gate on (1); confirmed present as
          192.168.1.1 on the xps repro host, NM in default mode);
      (c) a coexisting overlay's own pre-takeover backup file
          (/etc/resolv.pre-tailscale-backup.conf — on the repro host it held
          `nameserver 192.168.1.1`), parsed with the same 100.64/10 exclusion;
      (d) if still empty, DO NOT silently egress to a public resolver: leave
          non-`.ray` unresolved and surface a clear warning naming
          `torpedo config set dns-upstreams <ip>` (reuses the DNS-001 warnings
          channel + the DNS-002 status surface). A public default may later be
          an explicit opt-in, never the silent fallback.
    Result: order-independent correctness. Whether Tailscale or torpedo starts
    first, torpedo forwards non-`.ray` to the REAL router, `.ray` resolves
    locally, and `.ts.net` still resolves (Tailscale stays authoritative for its
    own zone — no loop, because torpedo's base is now the real router, not a VPN
    proxy).

    (3) HOUSEKEEPING — purge stale `100.100.100.53` literals that misdocument
    torpedo's own resolver as the legacy /10-derived address instead of the
    subnet-derived magic_dns_v4_node() (10.88.100.53 on the default subnet):
    src/dns_config.rs module doc (line ~3), the comments at ~953 and ~1079, and
    the test fixture string at ~1206/~1224. Cosmetic but prevents a reader from
    trusting a wrong resolver IP (the standing "resolver IP is subnet-derived"
    doc-fix item).

    NON-GOAL (v1): actively reconfiguring Tailscale, or special-casing `.ts.net`
    forwarding. Not needed — once torpedo's base is the real router, Tailscale's
    own authoritative answering keeps `.ts.net` working with no torpedo help.

    ENFORCEMENT: unit tests (reconcile.py's `test` check) — (i)
    parse_resolv_nameservers drops ALL 100.64/10 addresses (100.100.100.100 and
    100.100.100.53) while keeping a real router IP (192.168.1.1); (ii) the
    recovery chain's ordering/short-circuit is a pure, tested helper. The NM
    D-Bus source and the true two-daemon loop are not unit-testable in-process
    (per DNS-003), so a live xps + Tailscale re-test remains the integration gate.
    """
    req_id = "DNS-004"


class MagicDnsIsOptInNeverSeizesResolvConf(Requirement):
    """REQUIREMENT-ID: DNS-005

    THE PRIMARY resolution of DNS-003: torpedo does not touch /etc/resolv.conf by
    default. Magic DNS (`.ray` name resolution) is a convenience, not a
    requirement — the mesh data plane, firewall, embedded SSH, and file transfer
    never use system DNS, the `torpedo` CLI resolves hostnames daemon-side from
    the roster, and `torpedo status` already lists every peer's mesh IP (its own
    IP on each network header + an ipv4 column per peer row). So an operator can
    reach every host by mesh IP (or a one-time ~/.ssh/config alias) with no OS-DNS
    changes at all. Seizing /etc/resolv.conf to answer `.ray` is exactly what
    collides with another VPN that manages the same file (Tailscale) and produces
    the DNS-003 blackhole — so it must never be the default.

    MECHANISM: a node-global setting `magic_dns: MagicDnsMode` in settings.toml
    (config.rs), three values, set via `torpedo config set magic-dns off|auto|direct`:
      - `off`   — never configure OS DNS at all; DnsManager::configure returns
                  early. Pure mesh-IP operation.
      - `auto`  — DEFAULT. Use a CLEAN split-DNS backend if present
                  (systemd-resolved / NetworkManager dnsmasq / resolvconf — all
                  cooperative, none collide with another VPN); if only the direct
                  /etc/resolv.conf takeover remains, DECLINE it and surface a
                  plain-English notice (dns_config::magic_dns_declined_notice)
                  naming the two ways to enable `.ray`: install systemd-resolved,
                  or `magic-dns direct`.
      - `direct`— additionally permit the /etc/resolv.conf takeover as a last
                  resort (the pre-existing behavior), now guarded by DNS-004's
                  loop-breaker. Opt-in only.
    detect_and_configure(tun_name, allow_direct) returns Option<Box<dyn
    DnsConfigurator>>: Some(clean backend) is always used when present; the direct
    fallback is constructed only when allow_direct (i.e. mode == direct);
    otherwise Ok(None) => the decline notice. `off` short-circuits before
    detection. The cooperative backends are UNCHANGED — this only removes the
    unconditional DirectResolvConf fallback from the default path.

    WHY THIS IS THE RIGHT DEFAULT (field topology): the tier-5 host in the repro
    is the workstation (xps-17, LMDE trixie, NM in default DNS mode), the tier-1
    clean host is the headless server (AORUS, systemd-resolved). On a workstation
    the user does want `.ray`, but the safe default is still hands-off: it never
    blackholes DNS, and the workstation user can either install systemd-resolved
    (moves to the clean path both torpedo and Tailscale share) or opt into
    `magic-dns direct`. Users who love Magic DNS keep it — for free on any host
    with a clean backend (`auto` just works there, e.g. AORUS), or via the opt-in
    on a minimal host.

    ENFORCEMENT: unit tests (reconcile.py's `test` check) — MagicDnsMode::default
    is Auto and !allows_direct; parse/set/get roundtrip incl. reset-on-empty and
    rejection of a bad mode; persistence across save/load. The decline-vs-takeover
    branch and the true two-daemon coexistence are integration-verified by the
    live xps + Tailscale re-test (per DNS-003).
    """
    req_id = "DNS-005"


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


class ClosedNetworkInboundDefaultAllow(Requirement):
    """REQUIREMENT-ID: FW-001

    A CLOSED (invite-gated) network is a trusted mesh, so inbound from it defaults
    to ALLOW: connectivity is open like a normal LAN and the host service's own
    auth (SSH keys, DB creds, etc.) is the gate, instead of requiring an explicit
    firewall rule per service. OPEN networks keep the secure deny-inbound default —
    a stranger who joins must be explicitly allowed.

    Mechanism: an `allow in any` rule scoped to the network (RuleOrigin::ClosedDefault
    / firewall::closed_default_rule), appended at the BACK so any explicit rule —
    including a deny — overrides it. SharedFirewall::set_closed_default(net, on)
    seeds/removes it and returns the config to persist. Seeded when this node
    CREATES a closed (Restricted) network or JOINS one with an invite/reusable key
    (both prove the network is closed); removed on leave/nuke. Reconvergence (which
    replaces RuleOrigin::Network suggestion rules) never touches it.

    v1 limitation: members do not yet learn a network's mode from the signed blob,
    so the trigger is LOCAL knowledge (created-closed / invite-joined). An
    approval-joined closed network, or any open network, gets no rule and stays
    deny — conservative (deny) when the mode is unknown. Propagating the mode in
    the blob so members always classify correctly is a follow-up.

    ENFORCEMENT: unit test on set_closed_default (reconcile's `test` check).
    """
    req_id = "FW-001"
