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

    cargo clippy --all-targets is warning-free.

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

    After RENAME-006..008 (and RENAME-015's observability names), none of the
    collision-prone `rayfish` host-artifact / user-identifier tokens remain in
    src/: the curated set is `rayfish-dns.conf`, `.before-rayfish`, `# Added by
    rayfish`, `tun-rayfish`, `com.rayfish.vpn`, `rayfish://`, `RAYFISH_CONFIG_DIR`,
    the SCDynamicStore `rayfish` service key/client name, and (RENAME-015) the
    observability names `name = "rayfish"` / `name = "rayfish_peer"` (Prometheus
    metric families) and `service_name("rayfish")` / `tracer("rayfish")` (OTEL).
    This is a completeness + anti-regression gate; it targets those specific
    tokens only, so it never trips on the KEEP-ON-PURPOSE `rayfish` names
    (relay/discovery preset URLs, REPO_SLUG, crate name, author attribution),
    which are allowed to remain.

    ENFORCEMENT (reconcile.py): host_identity.leak_count equals 0.
    """
    constraint_id = "CON-007"
    enforcement_logic = "{{ host_identity.leak_count == 0 }}"


class BuildToolingIdentityRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-010

    `justfile`'s `deploy`/`deploy-dev`/`cross` recipes carried the pre-fork
    identity (`binary := "ray"`, `groupadd rayfish`, `systemctl restart
    rayfish`) — fixed in commit `b2c2d89` (`binary := "torpedo"`, `groupadd
    torpedo`, `systemctl restart torpedo`), predating this requirement being
    formally tracked. `contrib/` (`com.torpedo.vpn.plist`, `torpedo.service`)
    was already clean. This class exists mainly to record that the fix landed
    and give CON-008 (below) something to cite — see CON-008 for the
    anti-regression gate.

    Out of scope on purpose: `ray-mobile`/`libray_mobile` (the Android
    crate/artifact name referenced from `justfile`'s `apk` recipe) is a
    separate, deliberately-undecided naming question (TODO.md's Android
    rewrite section) — not a leftover to clean up here, and CON-008's token
    list does not flag it.

    Also fixed alongside this (2026-07-08): AGENTS.md's "justfile caution"
    note still warned `just cross`/`just deploy`/`just deploy-dev` were stale
    and unsafe to use, describing the pre-`b2c2d89` state — corrected to
    reflect that the identity fix landed and they're safe to use.

    ENFORCEMENT: see CON-008.
    """
    req_id = "RENAME-010"


class NoResidualBuildToolingIdentityLeak(Constraint):
    """CONSTRAINT-ID: CON-008

    Anti-regression gate for RENAME-010, mirroring CON-007's approach but for
    build/deploy tooling instead of Rust source: CON-007's `host_identity`
    check only scans `src/**/*.rs`, so a stale `rayfish` token reintroduced in
    `justfile` or `contrib/` would go completely undetected by the existing
    gates. Curated token set (same anti-false-positive rationale as CON-007):
    `binary := "ray"`, `groupadd rayfish`, `systemctl restart rayfish`,
    `systemctl stop rayfish`, `/etc/rayfish`, `rayfish.service`,
    `com.rayfish.vpn`. Deliberately excludes `ray-mobile`/`libray_mobile`
    (RENAME-010's documented out-of-scope item).

    ENFORCEMENT (reconcile.py): build_tooling_identity.unexpected_count
    equals 0.
    """
    constraint_id = "CON-008"
    enforcement_logic = "{{ build_tooling_identity.unexpected_count == 0 }}"


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
          config::resolve_upstreams, applied before recovery runs);
      (b) IMPLEMENTED: a coexisting overlay's own pre-takeover backup file —
          Tailscale's /etc/resolv.pre-tailscale-backup.conf, where it stashes the
          true DHCP upstream (on the repro host it held `nameserver 192.168.1.1`);
          read by dns_config::recover_real_upstreams() and parsed with the SAME
          100.64/10 exclusion so a poisoned backup can never re-introduce the loop.
      (c) DEFERRED: NetworkManager's DHCP-learned nameservers via D-Bus (the
          physical NIC's Ip4Config.Nameservers — unpoisoned even while Tailscale
          owns resolv.conf, since tailscale0 is unmanaged; NM detection as a
          SOURCE, not a gate on (1)). A more general source than (b) but more
          code; the backup-file source already covers the Tailscale repro, so NM
          D-Bus is a follow-up if a no-Tailscale-backup host ever needs it.
      (d) if still empty, DO NOT silently egress to a public resolver: leave
          non-`.ray` unresolved and surface a clear warning naming
          `torpedo config set dns-upstreams <ip>` (reuses the DNS-001 warnings
          channel). A public default may later be an explicit opt-in, never the
          silent fallback.
    Wired in DnsManager::configure: only when is_direct && the resolved upstream
    set is empty. Result (VERIFIED LIVE on xps + Tailscale, `magic-dns direct`):
    no loop, `github.com` resolves via torpedo->192.168.1.1, `.ray` resolves
    locally, tailscaled shows zero `deadline exceeded`/`queue full`.

    KNOWN LIMITATION of `direct` on a Tailscale host (verified: NXDOMAIN): while
    torpedo owns resolv.conf it is the SOLE nameserver and forwards non-`.ray`
    queries to the real router, which does not know `.ts.net` — so Tailscale's own
    MagicDNS names stop resolving system-wide (Tailscale itself still answers when
    queried directly at 100.100.100.100). `direct` therefore trades `.ts.net` for
    `.ray`. Recovering BOTH needs the deferred `.ts.net`->Tailscale forwarding
    (below), or the clean answer: run systemd-resolved so both VPNs take the
    split-DNS path and neither seizes resolv.conf.

    (3) HOUSEKEEPING — purge stale `100.100.100.53` literals that misdocument
    torpedo's own resolver as the legacy /10-derived address instead of the
    subnet-derived magic_dns_v4_node() (10.88.100.53 on the default subnet):
    done in src/dns_config.rs (module doc, the two direct-mode comments, and the
    resolv_conf_is_ours test fixture). Cosmetic but prevents a reader from
    trusting a wrong resolver IP.

    DEFERRED (v2): special-case `.ts.net` (and the tailnet reverse zones) to
    forward to 100.100.100.100 instead of the router, which would restore
    Tailscale MagicDNS under `direct` with no loop (Tailscale is authoritative for
    its own zone, so it answers rather than re-forwarding). Not done in v1 — the
    `direct` path is opt-in and the clean recommendation is systemd-resolved.
    NON-GOAL: actively reconfiguring Tailscale itself.

    ENFORCEMENT: unit tests (reconcile.py's `test` check) — is_overlay_resolver
    matches the whole 100.64/10 range and nothing else; parse_resolv_nameservers
    drops ALL 100.64/10 addresses (100.100.100.100 and 100.100.100.53) while
    keeping a real router IP (192.168.1.1). recover_real_upstreams (file read) and
    the true two-daemon loop are not unit-testable in-process (per DNS-003), so a
    live xps + Tailscale re-test in `magic-dns direct` remains the integration gate.
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


# --------------------------------------------------------------------------
# Requirement: CI/release workflow identity (RENAME-012) and correctness (CI-001)
# --------------------------------------------------------------------------

class ReleaseWorkflowBuildIdentity(Requirement):
    """REQUIREMENT-ID: RENAME-012

    Found 2026-07-08 while setting up GitHub Releases so remote test machines
    can fetch a prebuilt binary instead of building from source. `.github/
    workflows/release.yml` and `nightly.yml` were inherited from upstream
    verbatim and never adapted past the binary rename: both packaging steps do
    `BINARY=target/<matrix target>/release/ray`, but this fork's
    `Cargo.toml` renamed the bin target to `torpedo` — the `cp` fails
    ("No such file or directory") the moment either workflow actually runs.
    Fix: `ray` -> `torpedo` in both `Package for release` steps.

    Also renamed for consistency (these are OUR OWN fork's release artifacts,
    downloaded manually since self-update is disabled — see the carve-out
    below for why this is safe): the Linux/macOS asset names
    (`ray-linux-x86_64` -> `torpedo-linux-x86_64`, `ray-linux-aarch64` ->
    `torpedo-linux-aarch64`, `ray-macos-aarch64` -> `torpedo-macos-aarch64`,
    `ray-macos-x86_64` -> `torpedo-macos-x86_64`) and the Android artifact
    (`rayfish-android.apk` -> `torpedo-android.apk`, in both `release.yml` and
    `nightly.yml`). `nightly.yml`'s release-notes body also told users to
    "Install with `ray update --nightly`" — misleading since self-update is
    neutralized in this fork (CON-006) — replaced with a plain
    download-the-asset instruction.

    Deliberately NOT touched (do not "fix" this on a future pass): `src/
    update.rs`'s `release_asset_name` (`ray-{os}-{arch}`) and the matching
    literals in `src/main.rs`, which RENAME-011 already carved out on purpose.
    That code names asset filenames on **upstream's** rayfish/rayfish releases
    (the disabled self-updater's `REPO_SLUG` target, kept per CON-006) — a
    different `ray` than this class's, and renaming it would make a
    hypothetically re-enabled updater look for an asset upstream does not
    publish. This class's renames are entirely on the fork's own
    ErikAllanKincaid/torpedo release assets and do not interact with that code
    path at all.

    ENFORCEMENT: none — YAML workflow files, not `src/**/*.rs`, so CON-007's
    curated-token grep does not (and should not) cover them, same rationale as
    the justfile identity item (TODO.md). Verified by reading the diff and
    (once triggered) an actual Actions run producing correctly-named assets.
    """
    req_id = "RENAME-012"


class ReleaseWorkflowsActuallyRun(Requirement):
    """REQUIREMENT-ID: CI-001

    Found 2026-07-08, same audit as RENAME-012. `ci.yml` and `nightly.yml`
    both trigger on `push: branches: [master]`, but this repo's default
    branch is `main` (confirmed: local `main` tracks `origin/main`). Neither
    workflow has ever fired on an ordinary push to this fork — `ci.yml` only
    ran (if at all) via its unfiltered `pull_request:` trigger, and
    `nightly.yml` has no such fallback, so the rolling `nightly` pre-release
    has never been produced automatically. `reconcile.py`, run locally, has
    been the only gate exercised so far; GitHub Actions itself has likely
    never executed on this fork.

    Fix: `branches: - master` -> `branches: - main` in both workflows' `on:
    push:` blocks. `release.yml` is unaffected (it triggers on tag push /
    `workflow_dispatch`, not a branch push).

    ENFORCEMENT: none — YAML workflow files, same rationale as RENAME-012.
    Verified by reading the diff and (once pushed) an actual triggered run.
    """
    req_id = "CI-001"


class ReleaseWorkflowLinuxOnlyForNow(Requirement):
    """REQUIREMENT-ID: CI-002

    Decided 2026-07-08 while fixing RENAME-012/CI-001: `release.yml` and
    `nightly.yml` build Linux, macOS, and Android artifacts, but only Linux
    (`torpedo-linux-x86_64`, `torpedo-linux-aarch64`) is actually ready to
    ship. Neither of the other two platforms is safe or complete to publish:

    - **macOS**: `route_peer_range`/`route_self_loopback` in `src/tun.rs`
      still hardcode the old `100.64.0.0/10` range and ignore `--subnet`
      (TODO.md "macOS rewrite"), and no `#[cfg(macos)]` code is compiled or
      type-checked on any Linux CI runner or dev host in this project. A
      released macOS binary would silently misroute a real Mac's network
      config — unacceptable to publish to actual users' machines.
    - **Android**: the deep-link scheme is actively broken (manifest still
      `rayfish://` vs. the Rust side's `torpedo://`), plus the outstanding
      Kotlin/package identity rename and `ray-mobile` subnet-agnosticism
      (TODO.md "Android rewrite").

    Whether to finish these platforms or drop them entirely is undecided.
    Rather than delete the job definitions (losing the working matrix/build
    steps) or leave them silently broken, both are kept in the workflow files
    — with RENAME-012's identity fixes already applied so they are correct
    the moment they're reactivated — but gated `if: false` at the job level
    (`build-macos` in both workflows; `android` in both workflows), each with
    a comment citing this ID (CI-002) for the rationale. Only
    the `build` job (Linux matrix) and the Android/macOS-free `create-release`
    / `roll-tag` jobs actually run.

    ENFORCEMENT: none — YAML workflow files, same rationale as RENAME-012/
    CI-001. Verified by reading the diff (both disabled jobs present with
    `if: false`) and, once triggered, that only Linux assets appear on a
    release.
    """
    req_id = "CI-002"


class NightlyWorkflowManualOnly(Requirement):
    """REQUIREMENT-ID: CI-003

    Decided 2026-07-08, right after CI-001 fixed `nightly.yml`'s dead
    `push: branches: [master]` trigger to `main`. On reflection, an automatic
    push trigger is the wrong default for this project's actual commit
    pattern: many pushes are doc/spec/TODO-only (this session alone landed
    several), and each would have silently kicked off a full rebuild + moved
    the shared `nightly` tag the moment CI-001 made the trigger live.

    Fix: `nightly.yml`'s `on:` block is now `workflow_dispatch:` only — no
    `push:` trigger at all. A nightly build now happens only when explicitly
    requested (Actions tab -> Nightly -> "Run workflow", or `gh workflow run
    nightly.yml`), against whichever branch/ref is chosen at dispatch time
    (defaults to `main`). `release.yml` is unaffected — it already triggers on
    tag push / manual dispatch, not branch push, so it never had this problem.

    A `push` + `paths:` filter (only rebuild when `src/**`/`Cargo.toml`/
    `Cargo.lock`/the workflow file itself changes) was considered as an
    alternative that keeps some automation while filtering out doc-only
    noise; deferred in favor of full manual control while this pipeline is
    still new and untrusted. Revisit once the pipeline has a track record.

    ENFORCEMENT: none — YAML workflow file, same rationale as RENAME-012/
    CI-001/CI-002. Verified by reading the diff (no `push:` key under `on:`)
    and, once tried, that pushing to `main` alone does NOT start a run while
    "Run workflow" does.
    """
    req_id = "CI-003"


class SecurityPolicyIdentityAndReportingFix(Requirement):
    """REQUIREMENT-ID: RENAME-013

    Found 2026-07-08, same review pass that recovered a `SECURITY.md`
    unexpectedly missing from disk (a pre-existing unstaged working-tree
    deletion unrelated to this session's edits) and read it once restored.
    The file was upstream's own `SECURITY.md`, inherited verbatim and never
    adapted — same pattern as RENAME-012's release workflows, but with a
    sharper edge because this one is functionally misleading, not just
    cosmetically stale:

    - The vulnerability-reporting link pointed at
      `github.com/rayfish/rayfish/security/advisories/new` — upstream's own
      repo, not `ErikAllanKincaid/torpedo`. A real report against this fork
      would have gone to unrelated upstream maintainers who could not act on
      it.
    - The fallback contact was `dario@rayfish.xyz` — upstream's maintainer,
      same misdirection. Distinct from the `Cargo.toml` author-attribution
      carve-out (KEEP-ON-PURPOSE list): that one honestly credits upstream's
      *code*; this one misrouted a fork-specific *bug report* to someone
      unrelated to the fork.
    - `master` branch references (this repo's default is `main`) and a
      `ray report` command reference (binary is `torpedo`).
    - A "Supported versions" table implying a formal release/backport policy
      that this pre-release, unreleased personal fork does not have.

    Fix: the reporting link now points at `ErikAllanKincaid/torpedo`'s own
    private vulnerability advisories page. The upstream email fallback was
    dropped entirely rather than replaced with the operator's own address —
    decision: GitHub private reporting only, no personal email published in a
    public repo file. `master` -> `main`, `ray report` -> `torpedo report`.
    The versions table was replaced with an honest "personal, pre-release
    fork, report against `main`" statement. The "Security model" section
    (identity-based addressing, discovery-vs-admission, signed `GroupBlob`,
    `SO_PEERCRED` IPC auth, secrets-at-rest) was already accurate and is
    unchanged in substance.

    ENFORCEMENT: none — Markdown, not `src/**/*.rs`, same rationale as
    RENAME-012. Verified by reading the diff.
    """
    req_id = "RENAME-013"


# --------------------------------------------------------------------------
# Requirement: documentation accuracy, not identity (DOC-*)
# --------------------------------------------------------------------------

class DocsMatchCurrentBinaryAndSubnetFormula(Requirement):
    """REQUIREMENT-ID: DOC-001

    Found/fixed 2026-07-08, the two remaining items from TODO.md's doc-fix
    list. Distinct from the `RENAME-*` series: neither of these is stale
    `rayfish` identity, they are plain factual drift between AGENTS.md/
    TESTING.md and the current binary/formula.

    (1) **Hardcoded resolver IP.** AGENTS.md stated the Magic DNS resolver
    address as the fixed literal `100.100.100.53` in four places (the
    KEEP-ON-PURPOSE list, and the `forward.rs`/`dns.rs`/`dns_config.rs` module
    descriptions). Since SUBNET-007/008 this has been subnet-derived
    (`dns::magic_dns_v4`) — `10.88.100.53` on the default `10.88.0.0/16`,
    `10.99.100.53` on a `10.99.0.0/16` network, etc. — and was never a fixed
    value to begin with once that change landed. Fixed to describe the
    formula + default-subnet example instead of the stale literal.
    `DESIGN.md`'s mention was already correctly historical ("instead of the
    fixed 100.100.100.53") and needed no change; `TESTING.md`'s Results-log
    mention was likewise already a correct, dated finding and was left as-is.

    (2) **Invite CLI audit — the binary was right, the diagnosis was wrong.**
    TODO.md/TESTING.md's "attempt 1" finding claimed AGENTS.md documents
    invite flags (`--hostname`/`--expires`/`--qr`/`--reusable`/`list`/
    `revoke`) that the binary lacks. Reading `InviteAction` in `src/main.rs`
    and its dispatcher in `src/cli/invite.rs` shows all of them exist and
    match AGENTS.md's description. The actual bug: those flags belong to an
    explicit `create` subcommand variant, and clap will not parse
    subcommand-specific flags unless that subcommand word is present in
    argv — `torpedo invite testnet --hostname X` (no `create`) genuinely
    errors "unexpected argument", while `torpedo invite testnet create
    --hostname X` works. AGENTS.md's compact CLI reference omitted the
    `create` keyword, reading as if the flags attached to the bare `invite
    <net>` form; so did TESTING.md's Stage 3, Stage 12, and the hostname-change
    flow description. All four corrected to show `create` explicitly. The
    original TESTING.md finding was left in place (it accurately records what
    happened during that test run) with a follow-up note appended correcting
    the diagnosis, rather than rewritten, so the history of "what we thought
    was wrong vs. what actually was wrong" stays visible.

    ENFORCEMENT: none — Markdown, not `src/**/*.rs`. Verified by reading the
    diff and cross-checking against `src/main.rs`/`src/cli/invite.rs`/
    `src/dns.rs`.
    """
    req_id = "DOC-001"


class ReportAndRepoSurfaceIdentityRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-014

    Sibling of RENAME-011, but for the `rayfish` **product name** (not the
    `ray` binary short-name RENAME-011 handled) leaking into the diagnostic /
    reporting / repo surface — files RENAME-006..011 never touched. Found via
    the 2026-07-10 tree-wide `ray|rayfish` audit (Workstream A). Each is a
    LIVE, user-facing string that self-identifies the fork as upstream:

    - `src/daemon/mesh/diagnostics.rs` — `torpedo report` is active (unlike
      self-update). Renamed the sysinfo banner (`"rayfish {version}"`), the
      report bundle filename (`/tmp/rayfish-report-{ts}.tgz` — also a
      collision-prone host artifact: a genuine rayfish on the same host would
      write the same /tmp name), and the pre-filled GitHub issue title (both
      the crash and non-crash branches) + body header — all `rayfish` ->
      `torpedo`. Every bug report a user files currently mislabels itself.
    - `.github/ISSUE_TEMPLATE/bug_report.yml` + `feature_request.yml` — the
      user-facing issue forms said `rayfish` and used `ray <cmd>` examples.
      The load-bearing fix: bug_report told reporters logs live in
      `/var/log/rayfish` / `/Library/Logs/rayfish` — the WRONG directories
      (real paths are `/var/log/torpedo`, `/Library/Logs/torpedo`, per
      `logdir.rs`). Both `rayfish` -> `torpedo` and `ray <cmd>` -> `torpedo
      <cmd>` throughout (issue templates are user-facing, so RENAME-011's
      source-comment carve-out does not apply).
    - `cliff.toml` — the changelog "Full Changelog" compare link was
      hardcoded to `github.com/rayfish/rayfish/compare/...`, rendering an
      upstream URL into this fork's published release notes. Repointed to the
      fork repo (`github.com/ErikAllanKincaid/torpedo`, matching
      `status.rs`'s `REPORT_REPO_URL`). Distinct from the KEEP-ON-PURPOSE
      `REPO_SLUG = "rayfish/rayfish"` (self-update target, CON-006) — that
      names upstream on purpose; this one is our own changelog. Also fixed
      `CHANGELOG.md`'s header line ("All notable changes to Rayfish" ->
      "Torpedo"); the changelog *body* keeps its historical `ray <cmd>`
      entries (RENAME-011's deferred cosmetic class, not rewritten).
    - `src/firewall.rs` — folded in: a comment claimed `firewall.toml` is
      `0640 root:rayfish`; the real group is `torpedo` (`groupadd torpedo`,
      RENAME-002). Comment-only, but it misdescribed actual file permissions.

    All literal string swaps, no behavior change: verified that nothing parses
    the bundle filename or sysinfo line (display-only), no test asserts these
    strings, and the issue templates/cliff URL are consumed only by GitHub /
    git-cliff rendering.

    Deliberately EXCLUDED: source doc-comments still saying `ray <verb>` /
    `rayfish` (RENAME-011's deferred cosmetic carve-out, Workstream C); the
    Prometheus metric names `rayfish`/`rayfish_peer` in `src/stats.rs`
    (Workstream B — a metric rename breaks existing scrapers, needs its own
    decision); test fixtures (`rayfish-test-`, `rayfish 0.1.0`) which do not
    reach users.

    ENFORCEMENT: see CON-009 (curated-token anti-regression gate).
    """
    req_id = "RENAME-014"


class NoResidualReportIdentityLeak(Constraint):
    """CONSTRAINT-ID: CON-009

    Anti-regression gate for RENAME-014, same curated-token approach as
    CON-007/CON-008 but spanning a file set neither covers: the Rust source
    report path (`src/**/*.rs`) PLUS the release/repo tooling `.github/**` and
    `cliff.toml`. Curated so it never false-positives on KEEP-ON-PURPOSE names
    (the kept `REPO_SLUG` `rayfish/rayfish` has no `/compare` suffix; the relay
    presets, crate name, and author attribution are all different tokens) and
    never trips on RENAME-011's deliberately-deferred `ray <verb>` comments
    (those are the `ray` short-name, not these `rayfish`/path tokens).

    Tokens: `rayfish-report`, `root:rayfish`, `rayfish {version}` (src report
    strings); `/var/log/rayfish`, `/Library/Logs/rayfish` (issue-template log
    paths); `rayfish/rayfish/compare` (cliff changelog link).

    ENFORCEMENT (reconcile.py): report_identity.unexpected_count equals 0.
    """
    constraint_id = "CON-009"
    enforcement_logic = "{{ report_identity.unexpected_count == 0 }}"


class ObservabilityIdentityRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-015

    Workstream B of the 2026-07-10 `ray`/`rayfish` audit: the last observability
    identifiers still naming upstream. Distinct from RENAME-011 (`ray` binary
    short-name) and RENAME-014 (report/repo surface) — these are the names a
    monitoring stack sees:

    - `src/stats.rs` — the iroh-metrics group names drive the exported Prometheus
      family prefix on `:9090`. `#[metrics(name = "rayfish")]` (ForwardMetrics) ->
      `"torpedo"` and `#[metrics(name = "rayfish_peer")]` (PeerMetrics) ->
      `"torpedo_peer"`, so series export as `torpedo_packets_rx`,
      `torpedo_peer_rtt_us`, etc.
    - `src/main.rs` — the `otel` feature's OTLP span identity:
      `.with_service_name("rayfish")` and `provider.tracer("rayfish")` -> `"torpedo"`.
    - `src/daemon/mesh/bootstrap.rs` — folded in: the metrics-server doc-comment
      "Register rayfish counters" -> "torpedo" (self-consistency with the rename).

    `torpedo` is already the fork convention (`bootstrap.rs`'s mDNS
    `service_name("torpedo")`, unchanged). NOTE (breaking-only-after-release):
    renaming a metric family / OTLP service name breaks existing scrapers,
    dashboards, and collector filters — but this fork is pre-release with no such
    consumers, so the change is free NOW and only becomes breaking post-release.
    No back-compat alias added for that reason.

    ENFORCEMENT: CON-007 gains the four observability tokens (`name = "rayfish"`,
    `name = "rayfish_peer"`, `service_name("rayfish")`, `tracer("rayfish")`).
    Additionally locked by the `stats::tests::metrics_export_under_torpedo_prefix`
    unit test, which encodes both groups through `iroh_metrics::Registry` and
    asserts the rendered OpenMetrics text carries the `torpedo`/`torpedo_peer`
    prefixes and no `rayfish` — the exported prefix is otherwise a compile-time
    derive constant with no runtime assertion.
    """
    req_id = "RENAME-015"


class SourceCommentCliNameSwept(Requirement):
    """REQUIREMENT-ID: RENAME-016

    Workstream C of the `ray`/`rayfish` audit: the cosmetic source-comment
    residue RENAME-009 and RENAME-011 deliberately DEFERRED ("left for a later
    opportunistic pass"). Finishing it here so the fork reads consistently and,
    critically, so a coding agent reading a comment does not emit a `ray <verb>`
    that no longer exists.

    Two parts:

    (1) **`ray <verb>` CLI/binary references (217 across 44 src files).** Every
    occurrence of the pre-fork binary name `ray` followed by a subcommand (or
    the "run ray without sudo" prose) reworded to `torpedo`, in doc-comments,
    line comments, AND the dead `cli/update.rs`/`update.rs` string tail that
    RENAME-011 left behind the `SELF_UPDATE_ENABLED` early-return. Sweeping the
    dead tail too is what makes the CON-010 gate viable (RENAME-011 had rejected
    a gate precisely because those strings still held `ray <verb>`). Applied by
    the lookbehind regex `(?<![.\\w-])ray (?=[a-z])`, which by construction skips
    every KEEP form: `.ray` (Magic-DNS TLD), `ray-proto`/`ray-mobile` (crate
    names), `stingray`/`array` (substrings), `rayfish` (crate/preset), and the
    `"ray"` network-name wordlist entry. `ray-{os}-{arch}` upstream release
    asset names (hyphenated) are untouched.

    (2) **`rayfish` product-name prose in comments (9 of 24 candidates).** The
    9 that describe THIS fork's own daemon/behavior reworded to `torpedo`
    (`daemon/mod.rs` "The rayfish daemon", `firewall.rs` "rayfish/iroh control
    plane", `transport.rs` data-plane shape, `cli/firewall.rs` "the rayfish
    firewall", `cli/status.rs` header example, `invite.rs` `~/.config/rayfish`
    path, `apply.rs` hostname note). The other 15 are KEEP: they name UPSTREAM
    deliberately (coexistence comments in `dns_config.rs`/`deeplink.rs`/
    `status.rs`, the `rayfish`-operated preset URLs in `config.rs`, the
    `RAYFISH_CONFIG_DIR` collision note, the `rayfish/n0` preset keyword).

    No behavioral effect: comments and one unreachable dead-code string tail;
    build/clippy/test unaffected. No CHANGELOG entry (pure-internal).

    ENFORCEMENT: CON-010 gates part (1) — the clean, recurring class. Part (2)
    is NOT gated: a `rayfish`-prose gate cannot be made false-positive-free
    given the many legitimate `rayfish` tokens (crate, preset, REPO_SLUG,
    attribution, deliberate upstream mentions), so it is verified by reading.
    """
    req_id = "RENAME-016"


class NoResidualCliNameLeak(Constraint):
    """CONSTRAINT-ID: CON-010

    Anti-regression gate for RENAME-016 part (1) and RENAME-017: the pre-fork
    `ray <verb>` binary reference must not reappear in `src/**/*.rs` OR the
    `tests/` harness (extended to cover tests/ in RENAME-017). Regex, not a token
    list — `(?<![.\\w-])ray (?=[a-z])` — so it matches a bare `ray ` + lowercase
    word (always a stale CLI reference) while the lookbehind excludes every
    KEEP form (`.ray` TLD, `ray-proto`/`ray-mobile`, `stingray`/`array`,
    `rayfish`). This is the gate RENAME-011 could not add until Workstream C
    also swept the dead `cli/update.rs` string tail (its last false-positive
    source). Does not cover `rayfish` product-name prose (RENAME-016 part 2,
    ungated — see that requirement).

    ENFORCEMENT (reconcile.py): cli_reference_identity.unexpected_count equals 0.
    """
    constraint_id = "CON-010"
    enforcement_logic = "{{ cli_reference_identity.unexpected_count == 0 }}"


class TestHarnessIdentitySwept(Requirement):
    """REQUIREMENT-ID: RENAME-017

    Workstream D of the `ray`/`rayfish` audit: the e2e/bench harness under
    `tests/` (16 shell scripts + 11 READMEs). Unlike RENAME-016's src comments,
    this is a FUNCTIONAL fix — the scripts RUN against the deployed binary, and
    `deploy_all` uses `just deploy` (which installs the `torpedo` binary +
    service, no `ray` symlink), so every stale reference silently breaks or
    no-ops the test rather than being cosmetic. Confirmed-broken cases:

    - `on "$ip" 'ray <cmd>'` invocations (303 across tests/) → `command not
      found: ray`. Reworded to `torpedo` via the same lookbehind regex as
      RENAME-016 (`.ray` TLD, `ray-`, `rayfish` all excluded).
    - `reset_state` ran `systemctl stop rayfish; rm -rf /etc/rayfish
      /root/.config/rayfish` — a NO-OP against the `torpedo` service/paths, so
      state was never actually reset between runs. → torpedo.
    - `dns/run.sh` grepped `/etc/resolv.conf` for `"Added by rayfish"`, but the
      binary writes `# Added by torpedo` (`src/dns_config.rs`) — the direct-mode
      detection never matched. → torpedo.
    - `unpair` referenced the pkarr record `_rayfish_certgen`; the binary
      publishes `_torpedo_certgen` (`src/dht.rs`). Bench comment cited ALPN
      `rayfish/files/1`; real is `torpedo/files/1` (`src/transport.rs`). Invite
      helpers parsed CLI output for the literal `ray join`/`ray invite` strings
      the binary now prints as `torpedo`. → torpedo.
    - Cosmetic prose + bench comparison labels (`rayfish` vs direct, orchestrator
      comments) reworded uniformly; the `bench_pair "rayfish"` label arg and all
      its `get/ratio ... rayfish` lookups renamed together so the keying stays
      consistent.

    KEEP (unchanged): the `.ray` Magic-DNS TLD in every hostname/regex; and the
    `NAMES=(rayfish-*)` Scaleway instance labels (bare `rayfish`, retained — they
    are opaque ephemeral cloud-VM identifiers with an operational orphan cost and
    zero correctness benefit, the same rationale as keeping the crate name).
    Applied by skipping `NAMES=(` lines in the sweep.

    NOT in scope (separate pre-existing drift, flagged for follow-up): the
    `100.64.x.x` / `100.64.0.0/10` CGNAT range still cited in several bench/
    common.sh comments — a SUBNET doc-drift (default is now `10.88.0.0/16`),
    unrelated to this rename.

    Verified: `bash -n` parses every `tests/**/*.sh`; the full e2e run itself
    needs 3 provisioned cloud hosts and was NOT executed here.

    ENFORCEMENT: CON-010 extended to also scan `tests/` for the `ray <verb>`
    regex; CON-011 (below) curated-token gates the functional `rayfish`
    service/config/marker/record identity. Cosmetic prose is ungated (same
    reason as RENAME-016 part 2).
    """
    req_id = "RENAME-017"


class NoResidualTestHarnessIdentityLeak(Constraint):
    """CONSTRAINT-ID: CON-011

    Anti-regression gate for RENAME-017: the functional pre-fork `rayfish`
    identity must not reappear in the `tests/` harness. Curated token set
    (`systemctl {stop,start,restart} rayfish`, `/etc/rayfish`,
    `/root/.config/rayfish`, `Added by rayfish`, `_rayfish_certgen`,
    `rayfish/files/1`) — NOT a bare `rayfish` grep, so it never trips on the
    KEEP `NAMES=(rayfish-*)` Scaleway instance labels or the `.ray` TLD. Mirrors
    CON-008's approach (build/deploy tooling) but for the test harness, which no
    other gate covers. The `ray <verb>` CLI class is handled by CON-010's
    tests/-extended regex, not here.

    ENFORCEMENT (reconcile.py): test_harness_identity.unexpected_count equals 0.
    """
    constraint_id = "CON-011"
    enforcement_logic = "{{ test_harness_identity.unexpected_count == 0 }}"


class TestHarnessSubnetUpdated(Requirement):
    """REQUIREMENT-ID: SUBNET-015

    Found while doing RENAME-017 (2026-07-10): the `tests/` harness still assumed
    upstream's `100.64.0.0/10` CGNAT range and the pre-fork fixed magic-DNS IP
    `100.100.100.53`, both changed by the fork's core purpose — the default
    overlay is now `10.88.0.0/16` (SUBNET-011) and the resolver is subnet-derived
    to `10.88.100.53` (SUBNET-007/008). Two FUNCTIONAL breaks, not doc drift:

    - `tests/lib/common.sh` `own_ip()` grepped status output for
      `100\\.[0-9]+\\.[0-9]+\\.[0-9]+` — matches nothing in a real `10.88.x.x`
      address, so it returned an empty string and the five tests that derive a
      node's VPN IP from it (device-cert, ssh, unpair, bench, connect) fed empty
      IPs into pings/asserts. Regex → `10\\.88\\.[0-9]+\\.[0-9]+`.
    - `tests/e2e/dns/run.sh` set `MAGIC=100.100.100.53` and queried it; the
      resolver answers at `10.88.100.53`. → `MAGIC=10.88.100.53`.

    Plus 6 comment/README references to `100.64.x.x` / `100.64.0.0/10` /
    `100.100.100.53` reworded to the `10.88` reality. No test sets a custom
    `--subnet`, so the exact `10.88` literals are correct for the whole suite.

    ENFORCEMENT: CON-012 (below). Distinct from CON-002 (`grep_hardcoded_cgnat`),
    which polices the same drift in `src/` (membership/tun/dns).
    """
    req_id = "SUBNET-015"


class NoResidualTestCgnatLeak(Constraint):
    """CONSTRAINT-ID: CON-012

    Anti-regression gate for SUBNET-015: the `tests/` harness must not carry the
    pre-fork `100.64` CGNAT range or the `100.100.100` magic-DNS IP — literals
    that make `own_ip` extract nothing and the DNS test hit the wrong address.
    Counts both; must be 0. tests/-scoped counterpart to CON-002's src scan.

    ENFORCEMENT (reconcile.py): test_subnet_identity.unexpected_count equals 0.
    """
    constraint_id = "CON-012"
    enforcement_logic = "{{ test_subnet_identity.unexpected_count == 0 }}"


# ==========================================================================
# torpedo-min: the minimal variant (MINIMAL-*, CON-M*)
#
# This repository is torpedo-min, a clone of full torpedo at 4809edb that
# strips the product down to its core: connect machines into a private mesh
# with stable identity-derived addresses. See PROPOSAL.md for the rationale
# and design decisions D1-D6, PLAN.md for the commit-by-commit execution
# order. Inherited SUBNET-*/RENAME-*/CON-* specs above remain valid until a
# MINIMAL removal commit retires them explicitly. New constraints use the
# CON-M* namespace so future full-torpedo CON-0xx numbers never collide on
# cherry-pick.
# ==========================================================================


class MinimalIntent(UserStory):
    """USER-STORY: MINIMAL-INTENT

    Strip torpedo to a single-purpose tool that connects machines into a
    private mesh network, delegating firewalling, name resolution, file
    transfer, remote shells, and updates to the host tools that already do
    those jobs well, while remaining wire-compatible with full torpedo.

    Priority: high.
    User journey: install torpedo-min on two machines -> create a network on
    one -> join from the other -> approve the join -> reach the peer by its
    mesh IP from `torpedo status` -> filter traffic with nftables on the TUN
    interface if desired.
    Acceptance: the CLI exposes exactly the surface in PROPOSAL.md; the main
    crate is roughly 15k lines; a torpedo-min node and a full torpedo node
    interoperate on one network; the trimmed e2e harness is green.
    """
    brief_title = "Minimal connect-only variant"
    priority = "high"


# --------------------------------------------------------------------------
# Requirements: scope and removals (MINIMAL-*)
# --------------------------------------------------------------------------

class MinimalScope(Requirement):
    """REQUIREMENT-ID: MINIMAL-001

    torpedo-min provides exactly: identity, membership, mesh transport, TUN
    forwarding, closed-network admission with live approval, and a plain CLI
    (create/join/leave/nuke/requests/accept/deny/admin/kick/status/up/down/
    config/completions/version plus the sudo service verbs). Policy
    enforcement, naming, file transfer, remote shells, diagnostics probes,
    self-update, and multi-device identity are out of scope. Wire
    compatibility with full torpedo is preserved (see CON-M02): protocol
    version 1, unchanged ALPNs, unchanged GroupBlob schema; control messages
    a min node no longer initiates are answered passively or ignored on
    receipt, never a decode error or connection close.
    """
    req_id = "MINIMAL-001"


class RemoveSelfUpdate(Requirement):
    """REQUIREMENT-ID: MINIMAL-002

    Remove the self-update machinery entirely: src/update.rs,
    src/cli/update.rs, the `update`/`auto-update` CLI and the
    `install --auto-update` flag, and the deps it alone pulls (reqwest, the
    direct rustls handle, self-replace, sha2, semver). Full torpedo already
    ships it disabled (CON-006); in torpedo-min absence replaces the gate,
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
    ingress check. Packet filtering is nftables/ufw's job on the TUN
    interface; README states the posture change (every mesh peer reaches
    every port) loudly, with the nftables equivalent. Wire compat (D1):
    GroupBlob keeps its suggested_firewall field; reconverge ignores it and
    coordinator republish preserves it verbatim; ray-proto policy.rs types
    stay.
    """
    req_id = "MINIMAL-010"


class RemoveApplyLayer(Requirement):
    """REQUIREMENT-ID: MINIMAL-011

    Remove the declarative apply layer (which exists to push firewall specs
    and dies with MINIMAL-010): apply.rs, cli/alias.rs,
    daemon/mesh/alias.rs, `torpedo identityof`, and EXAMPLE_SPEC.
    Fleet reconciliation is a script over `torpedo status --json`.
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
    """
    req_id = "MINIMAL-012"


class ApprovalOnlyAdmission(Requirement):
    """REQUIREMENT-ID: MINIMAL-013

    One admission mode: `torpedo create` always makes a Restricted network
    (`--open` removed); joiners land in the pending queue and are admitted
    with `torpedo accept`. Removed: the single-use invite ledger (invite.rs,
    cli/invite.rs, daemon/mesh/invite.rs), invite gossip sending, and
    reusable-key minting. Kept: joiner-side invite-code redemption (a min
    node can still join a full-torpedo network by invite), blob reusable-key
    validation, requests/accept/deny, admin add/list (co-coordinator grant
    is the availability story for admission), and kick. InviteShare/
    InviteUsed from full co-coordinators are ignored on receipt, never an
    error (D1).
    """
    req_id = "MINIMAL-013"


class FixedHostnameNoEphemeral(Requirement):
    """REQUIREMENT-ID: MINIMAL-014

    Remove hostname rename propagation (`torpedo hostname`,
    daemon/mesh/rename.rs, pending_hostname) and the ephemeral auto-kick TTL
    (`torpedo ephemeral`, spawn_stale_member_pruner). Hostname is fixed at
    join; the coordinator still resolves collisions at admission. Manual
    `kick` remains the remediation tool.
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
    Implementation must keep CON-M02 wire compat: policy is node-local
    routing, never a blob/protocol change.
    """
    req_id = "TOR-M01"


# --------------------------------------------------------------------------
# Constraints: torpedo-min gates (CON-M*)
# --------------------------------------------------------------------------

class DependencyAbsenceGate(Constraint):
    """CONSTRAINT-ID: CON-M01

    Anti-regression gate for the MINIMAL removals: Cargo.toml's direct
    dependency sections must not name any dep owned by a removed subsystem:
    reqwest, rustls, self-replace, sha2, semver, russh, pty-process, uzers,
    zbus, inotify, iroh-mdns-address-lookup, indicatif, crossterm,
    unicode-width, humansize, mime_guess, opentelemetry*. (iroh and
    iroh-blobs are core and exempt; iroh-tor-transport is exempt while it
    stays `optional = true` behind the off-by-default `tor` feature, per
    MINIMAL-008/TOR-M01.) Added to reconcile.py once phases 1-2 of PLAN.md
    create the condition it gates.

    ENFORCEMENT (reconcile.py): dependency_absence.unexpected_count equals 0.
    """
    constraint_id = "CON-M01"
    enforcement_logic = "{{ dependency_absence.unexpected_count == 0 }}"


class WireCompatWithFullTorpedo(Constraint):
    """CONSTRAINT-ID: CON-M02

    torpedo-min stays wire-compatible with full torpedo (design decision D1):
    transport::MESH_PROTOCOL_VERSION must equal 1 and the GroupBlob struct in
    src/membership.rs must retain its `suggested_firewall` and
    `reusable_keys` fields (ignored/preserved, never enforced or minted).
    Breaking this silently severs mixed min/full networks.

    ENFORCEMENT (reconcile.py): wire_compat.mesh_version equals 1 and
    wire_compat.blob_fields_present is true.
    """
    constraint_id = "CON-M02"
    enforcement_logic = "{{ wire_compat.mesh_version == 1 and wire_compat.blob_fields_present }}"
