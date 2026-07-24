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
# Follow-up round: node subnet at boot (SUBNET-009/010).
# (UPGRADE-001 / CON-006 — the self-update requirement and its kill-switch
# constraint — were RETIRED by MINIMAL-002: tetron deletes the machinery
# outright, so absence replaces the gate.)
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


class DefaultSubnetSafe(Requirement):
    """REQUIREMENT-ID: SUBNET-011

    The built-in default overlay subnet (membership::default_subnet, used when a
    GroupBlob's / config's subnet is None) changes from 100.64.0.0/10 to
    10.88.0.0/24 — an uncommon 10.x slice deliberately chosen NOT to collide
    with Tailscale's 100.64.0.0/10, so a no-flag `tetron create` coexists with
    Tailscale out of the box. `--subnet` / `config set subnet` still override it.
    A /24 gives 256 host addresses, enough for personal/team meshes; users who
    need more can set a larger prefix explicitly.
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
    subnet (10.88.0.0/24), not the old 100.64.0.0/10 that SUBNET-011 replaced:
    - `tetron create --subnet` CLI help (src/main.rs) says the default is
      10.88.0.0/24.
    - The GroupBlob.subnet (src/membership.rs) and AppConfig.subnet
      (src/config.rs) field docs, and the IPC Create.subnet doc
      (tetron-proto/src/ipc.rs), describe `None` as the 10.88.0.0/24 default.

    Explicitly OUT OF SCOPE (documented deferrals, not the fork's Linux path,
    decision left for later): the macOS `route_peer_range` branch (src/tun.rs),
    the Android VpnService (android/), and the upstream e2e/bench shell harnesses
    (tests/) still assume 100.64.0.0/10. They are adapted or removed in a future
    project, not here.
    """
    req_id = "SUBNET-013"


# --------------------------------------------------------------------------
# Thorough-fork round: purge residual `rayfish` identity from host-visible
# artifacts and cosmetics (RENAME-007..009 / CON-007). Distinct from the
# KEEP-ON-PURPOSE names (upstream relay/discovery presets, REPO_SLUG, the
# `.ray` TLD, the internal Cargo crate name `rayfish`), which CON-001 and the
# honesty rationale explicitly protect and which this round must NOT touch.
# --------------------------------------------------------------------------

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
    fork (github.com/ErikAllanKincaid/tetron): Cargo.toml +
    ray-proto/Cargo.toml `repository`/`homepage`, the ray-proto `description`,
    and REPORT_REPO_URL (src/cli/status.rs) so `torpedo report` opens an issue on
    the fork's tracker, not upstream's. No behavioral effect on the mesh; done
    opportunistically in files already edited by RENAME-007..008.

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

    After RENAME-007..008 (and RENAME-015's observability names), none of the
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

    RENAME-007..009 renamed host artifacts, wire identifiers, and doc-comment/
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
    Verified by reading the diff, same as RENAME-007..009.
    """
    req_id = "RENAME-011"


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

    **Addendum, 2026-07-18 (0.3.0): the macOS half of this gate is
    resolved.** `MACOS-001` (the exact hardcoded-`100.64.0.0/10` bug named
    above) and `MULTISEG-008` (a second, deeper bug the first one's fix
    exposed — a member's locally-tracked subnet reverting to the node-wide
    default on reconnect, present since multi-segment TUN shipped in
    0.2.0) are both fixed, and macOS has now actually been live-verified
    on real Apple Silicon hardware — joined a live network, confirmed
    working over IPv4 and IPv6 (ping + real file transfer, both
    directions), including surviving a `down`/`up` standby cycle. `if:
    false` is removed from `build-macos` in both `nightly.yml` and
    `release.yml` as of this release. **Android is unaffected and remains
    gated off** — its blockers (deep-link scheme mismatch, Kotlin/package
    identity rename) are unrelated and still unresolved.
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
    reporting / repo surface — files RENAME-007..011 never touched. Found via
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
      fork repo (`github.com/ErikAllanKincaid/tetron`, matching
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


# --------------------------------------------------------------------------
# SUBNET-BUG-001: TUN created with local subnet, not network's subnet
# --------------------------------------------------------------------------

class SubnetMismatchOnJoin(Requirement):
    """REQUIREMENT-ID: SUBNET-BUG-001

    When a node joins a network whose overlay subnet differs from the node's
    locally configured subnet (from `tetron config set subnet` or the
    default), the TUN device is created with the *local* subnet, not the
    network's authoritative subnet from the `GroupBlob`. The member is
    assigned a mesh IP from the network's subnet (visible in `tetron status`
    and the signed roster), but the TUN interface carries an IP from the
    local subnet. Packets addressed to the member's correct mesh IP arrive
    via QUIC but are written to a TUN whose IP is in a different range --
    the kernel does not recognise the dst IP as local and drops the packet.
    This silently breaks the data plane (no ping, no TCP) with no error
    logged anywhere.

    Fix: reject the join in `join_network_inner` with a clear error
    message when the network's subnet (from `GroupBlob.subnet`) differs
    from the node's local subnet (`config::node_subnet()`). The error
    tells the user to run `sudo tetron config set subnet <network-cidr>
    && sudo tetron restart` and try again before joining. This matches
    the pattern already used by `tetron create --subnet` which rejects a
    `--subnet` that disagrees with the persisted node subnet (lines
    260-264 in create_join.rs).

    The existing persist-on-join code in `finalize_join` (lines 1199-1204
    in create_join.rs) that calls `config::set_node_subnet(joined_subnet)`
    is retained: when subnets already match, it redundantly persists the
    value, ensuring the next restart rebuilds the TUN in the correct
    subnet even if config was somehow reset.

    (Per-network TUN devices or policy routing — option (c) — is the
    correct long-term fix and is documented in SUBNET_COLLISION.md as
    deferred.)

    Found: 2026-07-15, network "shallows" with AORUS (10.77.0.0/24) and
    usbos-1 (10.88.0.0/16). Tested: 2026-07-16, network "test-tetronnet"
    with 590i-aorus-ultra, xps-17-9720, X10SRA, xeon40 (10.55.55.0/24).
    """
    req_id = "SUBNET-BUG-001"


# --------------------------------------------------------------------------
# CONVERGE-001: Co-coordinator publish race
# --------------------------------------------------------------------------

class CoCoordinatorPublishRace(Requirement):
    """REQUIREMENT-ID: CONVERGE-001

    When a promoted co-coordinator admits new members and publishes an
    updated blob to the DHT, the original coordinator may overwrite it with
    a stale blob on its 300s periodic publish timer. The cascade:

    1. Co-coordinator publishes updated blob (members: orig + co + new1 + new2)
    2. Original coordinator's 300s timer fires, publishes stale blob
       (members: orig + co only) to the SAME DHT key
    3. Co-coordinator's 60s group poller sees DHT hash regressed, fetches
       old blob, overwrites its in-memory state
    4. Members admitted by co-coordinator vanish from both coordinators

    Root cause: multiple coordinators publish to the same DHT key without
    coordination. The lazy publisher on co-coordinators has no dht_notify
    handle and uses polling (10s). The original coordinator's publisher
    overwrites the DHT on notify or 300s timer, regardless of whether the
    DHT already has a newer blob.

    Fix (read-before-write):

    Both `spawn_network_publisher` (original coordinator) and
    `spawn_lazy_publisher` (co-coordinator) add a DHT read before each
    publish. The rule:

    - Track `last_published_hash` (the hash we most recently published).
    - Before publishing, resolve the current DHT record via
      `dht::resolve_network` using the client + network public key
      (derived from `net_secret_key.public()`).
    - Publish if: `last_published_hash` is None (first publish), OR
      the DHT hash matches `last_published_hash` (no one else has
      published since we did), OR the DHT has no record yet.
    - Skip (do not publish) if: the DHT hash differs from both our local
      hash and `last_published_hash`. This means another coordinator
      published a newer blob. The 60s group poller on all nodes will
      fetch and reconcile it within one cycle.

    This prevents the 300s timer from ever overwriting a newer blob. When
    the original coordinator's timer fires but the DHT hash differs from
    `last_published_hash`, the publisher skips the cycle.

    The coordinator MUST also run a group poller (spawned in
    `spawn_coordinator_background_tasks`) to discover blob updates from
    co-coordinators. Without it, the coordinator's in-memory state is
    permanently stale if only co-coordinators publish changes.

    Found: 2026-07-16, e2e test with aorus (original coordinator) and
    xps-17-9720 (co-coordinator) on network "test-tetronnet"
    (10.55.55.0/24).
    """
    req_id = "CONVERGE-001"


# --------------------------------------------------------------------------
# CONVERGE-002: Stale DHT restore on coordinator restart (consequence of
# CONVERGE-001)
# --------------------------------------------------------------------------

class StaleDhtRestore(Requirement):
    """REQUIREMENT-ID: CONVERGE-002

    When the DHT record points to a stale blob (CONVERGE-001), a restarting
    coordinator fails to find the blob bytes at any seed peer and falls back
    to its config file, producing a roster with only the coordinator itself.
    Other members are denied with "no invite presented" because the
    coordinator does not recognize them.

    This is a CONSEQUENCE of CONVERGE-001, not a separate root cause. With
    the CONVERGE-001 (read-before-write) fix, the DHT record always points
    to the latest blob, so a restarting coordinator can find and fetch it.

    Additional hardening: if the DHT fallback fails, the restored
    coordinator should trigger an immediate reconverge (not wait 60s) so
    it discovers the latest blob faster.

    Found: 2026-07-16, consequence of CONVERGE-001.
    """
    req_id = "CONVERGE-002"


# --------------------------------------------------------------------------
# CONVERGE-003: Removed member never cleans up locally (ghost member)
# --------------------------------------------------------------------------

class SelfRemovalNoCleanup(Requirement):
    """REQUIREMENT-ID: CONVERGE-003

    A node that is dropped from the authoritative roster (kicked, or a
    casualty of the CONVERGE-001 publish race) never tears itself down
    locally. Two code paths detect "we are no longer a member":

    1. `spawn_group_poller`'s 60s tick: on detecting self-removal it logs
       `we have been removed from the network` and `break`s its own loop —
       nothing else. The reconnect loop keeps running.
    2. `reconverge_and_apply` (the debounced worker driven by `MemberSync`/
       `BlobUpdated` triggers — the path a live `tetron kick` actually
       exercises, well before the 60s poller would notice): it has *no*
       self-removal check at all. It silently applies a roster that
       excludes the local node, with no detection, warning, or cleanup.

    Neither path stops `spawn_reconnect_loop`, removes the network from
    persisted config, or updates `tetron status`. The observable result: a
    removed node keeps its stale config and keeps redialing coordinators,
    each attempt denied with "no invite presented" (it is a fresh unknown
    peer as far as `CoordinatorAcceptState::handle_connection` is
    concerned) in a tight ~5-6s crash loop — while its own `tetron status`
    keeps reporting a healthy, fully-connected membership indefinitely. No
    ping, no ssh, no traffic of any kind actually moves, and nothing
    anywhere logs an error a user would see. This is the same class of
    silent data-plane failure as SUBNET-BUG-001, and it also means a
    legitimately `tetron kick`-ed member was never actually cleaned up
    locally — closing its connection with `KICK_CODE` alone does not stop
    it from redialing forever.

    Fix: extract a shared `member_removed(members, approved, my_id)` check
    used by both `spawn_group_poller` and `reconverge_and_apply`. On
    detecting self-removal, signal the network name over a new `left_tx`/
    `left_rx` mpsc channel (mirroring the existing `promote_tx`/
    `promote_rx` AdminGrant-promotion signal — background tasks hold only
    field clones, not the full `MeshManager`, so they hand off to the main
    daemon loop that does). `serve_ipc` drains `left_rx` and calls
    `MeshManager::handle_removed_from_network`, which runs the same
    teardown as `tetron leave` (cancel the network's token — stopping the
    reconnect loop, poller, and publisher in one step since they all select
    on it — drop peers, unregister the ALPN, delete the network from
    config). `reconverge_and_apply` checks self-removal *before* applying
    the fetched roster to local state, so it never installs a self-less
    membership list in the first place.

    Out of scope for this fix: the CONVERGE-001 publish race itself can
    still let an objectively-stale-but-later-written blob win over a
    genuinely newer admission (no logical/version clock arbitrates the
    two) — that is the root cause of *why* a member can be wrongly
    dropped, tracked separately in docs/TODO.md. This fix addresses the
    local cleanup once removal is (correctly or incorrectly) detected, not
    the DHT race that produces a false removal.

    Found: 2026-07-16, network "converge-test" — X10SRA admitted by
    co-coordinator xps-17-9720, then lost to a CONVERGE-001-style stale
    publish from the original coordinator (590i-aorus-ultra); X10SRA's own
    `tetron status` kept reporting it as a healthy 3-member network for
    25+ minutes with zero working connectivity.
    """
    req_id = "CONVERGE-003"


# --------------------------------------------------------------------------
# CONVERGE-005: Monotonic generation counter closes the CONVERGE-001 publish
# race at its root (raw hash comparison could not tell newer from stale)
# --------------------------------------------------------------------------

class MonotonicBlobGeneration(Requirement):
    """REQUIREMENT-ID: CONVERGE-005

    CONVERGE-001's read-before-write guard (a9b0afa) compared raw DHT hashes:
    it could tell "did the record change under me" but not "is that change
    actually newer." Reproduced live twice more on 2026-07-16 with a9b0afa +
    6b2954d already deployed: once a publisher saw a DHT hash it didn't
    recognize, it deferred to that hash *permanently* — every subsequent
    publish attempt saw the same "mismatch" and skipped, even when the
    publisher's own state was objectively the correct, newer one (it had just
    admitted a member the DHT's blob didn't know about). A slower coordinator's
    stale periodic republish could out-write a co-coordinator's fresher
    admission purely by landing in the DHT later, permanently burying the
    newer state. Root cause: no logical clock, only wall-clock write order.

    A second, compounding gap: `spawn_group_poller`'s blob-fetch only tried
    live `PeerTable` connections over the `iroh-blobs` ALPN, with no seed-peer
    fallback (unlike `reconverge_and_apply`'s `fetch_verified_blob`, which
    tries both). Observed failing with "could not fetch updated group blob
    from any peer" even while a live, traffic-passing mesh connection to the
    same peer was up — so a coordinator could detect a hash change and still
    never converge on it.

    Fix: add `generation: u64` to `GroupBlob` (msgpack, `#[serde(default)]`
    for pre-generation compatibility) and to the signed pkarr network record
    (`g,<n>` TXT field, mirroring the existing `g,<n>` cert-generation-floor
    record in `dht.rs` — same pattern, same file, already precedented).
    `NetworkState` carries the same field: `refresh_snapshot()` recomputes
    hash/bytes from whatever generation is already set (adopting a fetched
    blob sets it directly, never bumped); a new `bump_generation_and_refresh()`
    increments it first, called from every genuine *local* content mutation
    (admit, kick, invite create/revoke, admin grant) instead of plain
    `refresh_snapshot()`.

    `dht_read_before_write` (`publish.rs`) is rewritten around generation, not
    hash: publish whenever the DHT sits at a strictly lower generation than
    ours (regardless of whether we recognize its hash — this is the actual
    fix, closing the permanent-wedge failure mode), defer whenever it's
    strictly higher. An exact generation tie (two coordinators independently
    mutated from the same base) is left alone rather than fought over — the
    loser's own next local mutation bumps past the tie and wins outright,
    rather than the two publishers flip-flopping forever. `spawn_group_poller`
    now gates its fetch on `remote_generation > current_generation` (not raw
    hash inequality) and fetches via `fetch_verified_blob` (peer + seed
    fallback) instead of its own live-peer-only loop, closing the second gap.
    `reconverge_and_apply` adds a defense-in-depth downgrade guard: a freshly
    fetched blob is only applied if `data.generation >= current_generation`,
    so a lagging seed peer's stale copy can never regress local state even if
    it happens to still verify against a signed hash.

    Verified live on 3 bare-metal machines (590i-aorus-ultra, xps-17-9720,
    X10SRA), reproducing the exact scenario that wedged permanently before
    this fix (co-coordinator xps admits x10sra while original coordinator
    aorus is offline/stale): aorus's log shows
    `group blob changed current_generation=0 remote_generation=3` and
    correctly fetches and applies the 3-member blob within one 60s poller
    cycle, with zero manual restart — where it previously stayed wedged at 2
    members indefinitely. Re-ran a `tetron kick` afterward to confirm
    CONVERGE-003's cleanup-on-removal still fires correctly alongside the new
    generation logic (it does, and faster — MemberSync-triggered rather than
    poller-bound).

    Out of scope: an exact generation tie with genuinely divergent content
    (two coordinators admitting different members from the same base
    generation) is not merged — one side's mutation is deferred until its own
    next local change bumps past the tie. A true CRDT merge is not attempted;
    this is judged sufficient since admission itself is idempotent (a deferred
    admit can simply be retried).

    Found: 2026-07-16, network "converge-test" across 590i-aorus-ultra,
    xps-17-9720, X10SRA — the CONVERGE-001 race reproduced twice more with its
    original fix already deployed, prompting the root-cause fix here.
    """
    req_id = "CONVERGE-005"


# --------------------------------------------------------------------------
# CONVERGE-006: Member boot-restore has no config fallback (asymmetric with
# the coordinator restore path)
# --------------------------------------------------------------------------

class MemberRestoreConfigFallback(Requirement):
    """REQUIREMENT-ID: CONVERGE-006

    `connect_all_networks`'s member-restore path (`join_network_inner(initial
    = false)`) calls `resolve_and_fetch_blob`, which has zero resilience to a
    transient DHT/network hiccup at boot: it resolves the pkarr record, then
    fetches the blob from one of the record's seed peers over iroh-blobs, with
    no local blob-store fallback and no persisted-config fallback. If pkarr
    resolution fails (relay unreachable, DNS not yet up) or none of the seed
    peers happen to be dialable at that exact instant, it returns `Err`,
    `join_network_inner` propagates it, and `connect_all_networks` just logs a
    warning and moves to the next network. The network is silently absent for
    that daemon's entire runtime -- invisible in `tetron status`, with no
    retry, no backoff, recoverable only by noticing and running `sudo tetron
    restart` again.

    This is asymmetric with the *coordinator* restore path:
    `restore_member_roster` (`runtime.rs`) tries the local blob store first,
    then DHT/seeds, and if DHT resolution fails outright, falls back to the
    persisted config roster (`NetworkConfig.members`/`.approved`) rather than
    erroring out -- a restarting coordinator degrades gracefully to a
    possibly-stale-but-non-empty roster. A restarting member gets none of
    that, even though the same config-roster data is already persisted for
    members too (`persist_join_config` writes it on every successful
    join/reconnect) -- it is simply never consulted on this path.

    Fix: on `join_network_inner`'s boot-restore call only (`!initial` -- a
    fresh `tetron join` still fails loudly, which is correct: there is no
    prior membership to fall back to), if `resolve_and_fetch_blob` fails,
    build a `GroupBlob`-shaped fallback directly from the persisted
    `NetworkConfig` (mirroring `restore_member_roster`'s config-fallback
    branch): `members`/`approved` from config, `generation: 0` (informational
    only for a member, which never publishes), `subnet` from
    `config::node_subnet()` (safe per the SUBNET-BUG-001 invariant that an
    already-joined member's node subnet already matches its network's), empty
    `suggested_firewall`/`reusable_keys`/`invites` (not persisted per-member;
    the next successful reconverge repopulates them like any other stale
    field). If the config lookup also comes up empty (no member entries
    persisted, e.g. this network was never actually joined), propagate the
    original error unchanged.

    A fresh `dial_reconnect` still runs against this fallback blob exactly as
    it already does for a live-fetched one, so the existing "coordinator
    offline at restore, reconnect loop will retry" degrade-and-recover
    machinery is unaffected -- this only widens what counts as "got *a*
    roster to start from" to include the persisted config, not just a live
    DHT/seed fetch.

    Found: 2026-07-16, while investigating CONVERGE-004 (a related but
    distinct finding: initial diagnosis of "no poller ever spawns on
    boot-restore" was live-reverified and found inaccurate for the ordinary
    reconnect-succeeds case -- `finalize_join` always spawns one. The real gap
    was traced to this narrower resolve/fetch-failure window instead.
    """
    req_id = "CONVERGE-006"


# ==========================================================================
# tetron: the minimal variant (MINIMAL-*, CON-M*)
#
# This repository is tetron, a stripped-down P2P mesh VPN. See docs/PROPOSAL.md
# for the rationale and design decisions, docs/PLAN.md for the commit-by-commit
# execution order. Inherited SUBNET-*/RENAME-*/CON-* specs above remain
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


class HostnameDefaultsToMachineHostname(Requirement):
    """REQUIREMENT-ID: HOSTNAME-001

    When no `--hostname` is given (and no `default_hostname` is configured
    via `tetron up --hostname`), default to the machine's own OS hostname
    instead of a random noun (the old behavior). `hostname::generate_hostname`
    tries `machine_hostname()` (via `libc::gethostname`, sanitized) first,
    falling back to the random generator only if the OS hostname is
    unavailable or sanitizes down to nothing usable.

    A random hostname gave zero information about which machine a roster
    entry actually was, forcing cross-referencing `tetron status` by IP or
    connection order. The real hostname is immediately meaningful at the
    cost of exposing it to every peer on every network joined -- a
    conscious trade accepted for tetron's model (a private mesh you
    deliberately invite people to); `--hostname` still overrides this for
    anyone who'd rather not.

    `hostname::sanitize_hostname(raw) -> Option<String>`: keeps only the
    first label (OS hostnames are sometimes FQDN-ish, e.g. macOS's
    `MyLaptop.local`), lowercases ASCII letters/digits, collapses anything
    else (spaces, underscores, other punctuation) to a hyphen, trims
    leading/trailing hyphens, truncates to 63 characters (re-trimming a
    hyphen the truncation might land on), and returns `None` if nothing
    usable survives.

    Also loosened *explicit* `--hostname` handling at its three entry points
    (`create_join.rs`'s create and join paths, `runtime.rs`'s `activate`
    for `tetron up --hostname`): each now lowercases the input before
    validating, instead of hard-rejecting mixed case outright (e.g.
    `--hostname MyLaptop` previously errored; now accepted as `mylaptop`).
    `is_valid_hostname` itself is unchanged (still the strict char-class/
    length/hyphen-boundary predicate over an already-lowercased string) --
    other invalid characters (spaces, dots) are still a hard error for an
    explicit `--hostname`, only case is auto-corrected, since silently
    dropping characters from something a user typed on purpose seemed like
    the wrong default whereas case was the actual, specific complaint.

    The `is_valid_hostname` lowercase-only *restriction* itself was
    investigated (traced via `git log -p -- src/hostname.rs` to upstream
    commit `430f670`, "add Magic DNS with .pi domain resolution" -- avoiding
    DNS-label case-folding, a concern MINIMAL-012 removed entirely) but
    deliberately left in place: it's cheap to satisfy (lowercase on the way
    in, both for the machine-hostname path and the explicit-input path
    above) and a single canonical case avoids a second design question
    (would roster lookups like `kick`/`admin add` need to become
    case-insensitive to match a case-preserving hostname?) that has no
    upstream requirement it was resolving anyway.

    Found: 2026-07-17, logged as a TODO note earlier the same session,
    implemented same-day at the user's request as part of the CLI
    flags-and-defaults review.

    **Verified live, 2026-07-17,** on 3 bare-metal machines (aorus, xps,
    x10sra). aorus's real OS hostname (`590I-AORUS-ULTRA`, mixed case) came
    up as `590i-aorus-ultra` in `tetron status` with no `--hostname` passed;
    xps joined the same way and showed as `xps-17-9720` on both its own
    status and the coordinator's roster view, confirming the sanitized
    hostname round-trips correctly through the signed roster.
    """
    req_id = "HOSTNAME-001"


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


class ProductIdentityRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-M02

    Full product identity rename from `torpedo` to `tetron` across every
    user-facing and host-visible surface:

    - Binary: `[[bin]] name = "tetron"` in Cargo.toml (the clap CLI crate name
      and version-string help output change automatically).
    - Service unit: `contrib/torpedo.service` -> `contrib/tetron.service`, with
      all `torpedo` references inside (ExecStart path, Description, group name).
      macOS launchd `com.torpedo.vpn` -> `com.tetron.vpn` (plist filename and
      label string).
    - Config dir: config_dir() in src/config.rs returns `/etc/tetron`.
    - Log dir: log_dir() in src/logdir.rs returns `/var/log/tetron`.
    - Socket path: tetron-proto/src/ipc.rs path changes from
      `/var/run/torpedo/torpedo.sock` to `/var/run/tetron/tetron.sock`.
    - ALPN prefix: transport::network_alpn() generates `tetron/net/<version>/<key>`
      instead of `torpedo/net/...`. This is the protocol-boundary change that
      severs wire compat with full torpedo (D1 retired).
    - CLI help text, error messages, version banner: all `torpedo` -> `tetron`
      in src/main.rs, src/cli/*.rs.
    - Config env var: any TORPEDO_CONFIG_DIR -> TETRON_CONFIG_DIR.
    - IPC response messages that embed the binary name.
    - justfile (`groupadd torpedo` -> `groupadd tetron`, service references).
    - cliff.toml, SECURITY.md, README.md: product name update.
    - Internal source comments referencing `torpedo` as the product name.
    - The `README.md` header and description shall include: "tetron is a
      derivative of torpedo (fork of rayfish)" for attribution, but no longer
      present itself as a fork.

    KEEP (not renamed):
    - The `"rayfish"` relay preset keyword and its URLs (CON-001).
    - Author attribution (Cargo.toml `dario@rayfish.xyz`).
    - LICENSE (MPL-2.0).
    - git history (the rename is a commit in the existing chain).
    - The tetron-proto crate name was set by RENAME-M01; it stays.

    **Follow-up dead-code cleanup, 2026-07-17:** this ALPN-prefix change is
    what actually severs D1 -- iroh negotiates the ALPN during the QUIC
    handshake, so a tetron node and a full torpedo node share no common
    protocol and cannot connect at all, at any level (control or data plane).
    Several "D1 wire compat: decode and ignore" code paths written *before*
    this requirement shipped were never revisited afterward to check whether
    they were still reachable. Audited every `D1` reference in `src/` and
    `tetron-proto/src/` (2026-07-17) and removed the ones gated on receiving
    a message over an established mesh-ALPN connection -- mathematically
    unreachable now, not just unlikely:

    - `ControlMsg::Unpaired`/`CertRefresh`/`InviteShare`/`InviteUsed`
      decode-and-ignore arms (`join.rs`, `coordinator.rs`) -- fell through to
      the existing catch-all with identical (no-op) behavior.
    - `MeshHello.device_cert` capture into the roster
      (`spawn_coordinator_control_reader` in `coordinator.rs`) -- the whole
      point of that block was storing a cert only a full torpedo peer would
      ever send; removed along with the now-unused `state` parameter it
      required (updated both call sites in `accept.rs`).
    - `GroupMode::Open` auto-admit in `CoordinatorAcceptState::handle_connection`
      (`accept.rs`) -- tetron itself can never create an open network
      (MINIMAL-013), and a tetron node could only ever encounter one by
      connecting to a full-torpedo coordinator, which is what this
      requirement makes impossible.
    - The `device_key`-matching prune exemption in `prune_departed_peers`
      (`reconverge.rs`) -- exempted a peer from pruning if the roster's
      `Member.device_cert.device_key` matched its transport id; `device_cert`
      can never be `Some` for any reachable peer once the two branches above
      are gone, so the exemption could never fire.

    **Not removed here, deliberately left in place at the time:**
    `GroupBlob.suggested_firewall` carry-through and the `magic_dns_v4`
    reserved-address logic were initially kept back as "lower-confidence,
    lower-urgency" -- their "dead" argument rested on a weaker, contrived
    cross-product-key-migration scenario rather than this requirement's flat
    ALPN-level impossibility. On reflection (prompted by a follow-up
    question) that distinction didn't actually hold up: the *feature* each
    one served (the userspace firewall, Magic DNS) was already fully removed
    by MINIMAL-010/MINIMAL-012 respectively, so neither one does anything in
    tetron regardless of D1 -- keeping them added complexity for no purpose
    tetron itself has. `magic_dns_v4` was removed the same day in a follow-up
    pass (see MINIMAL-012's own addendum); `suggested_firewall` was reviewed
    on the same pass and evaluated separately (see MINIMAL-010's own
    addendum for its outcome).

    **Also not removed, but for a different reason -- not dead weight at
    all:** `GroupBlob.reusable_keys` admission-time validation. The
    validation logic is product-agnostic (it just checks a presented secret
    against `GroupBlob.reusable_keys`) and is the exact substrate a future
    tetron-native `--reusable` invite flag would need; only its doc comment's
    "D1 compat" framing was stale, now corrected to describe it as dormant
    infrastructure rather than full-torpedo interop.

    **Second follow-up, found later (via `CLI-VOCAB-005`'s `kick` naming
    work), not caught by the 2026-07-17 pass above:** `kick_member`
    (`daemon/mesh/runtime.rs`) had its own `if mode == GroupMode::Open`
    refusal branch -- same provably-dead class as the auto-admit branch
    already removed (tetron never creates an open network, and D1's ALPN
    split means it can never coordinate a full-torpedo one either), just
    missed because the 2026-07-17 audit scoped itself to literal `D1`-
    comment references and this branch's comment didn't carry one. Removed,
    along with its now-dead `mode` local and the doc-comment/help-text/
    README/AGENTS.md lines claiming `kick` is "refused on open networks" or
    "closed networks only." `NetworkState.mode` (`daemon/mod.rs`) is now
    itself unread by anything -- kept (config/wire carries it) but marked
    `#[allow(dead_code)]`, the same treatment already given to
    `membership::OpenPolicy`/`policy_for_mode`, rather than cascading into
    removing the field/config schema entirely -- that bigger structural
    question (drop `GroupMode` down to a single implicit mode) is deliberately
    deferred to its own future pass, not folded into this naming cleanup.
    """
    req_id = "RENAME-M02"


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


class InviteStore(Requirement):
    """REQUIREMENT-ID: INVITE-001

    tetron gains a per-network invite store at
    `<config_dir>/invites/<network>/<invite-id>.toml`. Each file holds:
    - `id`: 8-byte random hex invite identifier (also the filename stem).
    - `secret_hash`: blake3 hex of the invite secret (64 hex chars), so the
      plaintext secret is never persisted.
    - `created_at`, `expires_at` (0 = never): unix timestamps.
    - `used`: bool, set true on single-use redemption.

    The store directory auto-creates under the config dir via the existing
    `config_dir()` helper. No new top-level config keys.
    """
    req_id = "INVITE-001"


class InviteMinting(Requirement):
    """REQUIREMENT-ID: INVITE-002

    The coordinator daemon can mint invite keys. On `invite_create`:
    1. Generate a random 16-byte secret.
    2. Compute its blake3 hash.
    3. Persist the hash + metadata in the invite store (INVITE-001).
    4. Return the printable invite key: `bs58(network_pubkey(32) ||
       coordinator_pubkey(32) || secret(16))`, using the existing
       `invite::encode_invite_code()`.

    The invite key encodes the minting coordinator's pubkey so the joiner
    knows which coordinator to dial. If the minting coordinator goes offline
    before the invite is redeemed, the joiner must wait or obtain a fresh
    invite from another coordinator (cross-coordinator gossip is deferred).
    """
    req_id = "INVITE-002"


class InviteStoreValidation(Requirement):
    """REQUIREMENT-ID: INVITE-003

    On join with `invite_secret` set, `redeem_invite_and_admit` in
    accept.rs checks the local invite store (INVITE-001) before falling
    back to `GroupBlob.reusable_keys` validation (D1 compat path):

    1. Hash the presented secret.
    2. Look up the hash in the store.
    3. If found and not expired and not used:
       - Mark single-use invites as `used = true`.
       - Auto-admit the joiner (skip pending queue).
    4. If not found, expired, or already used:
       - Send `JoinDenied`.

    Single-use invites are burned on first successful redemption.
    """
    req_id = "INVITE-003"


class CliInviteSubcommand(Requirement):
    """REQUIREMENT-ID: INVITE-004

    New CLI subcommand:

        tetron invite <network> create [--expires <duration>]
        tetron invite <network> list
        tetron invite <network> revoke <invite-id>

    `create` prints the invite key and its invite-id. `list` shows
    outstanding invites (id, status, age, expiry). `revoke` marks an invite
    as used so it cannot be redeemed. `tetron invite` with no subcommand
    shows subcommand help.

    The initial `cli/invite.rs` (currently requests/accept/deny handlers)
    is renamed to `cli/requests.rs` to avoid confusion; the invite handlers
    live in a new `cli/invite.rs`.
    """
    req_id = "INVITE-004"


class InviteIpcOps(Requirement):
    """REQUIREMENT-ID: INVITE-005

    New IPC messages for invite operations (tetron-proto/src/ipc.rs):

    - `InviteCreate { network, expires: Option<String> }` ->
      `InviteCreated { invite_key, invite_id, expires_at }`
    - `InviteList { network }` ->
      `InviteListResponse { invites: Vec<InviteInfo> }`
    - `InviteRevoke { network, invite_id }` ->
      `Ok`

    Daemon-side handlers `MeshManager::invite_create`,
    `MeshManager::invite_list`, `MeshManager::invite_revoke` in a new
    `daemon/mesh/invite_store.rs` module.
    """
    req_id = "INVITE-005"


class PostCreateInitialInvite(Requirement):
    """REQUIREMENT-ID: INVITE-006

    `tetron create` auto-mints one single-use invite key and returns it in
    the `Created` IPC response alongside the room id. The CLI displays it
    as the primary way for peers to join:

        created muddy-sunset-whale
          address  10.88.0.1  ·  abcd…1234
        ──────────────────────────────────────────────
        next: tetron join <invite-key>    single-use invite
              tetron invite <net> create  mint another invite
              tetron up                   activate the VPN

    The room id is still printed (it identifies the network to `create` more
    invites for), but the join hint references the invite key instead.
    """
    req_id = "INVITE-006"


class InviteKeyPrimaryAdmission(Requirement):
    """REQUIREMENT-ID: INVITE-007

    Invite keys are the primary enrollment method. The admission priority
    in `CoordinatorAcceptState::handle_connection` is:

      1. Invite secret presented in JoinRequest  -> redeem_and_admit
      2. Reusable key (D1 compat)                -> admit
      3. No invite, Restricted network           -> queue for live approval (fallback)

    The room id is discovery-only: it identifies the network but does not
    suffice to join without an invite key. `tetron join <room-id>` (no
    invite) lands in the pending queue (step 3 above) and waits for a
    coordinator to run `tetron accept`.

    FUTURE (not yet implemented): remove the pending queue entirely so that
    an invite key is required in all cases and `tetron join <room-id>` fails
    with a message directing the user to obtain an invite key. For now, the
    live-approval fallback remains so an operator can manually admit a peer
    who has the room id but no invite.

    The wire protocol still accepts `JoinRequest` without `invite_secret`
    on open networks (D1 compat for full-tetron open-mode networks), but
    tetron only creates closed networks.
    """
    req_id = "INVITE-007"


class InviteFormatUnchanged(Requirement):
    """REQUIREMENT-ID: INVITE-008

    The invite code format is unchanged from upstream:
    `bs58(network_pubkey(32) || coordinator_pubkey(32) || secret(16))`.
    The existing `invite::encode_invite_code` and `decode_invite_code` in
    `src/invite.rs` are reused as-is. The CLI `ipc_join()` in
    `src/cli/network.rs` already detects invite codes vs room ids via
    `decode_invite_code` and sends the secret in `JoinRequest.invite_secret`
    -- no change needed on the joiner side.
    """
    req_id = "INVITE-008"


class InviteExpiryDefault(Requirement):
    """REQUIREMENT-ID: INVITE-009

    Invite keys expire by default. `tetron invite create` without `--expires`
    mints an invite that expires in 7 days. The `--expires` flag accepts
    durations ("24h", "7d", "30d") to override. To create an invite that
    never expires, pass `--expires 0` or `--expires never`.

    `InviteStore::create` defaults `ttl_secs: None` to `7 * 86400` (7 days)
    instead of no expiry. An `expires_at` of 0 means no expiry (opt-in).

    **Correction, 2026-07-17:** `invite_create`'s own rustdoc (`invite_
    handler.rs`) had drifted to say "If absent the invite never expires,"
    directly contradicting the `None => 7 * 24 * 3600` default four lines
    below it and this requirement's own text. The 7-day default is correct
    and intentional (kept as-is); only the stale comment was wrong. Fixed to
    match.
    """
    req_id = "INVITE-009"


class RemoveLiveApproval(Requirement):
    """REQUIREMENT-ID: LIVE-001

    Remove the live-approval admission path entirely. Invite keys are the
    only way onto a tetron network. Removed:

    - Pending join queue (`pending: HashMap<EndpointId, PendingJoin>`) and
      `PendingJoin` struct in `NetworkState`.
    - `evict_oldest_pending`, `MAX_PENDING_JOINS`.
    - `ControlMsg::JoinPending` sender (decode-only kept for D1 compat).
    - `MeshManager::list_requests`, `accept_request`, `deny_request` and
      their IPC dispatch.
    - IPC variants `Requests`, `AcceptRequest`, `DenyRequest`,
      `PendingRequests`, `PendingRequestInfo`.
    - CLI commands `tetron requests`, `tetron accept`, `tetron deny` and
      `src/cli/requests.rs`.
    - Daemon handler file `src/daemon/mesh/invite.rs` (entirely replaced by
      `invite_handler.rs` for invite-key operations).
    - Config `PendingJoinEntry`, `pending_joins` field,
      `add_pending_join`/`remove_pending_join`.
    - Pending-joins restart loop in `connect_all_networks`.
    - `was_approved` parameter on `admit_peer`.
    - `owner_admits` function in `accept.rs` (paired-device D1 shortcut).

    The `approved` field in `GroupBlob` and `ApprovedList` type are
    retained for D1 compat decode only — a full-tetron coordinator may
    publish an approved list that tetron nodes must decode without error.
    tetron coordinators never write to it.

    **Follow-up, 2026-07-18:** one vestige survived this removal —
    `NetworkStatus.pending_requests: usize` (`tetron-proto/src/ipc.rs`)
    stayed on the wire and kept being populated by `diagnostics.rs`'s
    `network_status()`, but hardcoded to `0` at both construction sites
    (nothing left to count once the `PendingJoin` queue was gone) and never
    read by any CLI display code (confirmed via grep — zero hits in
    `src/cli/*.rs`). Found while fixing a related stale doc reference
    (`AGENTS.md` still listing the removed `tetron requests`/`accept`/
    `deny` commands). Removed: the field itself, its hardcoded-`0`
    construction sites, and the `pending_requests` element of
    `network_status()`'s destructured tuple (folded into a 3-element tuple
    with `members`/`member_count`/`nuke_proposals`). Not part of the signed
    `GroupBlob`/its canonical hash, so — unlike `suggested_firewall`'s
    removal — this carried no wire-compat hashing concerns, just a
    `NetworkStatus` field drop.

    **Second follow-up, 2026-07-20:** `pending_requests`'s exact twin,
    `StatusResponse.pending_networks: Vec<String>`, was missed by the
    follow-up above. Found while surveying available-but-unshown fields
    for the `STATUS-002` status redesign, unrelated to it otherwise. Its
    own doc comment claimed to reflect `AppConfig.pending_joins`, which
    this same requirement (`LIVE-001`) removed entirely; the one
    construction site (`diagnostics.rs`) always built it as `Vec::new()`,
    with a comment already admitting as much (`// LIVE-001 removed the
    pending-join queue; always empty.`). Verified zero consumers in
    *either* output mode this time — `grep -rn "pending_networks"
    src/cli/*.rs` returns nothing, and `status.rs`'s `--json` `json!({...})`
    block doesn't even include the field. Removed: the field, its
    construction site, and its slot in `status()`'s destructured tuple.
    Bundled into the `STATUS-002` implementation commit rather than a
    separate change, since it lives on the exact `StatusResponse` struct
    that redesign was already editing.
    """
    req_id = "LIVE-001"


# --------------------------------------------------------------------------
# Constraints: tetron gates (CON-M*)
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
    """CONSTRAINT-ID: CON-M02  [RETIRED]

    Design decision D1 — superseded by RENAME-M02 which changed the ALPN
    prefix and severed wire compatibility with full torpedo. The constraint
    is retired; the product rename is a deliberate protocol boundary.
    GroupBlob still retains suggested_firewall and reusable_keys for schema
    stability but they are inert.
    """
    constraint_id = "CON-M02"
    enforcement_logic = "true"  # RETIRED -- D1 severed by RENAME-M02


class CrateIdentityGate(Constraint):
    """CONSTRAINT-ID: CON-M03

    After RENAME-M01, the token `rayfish` is no longer the internal crate name
    and must not appear in src/**/*.rs (or benches/ or tests/) EXCEPT in the
    deliberately kept places: the relay preset keyword and its comments in
    src/config.rs (CON-001/SUBNET-001), the Cargo.toml author attribution
    `dario@rayfish.xyz`, and LICENSE. This is a curated-allowed-tokens gate, not
    a bare `rayfish` grep, so it never trips on the KEEP-ON-PURPOSE references.

    ENFORCEMENT (reconcile.py): crate_identity.leak_count equals 0.

    **Addendum, 2026-07-19 — the `Cargo.toml` author-attribution carve-out
    is narrowed: name kept, email dropped.** `authors = ["Dario
    <dario@rayfish.xyz>"]` in `Cargo.toml`/`tetron-proto/Cargo.toml` became
    `authors = ["Dario"]` -- an `authors` field is a published contact
    point (crates.io/docs.rs listings, security-scanner disclosure
    targets), and leaving upstream's personal address there risked routing
    this fork's own traffic to someone with no connection to it, the same
    category of problem `RENAME-013` already fixed once for `SECURITY.md`'s
    report-to address. USER's own framing: "give credit without leading to
    emails" -- the fix is dropping the email, not the credit, so the name
    stays. No enforcement change: this gate's own `check_crate_identity`
    (reconcile.py) only ever scanned `.rs` source, never `Cargo.toml`, so
    the carve-out was already inert with respect to the actual automated
    check either way -- confirmed by re-running `reconcile.py` green after
    the edit.

    **Second addendum, same day -- co-author added, same no-email rule.**
    `authors = ["Dario"]` -> `["Dario", "ErikAllanKincaid"]`. USER asked
    to add their own co-authorship but was uncertain about exposing their
    own email either ("better to have message to github"); resolved by
    using a GitHub username as the entry (no email, matching `Dario`'s
    pattern) since `Cargo.toml`'s own `repository` field already points at
    `github.com/ErikAllanKincaid/tetron` -- the actual contact path, same
    "GitHub only, no personal email in a published file" policy
    `SECURITY.md` already follows (`RENAME-013`).
    """
    constraint_id = "CON-M03"
    enforcement_logic = "{{ crate_identity.leak_count == 0 }}"


class ProductIdentityGate(Constraint):
    """CONSTRAINT-ID: CON-M04

    After RENAME-M02, the binary name in Cargo.toml must be `tetron`, the
    ALPN prefix in src/transport.rs must start with `tetron/net/`, and the
    config path in src/config.rs must reference `/etc/tetron`. This is the
    anti-regression gate for the product identity rename: if a cherry-pick
    from torpedo re-introduces `torpedo` in any of these load-bearing paths,
    reconcile.py catches it.

    ENFORCEMENT (reconcile.py): product_identity.binary_name equals "tetron",
    product_identity.alpn_prefix starts with "tetron/net/",
    product_identity.config_dir contains "/etc/tetron".
    """
    constraint_id = "CON-M04"
    enforcement_logic = "{{ product_identity.binary_name == 'tetron' and product_identity.alpn_prefix.startswith('tetron/net/') and '/etc/tetron' in product_identity.config_dir }}"


class LiveApprovalAbsenceGate(Constraint):
    """CONSTRAINT-ID: CON-M05

    Anti-regression gate for LIVE-001: the live-approval tokens
    `AcceptRequest`, `DenyRequest`, `PendingJoin`, `PendingRequestInfo`,
    `evict_oldest_pending`, and `MAX_PENDING_JOINS` must not appear in
    src/ daemon/ or CLI code. If a cherry-pick from torpedo re-introduces
    any of these, reconcile.py catches it.

    ENFORCEMENT (reconcile.py): live_approval_absence.unexpected_count
    equals 0.
    """
    constraint_id = "CON-M05"
    enforcement_logic = "{{ live_approval_absence.unexpected_count == 0 }}"


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


class PeerAddressCache(Requirement):
    """REQUIREMENT-ID: CACHE-001

    tetron saves known peer addresses (endpoint ID, direct addresses, relay
    URL, last seen timestamp) to a flat file on disk on graceful shutdown and
    periodically every 5 minutes. On startup, the cache is loaded and iroh's
    peer table is seeded before any DHT lookup.

    After an all-offline gap, the first member back tries each cached address
    directly. If any other member is also back, the QUIC handshake succeeds
    and the mesh is live without DHT or relay bootstrap. Stale addresses are
    harmless because iroh verifies endpoint identity via the QUIC crypto
    handshake (wrong address = connection failure, not wrong peer).

    Format: flat msgpack file at `<config_dir>/peercache.msgpack` containing
    `Vec<CacheEntry>` where each entry holds endpoint_id (32 bytes),
    known_addresses (Vec<SocketAddr>), relay_url (Option<String>), and
    last_seen (u64 unix timestamp). Entries older than 30 days are pruned on
    load. Writes are atomic (write to temp file, rename).
    """
    req_id = "CACHE-001"


class InviteInBlob(Requirement):
    """REQUIREMENT-ID: BLOB-001

    Move invite storage from machine-local files (`InviteStore`,
    `invites/<network>/<id>.toml`) into the signed `GroupBlob`. An invite is
    an `InviteEntry` struct in the blob:

        struct InviteEntry {
            secret_hash: String,    // blake3 hex
            created_by: EndpointId,
            created_at: u64,
            expires_at: u64,        // 0 = permanent
            used: bool,
        }

    Minting an invite adds an entry to the in-memory blob, signs it, and
    republishes to the DHT. Validating a presented secret: any online
    coordinator hashes the secret and checks the blob's invite table for a
    matching, not-expired, not-used entry. On admission the entry is removed
    (not just marked used) to bound blob size and prevent replay.

    The invite code encoding changes from
    `bs58(pubkey(32) || coordinator(32) || secret(16))` to
    `bs58(pubkey(32) || secret(16))` -- the coordinator endpoint ID is
    dropped so the joiner dials any peer, not the minting machine.

    Supersedes INVITE-001 (machine-local store), INVITE-002 (machine-local
    minting), INVITE-003 (machine-local validation), INVITE-008 (old format),
    and INVITE-009 (expiry logic -- still applies but against blob entries).

    Fetch-before-publish merge is required so concurrent mints from multiple
    coordinators do not clobber each other's entries (the merge logic from
    the PRIVILEGE_TIERS.md design is reused).

    Replay race mitigation: a local reject cache per coordinator (set of
    recently-admitted secret hashes, TTL 5 minutes) plus `InviteUsed` gossip
    (wire message broadcast on admission) prevents a used invite from being
    accepted by a coordinator who has not yet received the updated blob. Once
    the updated blob propagates via DHT poll (~30-60s), the reject cache
    entry expires naturally.
    """
    req_id = "BLOB-001"


class MultiCoordinatorRoutine(Requirement):
    """REQUIREMENT-ID: COORD-001

    `tetron admin add <net> <identity>` is the documented practice for making
    a fully trusted user a coordinator. Every fully trusted member should be
    granted the network key. This eliminates the single-point-of-failure
    where only one machine can admit, mint, kick, or publish.

    The CLI command already exists and works. No code changes are needed.
    Implementation consists of:
    - Update `docs/HOWTO.md` to recommend `admin add` as a routine post-join
      step for every trusted user.
    - Update `docs/TODO.md` to mark multi-coordinator as the expected default.
    - Update `README.md` quickstart to show `tetron admin add` after join.
    """
    req_id = "COORD-001"


# --------------------------------------------------------------------------
# FRAG-001: IPv4 fragmentation for QUIC datagram size limits
# --------------------------------------------------------------------------

class Ipv4Fragmentation(Requirement):
    """REQUIREMENT-ID: FRAG-001

    When the QUIC connection's `max_datagram_size()` is smaller than the TUN
    MTU (1280), IP packets read from the TUN device will not fit in a single
    QUIC datagram. The forwarder must fragment oversize IPv4 packets into RFC
    791-compliant IP fragments, each sent as a separate QUIC datagram, so TCP
    connections (SSH, HTTP, etc.) do not stall.

    Fragment payload size is rounded down to the nearest multiple of 8 bytes
    (RFC 791 Section 3.2). Each fragment carries the original IP header with
    updated Total Length, More-Fragments flag, Fragment Offset, and a
    recalculated header checksum. The original identification and Don't
    Fragment flag are preserved.

    Receiving kernel reassembles fragments before delivery -- no reassembly
    logic is needed in the daemon.

    IPv6 fragmentation is not yet implemented and oversized IPv6 packets are
    dropped with a warning log entry.

    Found: 2026-07-15, network "shallows" where Quinn's max_datagram_size
    was 1162-1192, below the 1228-byte TCP segments produced at TUN MTU 1280.
    SSH key exchange stalled silently at "expecting SSH2_MSG_KEX_ECDH_REPLY".
    """
    req_id = "FRAG-001"


# --------------------------------------------------------------------------
# ADMIN-ADD-EASY-ID: tetron admin add should accept hostname or mesh IP
# --------------------------------------------------------------------------

class AdminAddAcceptsHostname(Requirement):
    """REQUIREMENT-ID: ADMIN-ADD-EASY-ID

    `tetron admin <NETWORK> add <IDENTITY>` must accept a hostname, mesh IP,
    or short id (first 10 hex chars of the endpoint_id). Previously it only
    accepted the raw short-id, which required looking up the peer's endpoint_id
    from `tetron status --json` and manually truncating to 10 characters --
    error-prone for humans.

    Use the same resolution logic as `tetron kick` (`resolve_peer_name`):
    match the argument as a hostname against the signed roster, then fall back
    to short-id prefix matching against endpoint IDs. This makes the admin-add
    workflow as easy as `tetron admin shallows add usbos-1`.

    Found: 2026-07-15, while writing the co-coordinator HOWTO section in
    README.md. The short-id-only requirement forced an awkward `--json` + manual
    truncation step for what should be a simple operation.

    **Correction, 2026-07-17:** this requirement's own text was wrong on two
    points, found during a CLI doc-comment-vs-handler audit. (1) "mesh IP" was
    never implemented -- `resolve_peer_name` only checks hostname, then falls
    back to short-id prefix matching; it never inspects an address. Dropped
    "mesh IP" from the `--help` text (`main.rs`) and the daemon's own error
    message (`admin.rs`), since it promised a capability that did not exist.
    (2) "Use the same resolution logic as `tetron kick`" was also wrong --
    `kick_member` was never changed to use `resolve_peer_name`; it resolves by
    short-id/endpoint-id prefix only (`resolve_short_id_any_network`),
    deliberately, because removing the wrong member needs a cryptographic
    identity, not a spoofable hostname. `resolve_peer_name`'s own rustdoc had
    drifted to claim it backs `kick` (leftover from an edit that moved
    `kick_member`'s real doc comment onto the wrong function) -- restored
    `kick_member`'s doc comment and rewrote `resolve_peer_name`'s to correctly
    name `admin_add` as its caller and state the principle: additive commands
    (`admin add`) may resolve friendlier identifiers; destructive commands
    (`kick`, `nuke --second`) require the short id. `AGENTS.md`'s CLI
    reference had the same "hostname is NOT accepted" error and was corrected
    to match.

    **Fix, 2026-07-17 (same day, follow-up):** `resolve_short_id_any_network`
    took a prefix of *any* length and returned the first member whose
    endpoint id started with it (`.find(...)`) -- no minimum length, and no
    check for more than one match. For `admin_add` this was a UX gap; for
    `kick_member` (the destructive caller) it was a real correctness bug: a
    short-enough or colliding prefix could silently resolve to the wrong
    peer with no warning the input was ambiguous. Fixed: now returns
    `Result<EndpointId, String>`, rejects any input under 10 characters (the
    length `tetron status` already displays, so copy-pasting from status
    always satisfies it) with a "too short" message, and collects all
    matches instead of stopping at the first -- more than one distinct match
    now errors as "ambiguous" naming every candidate's short id, rather than
    guessing. `resolve_peer_name` and its two callers (`admin_add`,
    `kick_member`) propagate the specific message instead of a generic one.
    A full (complete, untruncated) id was already inherently unambiguous
    before this fix and needed no change -- `starts_with` matches a string
    against itself trivially, and no two peers share a full endpoint id.

    Found an analogous, not-yet-fixed gap in the same function family:
    `resolve_peer_name`'s hostname match also returns the first cross-network
    hit with no ambiguity check (lower severity -- only backs the additive
    `admin_add`, not a destructive command). Logged in
    `DO-NOT-COMMIT/TODO.md` rather than fixed in this pass.
    """
    req_id = "ADMIN-ADD-EASY-ID"


# --------------------------------------------------------------------------
# CLI-VOCAB-001: unify the "which locally-known network" argument's name
# --------------------------------------------------------------------------

class LeaveArgumentRenamedToNetwork(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-001

    `tetron leave`'s positional was named `name` while `invite`/`admin` (and
    the same underlying lookup's IPC field) already used `network` -- three
    commands doing the identical `self.networks.get(string)` lookup with two
    different field names for no reason. Renamed `Leave`'s field to
    `network` end to end: `main.rs`'s `Command::Leave`, `cli/network.rs`'s
    `ipc_leave`, `tetron-proto`'s `IpcMessage::Leave`, `daemon/mod.rs`'s
    dispatch arm, and `daemon/mesh/runtime.rs`'s `leave_network` (signature,
    body, and its `#[tracing::instrument]` field). Pure rename -- the lookup
    mechanism itself (a plain map keyed by the mutable local network name)
    is unchanged.

    This is deliberately scoped to `leave` only, not the full rename
    described in `DO-NOT-COMMIT/TODO.md`'s "CLI-wide vocabulary/rename
    pass". `nuke` and `kick` also have this same `name`/`network`
    inconsistency (`nuke` still says `name`), but those two are slated to
    stop using the mutable-name lookup entirely in favor of a not-yet-built
    short-id resolution mechanism (mirroring `resolve_short_id_any_network`,
    fixed for peers in `ADMIN-ADD-EASY-ID`'s follow-up addendum). Renaming
    their field ahead of that mechanism would just relabel today's
    unresolved-by-cryptographic-identity lookup with a more-honest-sounding
    name it doesn't yet deserve -- the same class of doc-vs-behavior mismatch
    this session has otherwise been finding and fixing. `leave`, `invite`,
    and `admin` are not changing lookup mechanism, so unifying their field
    name has no such dependency and was safe to do now.

    Found: 2026-07-17, while auditing all five network-selecting commands
    (`leave`/`nuke`/`kick`/`invite`/`admin`) for consistency at the user's
    request.
    """
    req_id = "CLI-VOCAB-001"


class NukeKickResolveByNetworkShortId(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-002

    `nuke` and `kick` stop resolving "which network" through the mutable
    local display name (`self.networks.get(string)`, the same lookup
    `leave`/`invite`/`admin` still use) and instead require the network's
    own short id -- a prefix of its public key, matching the peer short-id
    convention (`fmt_short()`, 10 hex chars). This is the mechanism gap
    identified in `ADMIN-ADD-EASY-ID`'s follow-up addendum and
    `CLI-VOCAB-001`'s deferred scope: a local alias is user/coordinator-
    chosen, freely mutable, and can collide in meaning across networks --
    unfit as the sole identifier for a destructive, hard-to-undo action.
    There is deliberately no name/alias fallback: unlike peer resolution
    (where `admin add` may resolve a friendlier hostname because it's
    additive), both of these are destructive, so the short-id-only rule
    is absolute.

    New resolver: `MeshManager::resolve_network_short_id` (`daemon/mod.rs`,
    next to `resolve_short_id_any_network`, which it mirrors structurally).
    Rejects prefixes under 10 characters as too short, and rejects a prefix
    matching more than one joined network as ambiguous -- same discipline as
    the peer-side fix, applied to networks for the first time (previously
    there was no network-resolution-by-cryptographic-identity path at all,
    just the raw map lookup). Returns the resolved display name so
    `nuke_network`/`kick_member`'s existing bodies, which are keyed off that
    name throughout, need only a resolution step inserted at the top --
    shadowing the parameter -- rather than a rewrite.

    `tetron status` (`cli/status.rs`) now prints each network's short id
    unconditionally (a new `id <short>` line, computed once per
    `print_network` call and reused for both that line and the nuke-proposal
    hint below) -- without this the feature has nothing to copy from.
    Fixed two now-broken "run this command" hints that echoed the local
    name back at the user: `nuke_network`'s own "have another coordinator
    run `tetron nuke {name}`" message, and `status.rs`'s nuke-proposal
    hint -- both now embed the short id instead, since the alias no longer
    works as an argument to `nuke`.

    `main.rs`'s `--help` text for `Nuke.name`/`Kick.network` corrected from
    "Three-word network name"/"Network name" to explicitly say short id, not
    local name -- leaving the old text would have been a doc-vs-behavior
    lie, the same class of bug this session has spent most of its effort
    finding elsewhere. The field *names* (`name`, `network`) are
    deliberately left untouched -- renaming them is scoped to a later,
    separate pass covering all five commands' `--flags` together, per the
    user's explicit sequencing (internal mechanism first, user-facing
    labels last).

    **Verified live, 2026-07-17,** on 3 bare-metal machines (aorus, xps,
    x10sra): `nuke`/`kick` given the old alias both correctly errored
    `could not resolve network '<alias>'` instead of falling back to the
    name lookup; given the short id both resolved correctly (`nuke
    --cancel` reached the real solo-coordinator rejection; `kick` actually
    removed xps from the roster, confirmed on both sides and via
    CONVERGE-003's self-removal on the kicked node). A prefix under 10
    characters was correctly rejected as too short on both commands. Final
    `tetron nuke <short-id> --force` cleanly destroyed the test network; TUN
    device count stayed at 1 on all three machines throughout.
    """
    req_id = "CLI-VOCAB-002"


class CliFlagsVocabularyPass(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-003

    Executes the `--flags`/positionals half of the CLI vocabulary cleanup
    (`DO-NOT-COMMIT/TODO.md`'s "CLI-wide vocabulary/rename pass"), deferred
    behind the internal-mechanism work (`ADMIN-ADD-EASY-ID`'s follow-up,
    `CLI-VOCAB-001`, `CLI-VOCAB-002`) per the user's explicit sequencing.
    The original proposed table (written before `CLI-VOCAB-001`/`002`
    shipped) was stale in two ways, reconciled here rather than followed
    literally:

    1. It proposed renaming `Leave`'s field to `alias` -- but `Leave` was
       already renamed to `network` by `CLI-VOCAB-001`, and the user's
       later, stronger "I do not like the alias in the first place"
       objection ruled the word out entirely as a lookup-selector name.
    2. It proposed renaming `Nuke`/`Kick`'s network argument to `alias` --
       flatly wrong once `CLI-VOCAB-002` moved both off name-based lookup
       entirely onto short-id resolution; `alias` would have described a
       mechanism those commands no longer use.

    Resolved table (implemented):
    - `Create.name` -> `network_name` (`--name` -> `--network-name`)
    - `Join.name` -> `alias` (`--name` -> `--alias`)
    - `Join`'s positional `network_key` -> `invite_code` (CLI-facing only --
      see the wire-field correction below)
    - `Nuke.name` -> `net_id`, `Kick.network` -> `net_id` (matches the
      short-id mechanism `CLI-VOCAB-002` gave them; `Kick.peer` unchanged)
    - `AdminAction::Add.identity` -> `peer` (matches `Kick.peer` -- same
      concept, `peer` already wins internally: `PeerTable`, `peers.rs`)
    - `Leave`/`Invite`/`Admin`'s `network` field: **no change** -- already
      consistent with each other (`Leave` via `CLI-VOCAB-001`; `Invite`/
      `Admin` were already `network`), which resolves the open "no alias
      anywhere" question for free rather than requiring a rename.

    **Caught and reverted before landing:** `IpcMessage::Join`'s *wire*
    field is not the same thing as `main.rs`'s CLI-facing positional. The
    CLI positional genuinely is raw invite-code text (correctly renamed to
    `invite_code`), but `cli/network.rs`'s `ipc_join` decodes it locally
    (`invite::decode_invite_code`) before ever sending anything over IPC --
    the wire field always carries the resolved network *public key* (with
    the secret riding separately in `invite`), in both the decode-success
    and bare-room-id-fallback branches. Renaming the wire field to
    `invite_code` would have been factually wrong; it stays `network_key`,
    documented with a comment explaining why it looks like a mismatch with
    the CLI layer but isn't.

    **Scope addition beyond the literal table, for internal consistency:**
    `IpcMessage::Created`/`Joined`'s response field (`name` in both) was
    also renamed to `network`, matching what `Leave`/`Invite`/`Admin`
    settled on -- these responses echo back the same "resolved local
    display name" concept, and leaving them as `name` while the identical
    concept elsewhere says `network` would have introduced a new
    inconsistency instead of removing one.

    **Also propagated to internal parameter names** at each renamed field's
    boundary function (`create_network`, `join_network`, `nuke_network`,
    `kick_member`, `admin_add`) so the rename doesn't stop at the wire
    struct. `join_network_inner` already used `alias` internally (predating
    this pass) -- confirms the outer boundary was the actual inconsistency,
    not the deep internals. `create_network_inner`'s `custom_name` and the
    resolved-identity locals in `nuke_network`/`kick_member`/`admin_add`
    were deliberately left as-is -- already clear, not part of the
    user-facing surface this pass targets.

    **Found and fixed as side effects while touching `AGENTS.md`:** the CLI
    reference block still listed `tetron requests`/`accept`/`deny` --
    commands removed by `LIVE-001` (confirmed absent from `main.rs`'s
    `Command` enum), sitting two lines above the very paragraph documenting
    their removal. Removed. Also flagged (not fixed, logged to
    `DO-NOT-COMMIT/TODO.md`): `NetworkStatus.pending_requests` is
    vestigial -- still on the wire, still populated, but hardcoded to `0` at
    both construction sites in `diagnostics.rs` since `LIVE-001` removed
    the queue it used to reflect, and never read by any CLI display code.

    Verified via real `--help` output on all five renamed commands (not
    just build success) before considering this done.
    """
    req_id = "CLI-VOCAB-003"


class CliNetworkKeyVocabularyFollowup(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-005

    Further rename of `CLI-VOCAB-002`/`003`'s `net_id` -> `network_key`,
    prompted by a concrete discoverability gap those two passes didn't
    catch: `net_id` (the `Nuke`/`Kick` positional, shown as `<NET_ID>` in
    `--help`) and `tetron status`'s text-output label (`id`) were both
    spelled differently from `tetron status --json`'s field for the exact
    same underlying value, `network_key`. A user's actual path to this
    value is `tetron status --json | jq`, not reading prose -- grepping
    that JSON for `net_id` or `id` finds nothing, only `network_key`. Fixed
    by standardizing on `network_key` for every human-facing spelling
    (`--help`, status text label, docs) and propagating it back into the
    wire field too, rather than leaving the JSON field as the odd one out.

    Changed:
    - `IpcMessage::Nuke`/`Kick` (`tetron-proto/src/ipc.rs`): field `net_id`
      -> `network_key`.
    - `main.rs`'s `Command::Nuke`/`Kick` clap positional: `net_id` ->
      `network_key` (so `--help` now shows `<NETWORK_KEY>`), plus the
      `main()` dispatch match arms.
    - `cli/network.rs`'s `ipc_nuke`/`ipc_kick` parameters.
    - `daemon/mod.rs`'s `IpcMessage::Nuke`/`Kick` dispatch match arms, and
      `resolve_network_short_id`'s error string (now names `network_key`
      instead of "the short id" when a prefix is too short).
    - `daemon/mesh/runtime.rs`'s `nuke_network`/`kick_member` parameters
      and the `#[tracing::instrument]` field on `nuke_network`.
      `resolve_network_short_id`'s own parameter (`short`) is untouched --
      it is a generic resolver, not part of this user-facing vocabulary.
    - `cli/status.rs`: the text-output line changes from `id <short>` to
      `network_key <short>`, matching `--json`'s field name exactly.
    - Docs (`AGENTS.md`, `README.md`, `docs/HOWTO.md`, `docs/PROPOSAL.md`):
      `<net-id>`/`<net-id-from-status>` placeholders and "the `id` line"
      references updated to `<network-key>`/`<network-key-from-status>`/
      "the `network_key` line", each noting the value may be an
      unambiguous >=10-char prefix rather than the full key.

    **Deliberately unchanged:** `IpcMessage::Join`'s existing `network_key`
    field (always the full public key, decoded client-side from the
    invite code) and this rename's `Nuke`/`Kick` `network_key` (accepts a
    prefix) now share a field name for the same underlying concept --
    intentional, not a naming collision to resolve. `resolve_network_short_id`'s
    behavior (>=10-char prefix, ambiguous-prefix rejection, no name/alias
    fallback) is unchanged; only the names pointing at it moved.

    **Same-session follow-up: `Kick`'s second positional, `peer` ->
    `endpoint_id`.** The same mismatch existed one level deeper: `Kick.peer`
    (shown as `<PEER>` in `--help`) is resolved exclusively by
    `MeshManager::resolve_short_id_any_network`, which matches only against
    a member's endpoint id -- it never accepts a hostname, unlike
    `AdminAction::Add.peer`, which deliberately resolves hostname-first
    (`resolve_peer_name`). `tetron status --json`'s `PeerStatus` struct
    carries both `endpoint_id` and `hostname` fields side by side, so
    without this fix a user would have no way to tell, from the JSON alone,
    which of the two `kick` actually wants -- and guessing `hostname` (the
    more human-looking field) would silently fail to resolve. Renamed
    `IpcMessage::Kick.peer` -> `endpoint_id` (wire), `Command::Kick.peer` ->
    `endpoint_id` (`main.rs`, so `--help` shows `<ENDPOINT_ID>`),
    `cli/network.rs`'s `ipc_kick` parameter, `daemon/mod.rs`'s dispatch
    match arm, and `kick_member`'s parameter in `daemon/mesh/runtime.rs`
    (plus the two internal uses of it in that function). `AdminAction::Add`'s
    `peer` field is deliberately left alone -- it genuinely accepts a
    hostname, so `peer` still describes it accurately.

    **Cross-repo consequence, not part of this change's own scope** (same
    class of risk as `CLI-VOCAB-004`'s `Up`/`Down` wire rename): any code in
    `tetron-webui`/`tetron-systray` constructing `IpcMessage::Kick { peer,
    .. }` directly will fail to compile against this crate until updated
    there.
    """
    req_id = "CLI-VOCAB-005"


# --------------------------------------------------------------------------
# CLI-VOCAB-004: up/down renamed to resume/standby; resume's escalation removed
# --------------------------------------------------------------------------

class ResumeStandbyRename(Requirement):
    """REQUIREMENT-ID: CLI-VOCAB-004

    `tetron up`/`tetron down` renamed to `tetron resume`/`tetron standby`,
    full depth (CLI and wire protocol both), fixing two problems in the
    inherited-from-upstream naming (full analysis in
    `DO-NOT-COMMIT/DECISION_tetron_UpDown_Naming_And_Behavior.md`, not
    reproduced here):

    1. The state `down` produced was never itself called "down" anywhere
       in the product -- `tetron status` and every daemon log/message
       already said "standby" (`·standby·` marker, "on standby (still
       connected to peers)"). The verb and the resulting state's name
       didn't match. Renaming the verb to `standby` makes it match the
       noun that was already in universal use.
    2. `up` silently escalated scope on hidden state (`src/cli/
       service.rs`'s old `cmd_up`): with a daemon reachable it was the
       narrow mirror of `down` (just activate data plane); with none
       reachable it silently did everything `install` does (write the
       systemd unit/launchd plist, `systemctl enable && restart`, wait
       for the daemon, grant operator) *before* activating -- an
       undocumented, asymmetric side door into full installation. `resume`
       removes this escalation entirely rather than renaming it: it is
       always exactly one operation, matching `standby`'s existing
       single-meaning behavior. With no daemon reachable, `resume` now
       errors the same way regardless of caller privilege (collapses the
       old root/non-root branch into one message): "tetron service is not
       running. Install and start it with: sudo tetron install." No new
       verb is needed for the bootstrap case -- `cmd_install` already
       calls the exact same `install_and_start_service()` the old
       escalation fallback called, verified identical before this was
       written.

    Renamed, full depth:
    - `tetron-proto::IpcMessage::Up { hostname, network }` ->
      `Resume { hostname, network }`; `Down { network }` -> `Standby
      { network }`. Wire-level, not just CLI text -- any client
      (`tetron-webui` included) constructing these variants directly
      needs updating in lockstep.
    - `src/main.rs`'s `Command::Up`/`Down` -> `Command::Resume`/
      `Command::Standby`, same fields, same `--hostname`/`--network`
      flags.
    - `src/cli/service.rs`'s `cmd_up` -> `cmd_resume` (escalation removed
      per point 2 above); `src/cli/status.rs`'s `ipc_down` -> `ipc_standby`.
    - `src/daemon/mod.rs`'s IPC dispatch match arms follow the wire rename;
      `activate()`/`deactivate()` themselves are unchanged (they already
      only ever meant "the data plane," never named after the old verbs).

    **State label decoupled from the command verb for the active side.**
    `src/cli/status.rs`'s daemon-wide summary line (`let state = if active
    { "up" } else { "standby" }`) does not become `"resume"` -- "resume" is
    a verb, not a state adjective ("the service is in resume" reads wrong;
    "the service is active" reads right). It becomes `"active"` instead,
    which is not new vocabulary: it already matches the internal `active:
    bool` field used throughout (`net.active`, the JSON output's `"active":
    active`). The `standby` side needed no equivalent split -- "standby"
    already worked as both verb and state-noun before this change, so the
    per-network `·standby·` marker and daemon log text are unchanged.

    **Hard cutover, no soft-deprecation** -- matches the `CLI-VOCAB-003`
    precedent (last CLI vocabulary rename pass, also a hard cutover with no
    aliases kept). No hidden `up`/`down` compatibility aliases; a
    `CHANGELOG.md` entry is the only transition aid. `start`/`stop` are
    explicitly out of scope and unchanged -- they already mirror `systemctl
    start`/`stop` directly and were confirmed not to have either problem
    above.

    **Cross-repo consequence, not part of this change's own scope:**
    `tetron-webui` calls `IpcMessage::Up`/`Down` directly (`src/api.rs`) and
    routes `POST /api/up`/`/api/down`; this wire rename breaks its build
    against `tetron-proto`'s `main` until it is updated separately, in that
    repo, immediately after this ships (per the DECISION doc's sequencing:
    tetron ships first, tetron-webui fixed right after since it's a hard
    compile failure not a backlog item, tetron-systray scaffolded fresh
    against `Resume`/`Standby` only).
    """
    req_id = "CLI-VOCAB-004"


# --------------------------------------------------------------------------
# ADMIN-RECONNECT-CTRL: admin-grant must work after coordinator reconnect
# --------------------------------------------------------------------------

class AdminGrantRespawnsControlListener(Requirement):
    """REQUIREMENT-ID: ADMIN-RECONNECT-CTRL

    When a member's coordinator connection drops and the reconnect loop
    re-establishes it, a new control-listener task must be spawned on the new
    connection. Previously the control listener was only spawned once during
    initial join (attached to the initial connection). When that connection
    dropped the listener died, and the reconnect loop only respawned a forward
    reader -- never a control listener. As a result, any `AdminGrant` sent by
    the coordinator after a reconnect was silently lost, making co-coordinator
    promotion impossible after the coordinator had restarted.

    The fix: pass daemon-wide resources (promote_tx, pending_pongs) and
    per-network state (live_state, reconverge_notify) to the reconnect loop.
    On a successful reconnect, spawn a fresh `spawn_member_control_listener`
    on the new connection alongside the forward reader. The per-network state
    is delivered via oneshot channels because it does not exist when the
    reconnect loop is spawned (it is created inside `join_mesh_shared`, which
    runs after the reconnect loop starts but before any disconnect can occur).

    Found: 2026-07-15, while testing co-coordinator promotion on network
    "shallows". AORUS granted the network key to USB-OS via `tetron admin
    shallows add usbos-1`, which succeeded. USB-OS never received the grant
    because its daemon had reconnected after an earlier restart of AORUS,
    and no control listener was running on the new connection.
    """
    req_id = "ADMIN-RECONNECT-CTRL"


# --------------------------------------------------------------------------
# KICK-REQUIRES-ID: kick requires endpoint-id, not hostname/IP
# --------------------------------------------------------------------------

class KickRequiresEndpointId(Requirement):
    """REQUIREMENT-ID: KICK-REQUIRES-ID

    `tetron kick <net> <peer>` must resolve the peer by its cryptographic
    identity (endpoint id / short id) only. No hostname or mesh IP
    resolution. The previous behavior accepted a hostname, mesh IP, or
    short id via `resolve_peer_name`, which made it possible to kick the
    wrong member if two peers shared a similar name or if the operator
    misread a mesh IP.

    Kicking is a destructive action (removes a member from the roster and
    severs all connections). Using the endpoint id as the sole identifier
    ensures the operator is explicitly naming the target by its
    cryptographic identity, not by a human-friendly alias that could be
    ambiguous.

    Implementation: `kick_member` in `runtime.rs` calls
    `resolve_short_id_any_network` directly instead of `resolve_peer_name`.
    The `resolve_peer_name` helper is unchanged and still used by `admin
    add`, where friendly hostname resolution is appropriate.

    Doc updates: CLI help text, HOWTO.md, and README.md updated to show
    short-id-only form.
    """
    req_id = "KICK-REQUIRES-ID"


# --------------------------------------------------------------------------
# NUKE-CONSENSUS: require at least two coordinators to nuke a network
# --------------------------------------------------------------------------

class NukeRequiresConsensus(Requirement):
    """REQUIREMENT-ID: NUKE-CONSENSUS

    `tetron nuke <net>` used to be runnable by any single coordinator,
    immediately publishing an empty DHT record (poisoning the pkarr record)
    and calling `leave_network`. This meant a single compromised or reckless
    coordinator could destroy the network irrecoverably.

    Require at least two coordinators to approve a nuke, **unless there is
    only one coordinator** in the network. A solo coordinator has no one to
    second and retains the original unilateral nuke behavior.

    Detection: count coordinators from the signed roster
    (`Member.is_coordinator == true`, `membership::coordinator_count`). If
    total coordinators >= 2, the nuke is a two-phase proposal; if exactly 1,
    the nuke proceeds immediately (original behavior, unchanged).

    Implemented flow (coordinators >= 2) -- **command-driven only, no
    automatic background trigger** (deliberately narrowed from an earlier
    draft of this spec that had any coordinator's reconverge/poller loop act
    on an observed blob; see "Scoped down" below):

    1. `GroupBlob.nuke_proposals: BTreeMap<String, u64>`, keyed by the
       proposing coordinator's **full identity string** (not
       `EndpointId::fmt_short()` as originally drafted -- a map key must be
       collision-free, and two coordinators' short ids could theoretically
       collide; short ids are used only for CLI display/matching via
       `membership::resolve_nuke_proposer`). Value is the Unix-seconds
       proposal timestamp. `#[serde(default, skip_serializing_if =
       "BTreeMap::is_empty")]`, matching `reusable_keys`/`invites`'s
       convention, so old blobs decode unchanged and an empty map serializes
       to nothing.

    2. `tetron nuke <net>` on a coordinator (`MeshManager::nuke_network`)
       adds the coordinator's own entry to `nuke_proposals`, bumps the
       generation, and checks the *local* result immediately:
       - If that addition itself brings the count of distinct, unexpired
         proposers to two or more (`membership::nuke_consensus_reached`),
         this same call executes the nuke right there -- publishes the empty
         tombstone record (`MeshManager::publish_nuke_tombstone`) and calls
         `leave_network` -- synchronously, no waiting on reconverge.
       - Otherwise it persists + publishes the proposal blob (same
         persist-then-notify pattern as `invite_create`/`invite_revoke`) and
         returns "N/2 coordinators required".

    3. `--second <short-id>` (`membership::resolve_nuke_proposer`) validates
       the named proposal is currently active before proceeding identically
       to a bare `tetron nuke <net>` -- an explicit safety check when there
       are more than two coordinators, not a different code path.

    4. `tetron nuke <net> --cancel` removes the caller's own entry from
       `nuke_proposals` and republishes (not destructive, no consensus check).

    5. Proposals auto-expire via a 24h TTL
       (`membership::NUKE_PROPOSAL_TTL_SECS`,
       `membership::active_nuke_proposers`) -- filtered at read time
       (consensus check, `--second` resolution, `tetron status` display), not
       actively pruned from the map on mutation.

    6. `tetron status` surfaces active (unexpired) pending proposals
       (`NetworkStatus.nuke_proposals`, `ipc::NukeProposalInfo`) so members
       can see a nuke is being considered. This is synced into
       `NetworkState.nuke_proposals` on every reconverge
       (`reconverge_and_apply`, `spawn_group_poller`) -- but purely for
       display; see "Scoped down" below.

    **Scoped down from the original draft (2026-07-17, before
    implementation):** an earlier version of this spec had a coordinator's
    background reconverge/poller loop independently notice an
    already-consensus-reached blob (e.g. a third coordinator who never ran
    `tetron nuke` at all) and execute the tombstone-publish on its own. That
    was deliberately cut: verifying an automatic, background-triggered,
    irreversible action needs the same kind of live multi-coordinator race
    testing CONVERGE-001 needed across two rounds before it was actually
    correct, and the payoff was narrow. What remains is strictly
    command-driven: the *only* code path that can ever publish the
    destructive tombstone is the synchronous `nuke_network` handler. The
    trade: two coordinators proposing at nearly the same instant (before
    either sees the other's write) can leave the blob showing 2 valid
    proposers with nobody having triggered execution -- resolved by either
    coordinator running `tetron nuke` once more, which then sees the merged
    count and finishes it. A liveness gap, not a safety gap -- it fails
    toward *not* destroying the network automatically, not toward an
    unexpected automatic destruction.

    Found: 2026-07-16, during multi-coordinator audit. Race C (no coordinator
    revocation) makes nuke the only way to remove a compromised coordinator.
    Requiring consensus prevents a single key holder from destroying the
    network, while the solo-coordinator exception avoids locking out networks
    that have never promoted a co-coordinator.

    **Two bugs found and fixed via live 3-machine testing, 2026-07-17**
    (neither could have been caught by unit tests alone -- both are
    distributed-convergence failures that only manifest with real network
    latency and real coordinator restarts):

    1. The tombstone's `(hash, generation)` pointer reached the DHT
       correctly, but the actual empty-blob *bytes* were never persisted
       anywhere fetchable -- the executing coordinator calls `leave_network`
       (closing its connections) immediately after publishing, so it was
       typically the only node that ever held them, and every other node's
       `fetch_verified_blob` attempt failed forever ("could not fetch
       updated group blob from any peer or seed"). `member_removed`
       (CONVERGE-003) never fired for remaining members. Predates
       NUKE-CONSENSUS (the original single-coordinator nuke had the
       identical gap) but only surfaces with other members present to
       notice the failure. Fixed by recognizing that a tombstone's content
       is fully deterministic given just its generation (always empty
       members/approved/etc.) -- `membership::try_decode_tombstone`
       reconstructs and verifies it locally, tried before ever attempting a
       peer fetch, sidestepping the distribution problem entirely.

    2. `spawn_group_poller`'s generation comparison treated an exact tie
       (`remote_generation <= current_generation`) as "nothing new" even
       when the hash differed. This is a general liveness bug, not
       nuke-specific: a node's own unrelated local mutations (e.g. pruning
       a peer that gracefully left) can independently advance its
       generation to the same number a different coordinator's mutations
       reached, purely by coincidence -- observed twice in this session via
       two different mechanisms. Once tied, the node would never fetch
       again for that network, regardless of how different the actual
       content became. Fixed: `poller_should_fetch` now also fetches on an
       exact-generation tie if the hash differs; a tie with a matching hash
       still correctly skips as a no-op.

    A third, separate, pre-existing issue was found during this testing but
    deliberately **not** fixed here (needs its own dedicated design, not a
    tail-end change to this spec): a coordinator's unconditional first
    publish after restart (`dht_read_before_write`'s `if
    last_published.is_none() { return true; }`) can resurrect stale state
    if that restart's restore fell back to local config (DHT/blob
    unreachable). Logged as a new TODO, out of scope for NUKE-CONSENSUS.
    """
    req_id = "NUKE-CONSENSUS"


# --------------------------------------------------------------------------
# DIAL-001: background, concurrent, timeout-bounded roster dials
# --------------------------------------------------------------------------

class BackgroundConcurrentBoundedDials(Requirement):
    """REQUIREMENT-ID: DIAL-001

    Three related dial-blocking gaps, identified while triaging upstream
    rayfish fixes (02dd60e, fe3f3c0, b26c26b) against tetron's current
    `join.rs`/`create_join.rs`/`runtime.rs` and confirmed still present by
    direct code inspection — not assumed from the upstream commit messages:

    1. `join_mesh_shared`'s `connect_to_roster_peers` (member join/reconnect)
       dialed the rest of the roster serially and `.await`ed the whole loop
       before the join completed. A single unreachable roster member (a
       stale, offline device still listed) blocked the *entire* join/reconnect
       on iroh's uncapped internal handshake timeout before any other peer
       connected — even though the coordinator link was already up and the
       network was otherwise usable.

    2. `dial_all_members` (the coordinator-restore full-mesh dial, used by
       both `create_network_inner` and `restore_coordinator_network`) was a
       plain serial `for member in members { ...await... }` loop with no
       timeout at all — confirmed by direct read, not inherited from
       upstream's history. Restore time scaled linearly with roster size and
       could stall indefinitely on one dead peer.

    3. `restore_coordinator_network` `.await`ed that entire serial,
       unbounded `dial_all_members` call *before* `self.networks.insert(...)`
       — confirmed by reading the function directly. `tetron status` run in
       that window (routinely triggered right after `sudo tetron restart`)
       reported no active networks at all, even though the config and local
       roster were completely intact, for as long as the slowest/least
       reachable roster member took to resolve.

    Fix, applied together since all three are faces of the same root cause
    (serial + unbounded dialing blocking usability):

    - `connect_to_roster_peers` becomes `spawn_roster_peer_dials`: the
      coordinator/initial peer link is registered synchronously (as before),
      then the rest of the roster dials concurrently in a spawned background
      task (`futures::stream::FuturesUnordered`), each bounded by
      `MESH_PEER_DIAL_TIMEOUT` (30s — generous since it's off the boot path)
      and cancellation-aware via the network's token. The join/reconnect
      completes as soon as the initial link is up; peer links fill in as they
      connect, and the existing reconnect loop recovers any that time out.
    - `dial_all_members` gains the same `FuturesUnordered` concurrency and a
      `DIAL_TIMEOUT` (10s — tighter, since this dial runs proactively on
      every restore regardless of whether a peer will ever answer, and the
      per-peer reconnect loop is the real recovery path either way, not this
      one-shot proactive dial).
    - `restore_coordinator_network` inserts the `NetworkHandle` into
      `self.networks` before the (now backgrounded, non-blocking-in-spirit
      but still practically fast) `dial_all_members` call, so the network is
      visible to `tetron status`/IPC as soon as local restore completes,
      matching the ordering `create_network_inner` already effectively gets
      for free.

    tetron's dials never carry a real `DeviceCert` (device pairing was
    removed by MINIMAL-004; `device_cert: None` is hardcoded at every
    `MeshHello` site already), so unlike the upstream commits this fix
    carries no device-cert plumbing — one parameter fewer throughout.

    Found: 2026-07-16, triaging rayfish commits 02dd60e/fe3f3c0/b26c26b for
    tetron applicability. All three confirmed missing by direct code
    inspection of tetron's current `join.rs`/`create_join.rs`/`runtime.rs`,
    not assumed from upstream history — tetron's fork point and subsequent
    MINIMAL-* rewrites make no guarantee upstream fixes were ever inherited.
    """
    req_id = "DIAL-001"


# --------------------------------------------------------------------------
# CONVERGE-007: a kick-coded connection close never mutates the roster
# --------------------------------------------------------------------------

class CloseCodeNeverMutatesRoster(Requirement):
    """REQUIREMENT-ID: CONVERGE-007

    Found triaging the applicable slice of upstream rayfish commit 1c193b9
    (most of that commit — status device-grouping, `RequestUnpair` — is N/A
    for tetron, since device pairing was removed by MINIMAL-004) against
    tetron's current `forward.rs`/`coordinator.rs`. Confirmed present by direct
    inspection, not assumed from upstream.

    `DisconnectEvent.intentional` was computed `true` for *both* `LEAVE_CODE`
    and `KICK_CODE` (`forward.rs:314-319`), and `coordinator.rs`'s
    `spawn_peer_cleanup` treated `intentional == true` as authority to prune
    the canonical roster (`st.members.remove(&member_id)`). But
    `prune_departed_peers` (CONVERGE-005's territory) closes a connection with
    `KICK_CODE` on *every* node, coordinator or not, whenever its own local
    roster momentarily doesn't list the peer on the other end — including
    during an ordinary, still-resolving convergence race, not just a real
    kick. If that peer happens to be the coordinator's own link to a
    genuinely-still-valid member (a transient reconverge race, exactly the
    class CONVERGE-005 narrows but does not fully eliminate — the
    same-generation-tie window is explicitly left unresolved), the
    coordinator's cleanup handler saw the `KICK_CODE` close, computed
    `intentional = true`, and pruned that real member from its own roster and
    republished — a false eviction, driven by connection-close inference
    instead of the signed record. Worse, thanks to CONVERGE-003, the
    mistakenly-pruned member would now promptly and cleanly *leave* on
    receiving that wrongly-updated blob — CONVERGE-003 makes a bogus eviction
    complete faster and more silently than before that fix, since there is no
    longer a stuck "ghost" state to notice and investigate.

    tetron's actual, coordinator-authoritative kick path
    (`remove_member_roster_only` + `finalize_removal` in `coordinator.rs`) was
    never the problem — it already mutates the roster directly as a real
    decision, then closes the victim's connection with `KICK_CODE` as a
    consequence, not a cause. The bug was a second, redundant, and incorrect
    path to the same roster mutation, reachable from mere connection-close
    observation on *any* node running `prune_departed_peers` — not the actual
    kick command.

    Fix: replace `DisconnectEvent.intentional: bool` with a `CloseReason`
    enum (`Left` / `Kicked` / `Other`) and a `prunes_member()` helper that is
    `true` only for `Left`. `coordinator.rs`'s cleanup now prunes the roster
    only on `Left`; a `Kicked` (or `Other`) close just stamps `last_seen`,
    matching the existing non-intentional-drop branch. `join.rs`'s reconnect
    loop narrows its "peer left, not reconnecting" skip to `Left` only,
    letting a `Kicked` close fall through to the existing `pruned_peers` check
    immediately below it — which is *already* the correct, signed-roster-
    driven arbiter (populated only by `prune_departed_peers` after a verified
    reconverge, never by raw close-code inference) for whether to actually
    stop reconnecting. This ties every reconnect-suppression and every roster
    mutation to the signed record, never to a bare close code, continuing the
    "generation/signed record is the only source of truth" principle
    CONVERGE-005 established for publishing.

    The synthetic disconnect event `dial_reconnect` sends per member on a
    cold restore (no live connection yet, used only to force the reconnect
    loop's first dial attempt) maps to `CloseReason::Other` — it was never a
    leave or a kick, just a kick-start (pun unintended) for the dial loop.

    Found: 2026-07-16, triaging rayfish 1c193b9 for tetron applicability.
    """
    req_id = "CONVERGE-007"


# --------------------------------------------------------------------------
# CONVERGE-008: no unconditional "first publish" bypass -- always
# read-before-write, even on a coordinator's very first publish attempt
# --------------------------------------------------------------------------

class NoUnconditionalFirstPublish(Requirement):
    """REQUIREMENT-ID: CONVERGE-008

    `dht_read_before_write` (CONVERGE-005's generation-authoritative publish
    guard) had a bypass: `if last_published.is_none() { return true; }` --
    a caller's very first publish attempt (no locally-tracked prior publish
    yet) always proceeded unconditionally, skipping the DHT comparison
    entirely. This was meant for the genuinely-new-network case (nothing to
    compare against), which the guard's own `Err` arm (no DHT record found)
    already handles correctly on its own -- the bypass was never actually
    load-bearing for that case.

    What it was actually doing, unintentionally: `seal_and_publish` (shared
    by `create_network_inner` and `restore_coordinator_network`) calls
    `dht::publish_network` directly -- with no read-before-write guard at
    all -- immediately at restore time, before the periodic publisher loop
    (which does have the guard, but only after its own first-iteration
    bypass) even starts. `restore_member_roster` falls back to stale local
    config when the DHT/blob is unreachable at restart ("could not restore
    roster from DHT blob; falling back to config"). Combine the two: a
    coordinator restarting under flaky DHT connectivity restores a
    possibly-stale roster, then `seal_and_publish` unconditionally
    republishes it, potentially overwriting a concurrently-mutated (or even
    already-nuked, see NUKE-CONSENSUS) DHT record with old, wrong content.

    Fix: removed the bypass from `dht_read_before_write` entirely (now
    `pub(crate)`, no `last_published` parameter -- it always does the real
    generation/hash comparison, with the existing `Err` arm still covering
    "nothing published yet"). `seal_and_publish` now goes through the same
    guard before its `dht::publish_network` call, instead of calling it
    unconditionally; if the guard defers, the (already generation-authoritative)
    group poller picks up the real current state on its next tick. For a
    genuinely brand-new network this adds one harmless extra `resolve_network`
    round-trip (always `Err`, guard passes). `spawn_network_publisher`'s
    `last_published` local variable is now unused for gating (removed);
    `spawn_lazy_publisher` keeps its own `last_published` check as a
    separate, still-valid optimization (skip even attempting a DHT
    round-trip when the local hash hasn't changed) -- that check is
    independent of what the guard itself does internally.

    This does not touch the established pattern used by one-shot,
    fresh-local-mutation publishes (`invite_create`, `invite_revoke`, kick,
    `admin_add`'s `store_and_publish_group`) -- those correctly publish
    unconditionally because they *are* the authoritative new state (a local
    mutation just happened), unlike a restore, which may or may not reflect
    reality depending on whether the DHT fetch that fed it actually
    succeeded.

    Found: 2026-07-17, as a side effect of NUKE-CONSENSUS live testing --
    repeated manual daemon restarts (for redeploying binaries mid-test) on
    the original coordinator collided with this, resurrecting a stale
    record and getting the node stuck comparing against its own
    resurrected write rather than the real state. Deliberately deferred out
    of the NUKE-CONSENSUS commit (needed its own scoped fix + live
    validation, not a tail-end change to an already-large feature PR).

    Live validation needed before trusting this in production, same bar as
    CONVERGE-001/NUKE-CONSENSUS: restart a coordinator with the DHT/blob
    deliberately blocked (falls back to stale config), verify it does not
    clobber a concurrently-mutated (or nuked) record once connectivity
    returns.
    """
    req_id = "CONVERGE-008"


# --------------------------------------------------------------------------
# MULTISEG-001: per-network subnet field on NetworkConfig (additive, unread)
# --------------------------------------------------------------------------

class PerNetworkSubnetConfigField(Requirement):
    """REQUIREMENT-ID: MULTISEG-001

    Step 1 of the multi-segment TUN plan (scoped in full in
    `DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`, "Scoped code changes"):
    tetron today shares one TUN device and one node-wide overlay subnet
    (`AppConfig.subnet`, SUBNET-010) across every joined network, even
    though each network's signed `GroupBlob` already carries its own
    `subnet: Option<Subnet>` — the data model has supported per-network
    subnets since BLOB-001; only the daemon's single-TUN orchestration
    hasn't caught up. Multi-segment TUN (one TUN device + subnet per
    network, so a host can bridge two operator-distinct segments the way
    two physical NICs would) needs a place to persist each network's own
    subnet locally, ahead of any per-network TUN device existing to use it.

    Adds `subnet: Option<crate::membership::Subnet>` to `NetworkConfig`
    (`src/config.rs`), serialized the same way as the existing node-wide
    `AppConfig.subnet` / `Settings.subnet` fields (`with =
    "crate::membership::cidr_opt"`, CIDR string on disk, `None` omitted).
    `None` means "this network uses the node-wide subnet," identical to
    today's actual behavior — so this field starts fully inert. The three
    non-test `NetworkConfig` construction sites: `create_join.rs`'s
    `create_network_inner` and `join.rs`'s `join_network_inner` set it to
    `None` (nothing mints a per-network subnet yet); `runtime.rs`'s
    `restore_coordinator_network` carries the persisted value forward
    (`net_config.and_then(|nc| nc.subnet)`), matching the existing
    preserve-across-restart pattern already used for `admins`/`direct`.

    Deliberately scoped to *only* this field — no `--subnet` CLI wiring, no
    read site, no interaction with `SUBNET-010`'s node-wide-subnet
    enforcement (both its sites are untouched). This is intentionally the
    one part of the multi-segment TUN plan that is safe and independently
    shippable on its own: the field is round-tripped by serde but nothing
    in the daemon ever reads it, so there is no behavior change and no way
    for this commit alone to reintroduce SUBNET-BUG-001 (a previously-fixed
    bug where a subnet mismatch silently misconfigured the single shared
    TUN). Every later step in the plan (relaxing `SUBNET-010`'s join-side
    check, per-network `NetworkHandle`/`MeshCtx`/TUN-lifecycle
    restructuring, `forward.rs`) depends on this field existing first, and
    is unsafe to land before per-network TUN devices actually exist to
    honor it -- see the corrected "Suggested commit sequence" in the ideas
    doc.

    Found: 2026-07-18, first commit of the `feat/multi-segment-tun` branch.
    """
    req_id = "MULTISEG-001"


# --------------------------------------------------------------------------
# MULTISEG-002: per-network PeerTable/MeshCtx (NetworkHandle owns its own
# data-plane routing table instead of sharing one daemon-wide table)
# --------------------------------------------------------------------------

class PerNetworkPeerTableAndMeshCtx(Requirement):
    """REQUIREMENT-ID: MULTISEG-002

    Step 3 of the multi-segment TUN plan. Moves `PeerTable` off `MeshManager`
    (previously one daemon-wide table shared by every joined network) onto
    each `NetworkHandle` — every network now owns its own routing table,
    populated as soon as the handle exists (independent of whether a TUN is
    attached yet, matching the pre-existing headless-before-attach pattern
    `build_headless()` already relied on).

    `MeshCtx` (the per-accept-handler/background-task bundle of
    `identity`/`peers`/`tun_tx`/`stats`/`blob_store`/`pruned_peers`) is no
    longer built once daemon-wide via a `mesh_ctx()` method. Two construction
    paths now exist: (1) every call site that establishes a network — the
    `create_network_inner`/`join_network_inner`/`restore_coordinator_network`
    handlers, plus the `try_dht_fallback_join` dead-code path kept compiling
    for consistency — builds a fresh `MeshCtx` from a freshly created
    `peers`/placeholder `tun_tx` pair (`MeshManager::new_network_data_plane`)
    *before* the `NetworkHandle` exists in `self.networks`, since there is
    nothing yet to look up; (2) `MeshManager::mesh_ctx_for(network)` looks up
    an *existing* handle's own `peers`/`tun_tx`, used only by
    `promote_to_coordinator` (the one call site where the handle already
    exists). `register_coordinator_handler` and `spawn_coordinator_background_
    tasks` both take `ctx: MeshCtx`/`ctx: &MeshCtx` as an explicit parameter
    now, rather than building it internally, so each caller supplies whichever
    of the two is correct for its situation.

    **Deliberate deviation from the original scoping doc
    (`DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`):** the doc suggested
    `PeerEntry.conns: HashMap<SmolStr, Connection>` could collapse to a bare
    `Connection` once each network has its own table (a peer only ever has one
    connection within a single-network-scoped table). Implemented instead as
    **N separate instances of the existing `PeerTable`/`PeerEntry` shape,
    unchanged** — `src/peers.rs` has zero code changes. Reasoning: the
    `conns`-collapse would touch every one of `PeerTable`'s ~15 methods'
    signatures (dropping their `network: &str` parameter) and every call site
    across `accept.rs`/`join.rs`/`runtime.rs`/`create_join.rs`/
    `diagnostics.rs`/`admin.rs`/`publish.rs` — a second, independently risky
    refactor layered on top of an already-large one, for a data-structure
    tidiness gain with no behavioral difference (a table now holding only one
    network's entries makes the existing `_for_network`/`_by_network`-suffixed
    methods over-general but not incorrect — calling
    `peers_for_network_with_conn(name)` on a table that only ever contained
    `name`'s entries returns exactly the same thing a hypothetical
    `all_with_conn()` would). Chose the smaller, safer diff. Flagged here as a
    real follow-up cleanup, not silently dropped.

    `crate::peercache::refresh_from_peers` (CACHE-001) took one `&PeerTable`;
    with N tables it is now called once per network via a new
    `MeshManager::refresh_peer_cache()` that iterates `self.networks`.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-003/004/005/006 (see MULTISEG-004's "Suggested commit sequence"
    note in the ideas doc for why these five could not safely ship as
    separate commits despite being granular, separable requirements).
    """
    req_id = "MULTISEG-002"


# --------------------------------------------------------------------------
# MULTISEG-003: per-network TUN lifecycle (attach_tun/detach_tun become
# per-network; each network creates/tears down its own OS TUN device)
# --------------------------------------------------------------------------

class PerNetworkTunLifecycle(Requirement):
    """REQUIREMENT-ID: MULTISEG-003

    Step 4 of the multi-segment TUN plan. `MeshManager::attach_tun`/
    `detach_tun` (the embedding API previously used once, daemon-wide, by a
    hypothetical mobile embedder attaching a single `VpnService` fd) now take
    a `network: &str` and operate on that network's own
    `peers`/`tun_name`/`tun_tx`/`tun_tasks` (all moved onto `NetworkHandle` by
    MULTISEG-002). **New finding since the doc was written:** `ray-mobile`
    was removed by MINIMAL-016 and grepping the workspace `Cargo.toml` and
    `src/` finds no in-tree consumer of this embedding API today — extending
    it to be per-network (called once per network instead of once per daemon)
    is a natural extension, not a break of any live integration. A future
    embedder attaches one packet interface per network it wants active,
    rather than one for the whole daemon.

    `run_daemon` (`bootstrap.rs`) no longer creates one OS TUN device at boot
    before any network exists. Instead, `MeshManager::
    create_and_attach_network_tun(network, my_ip, subnet)` runs inside each
    of the three live network-establishment paths (`create_network_inner`,
    `finalize_join`, `restore_coordinator_network`), right after that
    network's `NetworkHandle` is inserted: it calls `tun::create()` in that
    network's own subnet, records the OS-assigned device name (already unique
    per call — `tun.rs`'s Step-0 finding that every function is already
    parameterized by device name held up unchanged), and calls the new
    per-network `attach_tun`. Failure is non-fatal (logged, network stays
    control-plane-connected without a data plane), matching `activate()`'s
    existing warn-don't-fail pattern for TUN problems.

    If the VPN is already active (`self.active`) at that point —
    `tetron join`/`create` while already up, or a restore whose attach lands
    after boot's one `activate(None)` call already ran — this also brings
    that network's link up and installs its routes immediately, instead of
    waiting for a future `activate()` call it would otherwise miss entirely
    (since `activate()` only iterates whatever is in `self.networks` *at the
    moment it runs*). **Known, documented, unclosed residual race:**
    `connect_all_networks` fires each saved network's restore as a detached
    `tokio::spawn` task and does not await them; in principle a restore's own
    post-attach `self.active` check could run a moment before `activate()`'s
    own `self.active.swap(true, ...)` executes, in which case neither catches
    it and that network's TUN stays administratively down until a manual
    `tetron down && tetron up`. In practice every restore does a DHT
    round-trip (tens to hundreds of ms) before reaching that check, while
    `activate()`'s swap runs within microseconds of `connect_all_networks()`
    returning, so the window is not expected to be hit — but it is not a hard
    guarantee, and closing it fully would mean awaiting every restore before
    `run_daemon` proceeds, undoing the fire-and-forget design
    `connect_all_networks` deliberately uses so one dead/slow network can't
    delay the others (a DIAL-001-adjacent tradeoff this does not reopen).
    Documented in code at `MeshManager::create_and_attach_network_tun`'s doc
    comment; flagged here as a known gap needing live multi-network-boot
    testing to actually observe (or not) before this can be fully trusted.

    `activate()`/`deactivate()` (previously operating on one daemon-wide
    `tun_name`) now iterate `self.networks`, bringing every network's own link
    up/down and installing its own loopback self-route (`handle.my_ip`, which
    MULTISEG-004 makes genuinely per-network rather than the node-wide
    identity IP). **Known, documented, unresolved limitation surfaced by this
    change, not present in the original scoping doc:** peer IPv6 addresses
    (`derive_ipv6`) are identity-derived and global across every network a
    node joins (`200::/7`, "never rotates" per `AGENTS.md`'s addressing
    section) — unlike IPv4, they are not subnet-scoped per network. The
    `route_peer_range` call installs one system-wide `200::/7 -> <tun>`
    kernel route; with N TUN devices the last one activated would otherwise
    silently win that route, leaving every other network's peers unreachable
    over IPv6 (IPv4 stays correctly segmented regardless, since each network
    has its own distinct v4 subnet/TUN). `activate()` now installs the
    `200::/7` route on only the first network encountered, deterministically,
    so this is an explicit "IPv6 mesh reachability works on one segment only"
    limitation rather than a silent last-writer-wins race. This is a genuine,
    unresolved product question (does multi-segment TUN need IPv6 addressing
    to become per-network too, e.g. by deriving it from `(identity, network)`
    the way IPv4 already is, or is single-segment IPv6 an acceptable interim
    state?) that was out of scope to resolve in this pass and needs an
    explicit decision before this ships.

    Network teardown (`teardown_network_runtime`, reached by `leave_network`/
    `nuke_network`'s solo-coordinator immediate-destroy path/kick-of-self)
    now aborts that network's own forwarding tasks and calls the new
    `tun::delete()` (added to `src/tun.rs`: `ip link delete` on Linux,
    `ifconfig <name> destroy` on macOS) rather than relying solely on the
    kernel to reclaim the device whenever the whole process eventually exits.
    This incidentally closes the pre-existing "stale TUN devices survive a
    daemon restart/crash" gap logged in the ideas doc's "Fallback" section —
    per-network teardown now runs mid-process, with other networks' devices
    still live, so relying on process-exit-triggered cleanup was no longer
    viable regardless.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/004/005/006.
    """
    req_id = "MULTISEG-003"


# --------------------------------------------------------------------------
# MULTISEG-004: relax SUBNET-010 (per-network TUN means no shared TUN left
# for a subnet mismatch to break); SUBNET-014's warning mechanism retired
# --------------------------------------------------------------------------

class SubnetCoherenceRelaxed(Requirement):
    """REQUIREMENT-ID: MULTISEG-004

    Step 2 of the multi-segment TUN plan, landed only once MULTISEG-003 (per-
    network TUN) actually exists — see the corrected "Suggested commit
    sequence" logged in `DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`: relaxing
    this before per-network TUN existed would have reintroduced
    SUBNET-BUG-001 (joining a network whose subnet didn't match the single
    shared TUN silently misconfigured that TUN, breaking the data plane with
    no error). Per-network TUN removes the precondition that bug depended on
    — there is no longer a single shared TUN for a network's subnet to
    disagree with.

    **Create side** (`create_network_inner`): removed SUBNET-010's rejection
    of a `--subnet` that disagreed with the already-persisted node-wide
    value. A brand-new network name has nothing to conflict with (the
    existing `already active` check already rejects reusing a name); the only
    remaining validation is the pre-existing `already active`/hostname/CIDR
    checks. `AppConfig.subnet` (the node-wide cache, `config::node_subnet()`)
    keeps exactly one job: seeding the *default* subnet for a create with no
    explicit `--subnet` and nothing persisted yet. An explicit `--subnet`
    still updates that default (for the node's next unspecified create), it
    just no longer gets rejected for disagreeing with a prior one.

    **Join side** (`join_network_inner`): removed the SUBNET-BUG-001 guard
    outright (`network_subnet != node_subnet` -> `bail!`). `my_ip` is now
    derived directly from the joining network's own blob-carried subnet
    (`if network_subnet == self.identity.subnet() { self.identity.local_ip() }
    else { derive_ip(&self.identity.local_identity(), network_subnet) }`),
    mirroring the derive-if-different pattern `create_network_inner` already
    used — this was a real, previously-missed bug in the pre-relaxation code:
    `my_ip` was computed from `self.identity.local_ip()` (the node-wide
    identity IP) unconditionally, which would have been wrong the moment the
    coherence guard was removed without this fix. `restore_coordinator_network`
    gets the equivalent fix: its `subnet`/`my_ip` are now derived from
    `NetworkConfig.subnet` (MULTISEG-001's field — this is its first real
    read) falling back to the default, not from `self.identity.subnet()`.

    **SUBNET-014's warning mechanism is retired, not removed.** That
    requirement's `warning: Option<String>` field on the `Created`/`Joined`
    IPC responses existed because a subnet mismatch used to require a full
    `sudo tetron restart` to take effect on the one shared TUN. That scenario
    no longer exists — every network's TUN is created fresh, in its own
    correct subnet, at the moment it's established. All four call sites that
    used to call `membership::subnet_change_warning` now pass `warning: None`
    unconditionally. The wire field itself, `subnet_change_warning`'s
    definition, and its unit test are left in place (harmless, no longer
    exercised by any live call site) rather than removed — deleting an
    IPC/wire surface is a separate, deliberate cleanup better done on its own,
    not a drive-by of this change. Flagged as a real follow-up, not forgotten.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/003/005/006.
    """
    req_id = "MULTISEG-004"


# --------------------------------------------------------------------------
# MULTISEG-005: forward.rs needs no changes -- confirmed, not just assumed
# --------------------------------------------------------------------------

class ForwardingLoopUnchanged(Requirement):
    """REQUIREMENT-ID: MULTISEG-005

    Step 5 of the multi-segment TUN plan. Confirms (rather than merely
    assumes, per the ideas doc's own "Still unverified" caveat) that
    `src/forward.rs` needed zero code changes. `run_mesh`, `spawn_tun_writer`,
    `spawn_peer_reader`, and `ForwardCtx` already took `peers`/`tun_tx` as
    plain parameters/fields with no daemon-wide assumption baked into their
    own bodies — the daemon-wide-ness lived entirely in what
    `MeshManager::attach_tun` passed them, not in `forward.rs` itself. Once
    MULTISEG-002/003 made that a per-network `peers`/`tun_tx` pair, the same
    loop runs once per network's TUN reader task (mirroring the pre-existing
    one-writer/one-reader-task-per-`attach_tun`-call pattern, now called once
    per network instead of once per daemon) with no logic change. The
    per-packet ingress anti-spoof check (`evaluate_inbound`, a peer may only
    source packets from its own mesh IP) is unaffected: it validates a
    datagram against the specific peer that sent it, already scoped to one
    connection regardless of how many networks or tables exist elsewhere.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/003/004/006.
    """
    req_id = "MULTISEG-005"


# --------------------------------------------------------------------------
# MULTISEG-006: remaining daemon-wide `self.peers`/`self.mesh_ctx()` call
# sites updated to their network-scoped equivalents
# --------------------------------------------------------------------------

class RemainingPeerTableCallSitesScoped(Requirement):
    """REQUIREMENT-ID: MULTISEG-006

    Step 6 of the multi-segment TUN plan. Beyond the sites the ideas doc
    enumerated (`accept.rs`'s `self.ctx.peers.add(...)`, already
    network-scoped via the `MeshCtx` each accept handler already carries as a
    struct field — needed zero changes, confirmed; `runtime.rs`'s
    `leave_network`/`kick_member`; `create_join.rs`'s create/join paths,
    covered by MULTISEG-002/003/004 directly), a full-crate re-grep (not
    trusting the doc's now-stale line numbers) found three more daemon-wide
    `self.peers`/`self.mesh_ctx()` sites the doc's Step-6 pass had not
    enumerated: `daemon/mesh/admin.rs`'s `admin_add` (finding the live
    connection to send an `AdminGrant` over), `daemon/mesh/publish.rs`'s
    `store_and_publish_group` (collecting seed peers for a re-publish after
    `tetron accept`-style admission), and `daemon/mesh/diagnostics.rs`'s
    `network_status` (building `tetron status`'s per-peer connection info).
    All three now resolve `self.networks.get(network)` and read that handle's
    own `peers` table instead of a daemon-wide one. `runtime.rs`'s
    `leave_network` (closing connections gracefully before teardown) and
    `kick_member` (via `mesh_ctx_for(network)` replacing `self.mesh_ctx()`)
    were fixed as part of the same sweep. None of these needed the
    `_for_network`/`_by_network`-suffixed `PeerTable` methods themselves to
    change (see MULTISEG-002's note on why `peers.rs` has zero code changes)
    — only which table instance each call site reads.

    Found: 2026-07-18, `feat/multi-segment-tun` branch (the three
    previously-unenumerated sites found via `cargo build` after
    MULTISEG-002/003/004 landed, not via grep — the compiler caught what a
    line-based grep across multi-line `self\n    .peers` call chains missed).
    Landed together with MULTISEG-002/003/004/005.
    """
    req_id = "MULTISEG-006"


# --------------------------------------------------------------------------
# MULTISEG-007: join-side anti-spoof false positive on a subnet-diverging
# network -- found via live 3-machine testing, not caught by reconcile.py
# --------------------------------------------------------------------------

class JoinSideIpDerivationFixed(Requirement):
    """REQUIREMENT-ID: MULTISEG-007

    Found live-testing MULTISEG-002..006 on 3 real machines (aorus
    coordinating two networks at once: `multiseg-test-a` on the node-wide
    default subnet, `multiseg-test-b` on an explicit `--subnet 10.77.0.0/16`
    diverging from it) — `reconcile.py` was green throughout MULTISEG-002..006
    and never caught this; it is a real functional bug, not a lint/build gap.

    **Symptom:** every real packet from the coordinator (aorus) to a member
    (x10sra) on the subnet-diverging network was silently dropped by the
    member as `DropSpoof` ("dropped inbound packet with spoofed source IP"),
    100% loss, while the identical topology on the network sharing the node's
    default subnet worked fine (`multiseg-test-a`, aorus<->xps). The QUIC
    control connection itself was healthy (`tetron status` showed a live,
    connected peer on both sides) — only the data-plane anti-spoof check
    (`forward::evaluate_inbound`, "a peer may only source packets from its
    own mesh IP") was failing.

    **Root cause:** `daemon/mesh/join.rs`'s `join_mesh_shared` computed both
    its own `my_ip` (`identity.local_ip()`) and the coordinator's `remote_ip`
    (`identity.derive_ip(&remote_id)`) via `IrohIdentityProvider`'s trait
    methods — bound to the single subnet baked into `MeshCtx.identity` at
    daemon boot (the node-wide default, `config::node_subnet()`), which
    MULTISEG-002's per-network `MeshCtx` restructuring left unchanged (every
    network's `MeshCtx` clones the same node-wide-subnet identity provider;
    only `peers`/`tun_tx` became per-network). `create_network_inner`
    (create_join.rs) and `join_network_inner`'s own subnet resolution
    (create_join.rs, MULTISEG-004) both correctly derive a network-scoped
    `my_ip` via the free `membership::derive_ip(identity, network_subnet)`
    function when the network's subnet differs from the identity's default —
    but that correct value never reached `join_mesh_shared`, because
    `JoinParams` (the struct threading per-join inputs into it) had no
    `my_ip` field at all, so `join_mesh_shared` recomputed its own (wrong)
    value from scratch. The `remote_ip` used to register the coordinator's
    connection for the anti-spoof check (`register_mesh_peer` ->
    `spawn_peer_reader`'s `peer_ip` parameter) was `identity.derive_ip`
    against the same wrong node-wide subnet, landing outside the network's
    real range — so a legitimate packet correctly sourced from the
    coordinator's real (correctly-derived, roster-authoritative) IP failed
    the `src_ip == expected_peer_ip` check every time.

    A second, lower-severity effect of the same root cause: `my_ip` also fed
    `persist_join_config`, so the wrong value was written to
    `NetworkConfig.my_ip` on disk for a subnet-diverging network — the live
    in-memory value (correctly set elsewhere, from `JoinContext.my_ip`) papered
    over this at runtime, but a fresh restart reading the persisted value back
    could have surfaced it. Fixed by the same change.

    **Fix:** `JoinParams` gained a `my_ip: Ipv4Addr` field; `run_join_handshake`
    (create_join.rs) now passes `ctx.my_ip` (the already-correct,
    network-scoped value) through instead of `join_mesh_shared` recomputing
    it. `remote_ip` is now looked up from the just-admitted `members` roster
    returned by `perform_join_handshake` (authoritative, network-scoped, and
    already available at that point in the function) rather than
    re-derived; `identity.derive_ip(&remote_id)` is kept only as a defensive
    fallback for the practically-impossible case of the coordinator not
    being in its own roster.

    **Not fixed, found harmless on inspection:** `accept.rs`'s
    `handle_connection` has an analogous-looking fallback
    (`member_ip.unwrap_or_else(|| self.ctx.identity.derive_ip(&remote_id))`)
    for a *fresh, not-yet-admitted* joiner. Traced its only consumer,
    `admit_peer`'s `_suggested_ip` parameter — the leading underscore was
    already a deliberate signal it's unused; `validate_admission` always
    recomputes the authoritative IP via `membership::assign_ip(&s.members,
    &remote_id, s.subnet)` (correctly network-scoped, since `s.subnet` is
    `NetworkState`'s own per-network field), and that value — not the
    fallback — is what actually gets registered. Left as-is rather than
    changed as a drive-by; a real (if confusing) piece of dead input, not a
    functional bug.

    Live-verified after the fix, same 3-machine topology: 0% loss both
    directions on the subnet-diverging network, `reconcile.py` green (build,
    clippy 0 warnings, tests, all identity/regression gates).

    Found: 2026-07-18, `feat/multi-segment-tun` branch, live 3-machine
    testing (aorus/xps-17-9720/x10sra) per `DO-NOT-COMMIT/TESTING.md`'s
    "multi-segment TUN" run.
    """
    req_id = "MULTISEG-007"


# --------------------------------------------------------------------------
# IPV6-001..003: per-network IPv6 addressing, the follow-up MULTISEG-003
# explicitly deferred ("Making IPv6 fully per-network would mean ... a
# larger, separate change")
# --------------------------------------------------------------------------

class PerNetworkIpv6Derivation(Requirement):
    """REQUIREMENT-ID: IPV6-001

    Follow-up to MULTISEG-003's flagged limitation: `derive_ipv6(identity)`
    is identity-only, so a node's peer IPv6 address is identical across
    every network it joins — unlike IPv4, which is genuinely per-network
    (`derive_ip(identity, subnet)`). This makes `derive_ipv6` take the
    network's own public key too, mirroring the IPv4 shape, so each
    network's v6 range becomes its own real, disjoint, routable block
    instead of one address shared across every network a node belongs to.

    **New signature:** `derive_ipv6(identity: &EndpointId, network: &EndpointId)
    -> Ipv6Addr`.

    **Structural split (decided 2026-07-18, not just "shrink the hash"):**
    byte 0 fixed `0x02` (unchanged, keeps the address inside the existing
    `200::/7` product-documented range) + a 48-bit **network-prefix**
    (bytes 1-6, `blake3(network.to_string())` truncated to 6 bytes) + a
    72-bit **peer-part** (bytes 7-15, `blake3(format!("{identity}:{network}"))`
    truncated to 9 bytes). The network-prefix is the part that actually
    matters: it is *only* a function of the network's public key, so every
    member of a given network shares the same 56-bit prefix (`0x02` + 48
    bits), giving that network a real `/56` CIDR block a route can target —
    without this structural split, folding "network" into the hash input
    alone would still produce addresses fully interleaved with every other
    network's, with no CIDR block to route (this is what IPV6-003 needs).
    The peer-part deliberately mixes in `network`, not just `identity`, so
    the same identity gets an unrelated peer-part in each network it joins
    — this closes a cross-network grinding-reuse loophole that would
    otherwise undermine IPV6-002's collision defense (see that
    requirement).

    **No collision-index** (confirmed 2026-07-18 via birthday-paradox math
    at the more realistic 1%-probability threshold, not just 50%): IPv4's
    default `/24` needs only ~2-3 nodes for a 1% collision chance (why
    `collision_index`/`assign_ip`'s rotation exists at all), while a
    72-bit peer-part needs ~3.1 billion nodes for the same 1% risk —
    astronomically beyond any realistic mesh size. `assign_ip`'s
    IPv4-style rotate-on-collision approach is not extended to v6; a
    genuine (non-adversarial) collision is not expected to ever occur.
    Deliberate grinding is a different threat model, handled separately by
    IPV6-002.

    **Call-site audit** (every non-test caller of the old identity-only
    signature, found via full-crate grep, each now threads through the
    relevant network's own public key — already in scope at every site
    below via `NetworkState.network_public_key`, `NetworkHandle.network_key`,
    or an explicit `network`/`net_pubkey` parameter already being passed
    for other reasons):
    - `daemon/mesh/create_join.rs` — create/join success paths building
      `my_ipv6`/roster `ipv6` fields for IPC responses.
    - `daemon/mesh/accept.rs` — `spawn_admitted_member_tasks` and the two
      other sites registering a peer's v6 route for the anti-spoof-adjacent
      data plane.
    - `daemon/mesh/diagnostics.rs` — `tetron status`'s per-network,
      per-member v6 display.
    - `daemon/mesh/join.rs` — registering the coordinator's v6 on initial
      join.
    - `daemon/mesh/coordinator.rs`, `daemon/mesh/reconverge.rs` — peer
      removal/pruning, which must recompute the same network-scoped v6 that
      was used to register the peer, or the removal is a no-op key-miss.
    - `daemon/mesh/runtime.rs`, `daemon/mod.rs` — `activate()` and
      `create_and_attach_network_tun`'s own-address computation, feeding
      `route_self_loopback` (each network's loopback self-route must match
      that network's own derived v6, not one node-wide value — the same bug
      shape as MULTISEG-007 if left identity-only here).

    `src/peers.rs`'s `PeerTable` needs no structural change (its `v6:
    Arc<FastDashMap<Ipv6Addr, PeerEntry>>` is already a distinct instance
    per network since MULTISEG-002 gave every `NetworkHandle` its own
    `PeerTable`) — only what value each call site above computes as the key
    changes.

    Existing unit tests (`test_derive_ipv6_deterministic`,
    `test_derive_ipv6_in_200_range`, `test_derive_ipv6_different_identities_differ`)
    update for the new signature; new coverage added for same-identity
    producing different addresses across two networks, and the network-
    prefix being shared across different identities on the same network.

    Found: 2026-07-18, decided during a design discussion following the
    MULTISEG-002..007 merge; implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-001"


class Ipv6CollisionRejectedAtAdmission(Requirement):
    """REQUIREMENT-ID: IPV6-002

    Defense-in-depth alongside IPV6-001's structural collision-resistance:
    mirrors `validate_admission`'s existing IPv4 behavior (`accept.rs`,
    "IP collision: {ip} already assigned" — a different identity already
    holding a candidate's derived address is rejected, not silently
    admitted) for IPv6. Scoped explicitly against a *deliberately grinded*
    collision (an adversary generating on the order of 2^36 keypairs to
    force a specific 72-bit peer-part match is realistically feasible with
    modest hardware), not the accidental case — IPV6-001's math already
    makes accidental collision astronomically unlikely; this closes the
    much narrower gap that a probabilistic argument alone does not cover
    for an adversarial actor.

    Since `Member` carries no persisted `ipv6` field (v6 addresses are
    never transmitted or signed — always freshly re-derived locally by
    every node, confirmed by inspection of the `Member` struct), the check
    cannot look up a stored value. `validate_admission` instead recomputes
    `derive_ipv6(&m.identity, &s.network_public_key)` for every existing
    roster/approved entry and compares against the joiner's candidate
    address — an O(n) scan, cheap at realistic roster sizes, same shape as
    the existing hostname-collision scan a few lines above it in the same
    function.

    On a collision against a *different* identity, admission is rejected
    with `"IPv6 collision: {addr} already assigned"` (mirroring the v4
    message's wording) — the joiner's admission fails outright. Unlike
    IPv4, there is no collision-index to rotate to and retry (IPV6-001
    deliberately has none), so this is a hard denial, not a resolution
    step. A re-add of the *same* identity (e.g. a reconnect) is not a
    collision, matching `assign_ip`'s existing same-identity exemption.

    Found: 2026-07-18, decided as part of the same design discussion as
    IPV6-001 (explicit user decision to add this check rather than accept
    the residual grinding risk); implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-002"


class PerNetworkIpv6RouteInstallation(Requirement):
    """REQUIREMENT-ID: IPV6-003

    Closes the limitation MULTISEG-003 explicitly flagged and deferred:
    "IPv6 mesh reachability works on one segment only" — `activate()`
    previously installed one system-wide `200::/7 -> <tun>` kernel route,
    guarded by an `installed_peer_range_route` bool so only the *first*
    network encountered got it (a deterministic, documented limitation,
    not a last-writer-wins race, but still a real one: every other
    network's peers were unreachable over IPv6). This was only fixable
    once IPV6-001 existed — a single shared `200::/7` superset has no
    narrower per-network block a route could target; disjoint per-network
    `/56` prefixes do.

    `tun::route_peer_range` changes signature from `(tun_name: &str)` to
    take the specific prefix/width to install (the network's own `/56`
    block: `0x02` + IPV6-001's 48-bit network-prefix, peer-part bits
    zeroed) instead of the hardcoded `Ipv6Addr::new(0x0200, ..), 7`
    literal — both the Linux (netlink `RouteMessageBuilder`) and macOS
    (`route add -inet6`) implementations swap the constant for the passed-
    in value. A new `membership::ipv6_network_prefix(network: &EndpointId)
    -> Ipv6Addr` helper computes the zeroed-suffix prefix address from
    IPV6-001's derivation, reused by both call sites below and by tests.

    Both call sites — `daemon/mod.rs`'s `create_and_attach_network_tun`
    and `daemon/mesh/runtime.rs`'s `activate()` — drop their "only the
    first network" bookkeeping entirely and call `route_peer_range`
    unconditionally per network: routes no longer collide, since each
    network's `/56` is disjoint from every other's (birthday math on a
    48-bit space, IPV6-001). `route_self_loopback`'s own-address argument
    switches from `derive_ipv6(identity)` to the network-scoped
    `derive_ipv6(identity, network)` at both sites, closing the same bug
    shape MULTISEG-007 fixed for IPv4 (a node-wide value used somewhere
    that needed to be network-scoped) before it can ever manifest here.

    `AGENTS.md`'s multi-segment TUN section and MULTISEG-003's own spec
    docstring both documented "IPv6 mesh reachability works on one segment
    only" as a known, unresolved limitation needing a product decision —
    that decision was made (IPV6-001) and this requirement is what acts on
    it; both docs get their limitation note removed/updated to reflect the
    resolved state once this lands and is live-tested.

    Needs its own live multi-machine test: a node dual-homed on two
    networks reaching a peer over IPv6 on *both* networks simultaneously
    (the exact scenario MULTISEG-003 could not support), not just IPv4 as
    the earlier MULTISEG live-testing pass covered.

    Found: 2026-07-18, decided as part of the same design discussion as
    IPV6-001/002; implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-003"


# --------------------------------------------------------------------------
# MACOS-001: fix macOS route_peer_range's hardcoded pre-fork CGNAT literal
# --------------------------------------------------------------------------

class MacosRoutePeerRangeUsesActualSubnet(Requirement):
    """REQUIREMENT-ID: MACOS-001

    `src/tun.rs`'s macOS variant of `route_peer_range` (needed because
    macOS's point-to-point `utun` doesn't reliably self-install either
    range the way Linux's kernel does) hardcoded the pre-fork upstream
    literal `100.64.0.0/10` for the IPv4 family, regardless of the
    network's actual configured subnet. Since tetron's own default is
    `10.88.0.0/24` (SUBNET-011), this silently misrouted IPv4 on every
    macOS-joined network by default — the exact same bug shape as
    MULTISEG-007 (a hardcoded/wrong value used where a network-specific
    one was needed), just never caught because no macOS build/test has
    run in CI (`build-macos` is `if: false` in both `nightly.yml` and
    `release.yml`, specifically citing this bug as the reason it's
    gated off) or on real hardware since the bug was first found
    2026-07-17.

    **Fix:** `route_peer_range` (both the Linux and macOS `cfg` variants,
    which must share a signature) gained a `subnet: crate::membership::Subnet`
    parameter. The macOS body now formats `subnet` into a real CIDR string
    (`format!("{base}/{prefix}")`) and installs *that* as the `-inet` route
    instead of the literal. Linux's variant receives the same parameter
    (as `_subnet`, deliberately unused — the kernel already installs the
    correct IPv4 connected route from the interface's own address/netmask
    automatically on link-up, so Linux never needed this to begin with).
    Both call sites (`daemon/mod.rs`'s `create_and_attach_network_tun`,
    which already had the network's `Subnet` as its own parameter, and
    `daemon/mesh/runtime.rs`'s `activate()`, which reads it from
    `handle.state.read().unwrap().subnet`) now thread the real value
    through instead of the function inventing its own.

    **Not yet verified on real hardware or in CI** — found and fixed via
    direct code read (this bug cannot be exercised or caught by
    `reconcile.py`'s Linux-only build/test/clippy gates, same as
    MULTISEG-007 needed live multi-machine testing to surface). Real
    verification (native build + `sudo tetron up` + join an existing
    mesh + confirm IPv4 reachability, mirroring the live-testing rigor
    already applied to MULTISEG-002..007 and IPV6-001..003) is a
    separate, subsequent step on real Apple Silicon hardware — this
    commit is the code fix only. `build-macos`'s `if: false` should stay
    in place until that real-hardware pass actually happens; flipping it
    based on this fix alone (unverified) would repeat exactly the mistake
    the CI comment was written to prevent.

    Found: 2026-07-17 (original discovery, logged in
    `DO-NOT-COMMIT/TODO.md`'s "macOS port" section). Re-confirmed still
    present 2026-07-18 while auditing macOS support end to end. Fixed:
    2026-07-18.
    """
    req_id = "MACOS-001"


# --------------------------------------------------------------------------
# MACOS-002: capture real route(8) output instead of only its exit code
# --------------------------------------------------------------------------

class MacosRouteCommandOutputCaptured(Requirement):
    """REQUIREMENT-ID: MACOS-002

    Found live 2026-07-18 diagnosing `MACOS-001` on real Apple Silicon
    hardware: even after that fix, a `tetron down` / `tetron up` cycle
    left the network's IPv4 peer route missing from the routing table,
    silently breaking outbound connectivity (inbound still worked — the
    forwarder's inbound path has no destination check and writes straight
    to the TUN regardless of routing, so only *outbound* traffic showed
    the symptom, and the daemon logged no error at all). Manually running
    the *exact same* `route -n add -inet -net <cidr> -interface <tun>`
    command as root, standalone, worked correctly and the route appeared.
    So the command is right; something about the daemon's own execution
    of it differs, and `route_peer_range`'s exit-code-only check
    (`.status()`, discarding stdout/stderr) couldn't distinguish "really
    succeeded" from "exited 0 but the OS didn't do what was asked" —
    there was no way to see what actually happened.

    **Fix (observability only, not a behavior fix):** `route_peer_range`'s
    macOS variant now uses `.output()` instead of `.status()` for both
    the pre-add `delete` and the `add` themselves, logging the real exit
    status, stdout, and stderr — `debug` level for the delete (failure
    there is normal, it's cleaning up a possibly-nonexistent stale route)
    and for a successful add, `warn` level with the full output on a
    failed add (replacing the old bare `anyhow::ensure!` that discarded
    whatever `route(8)` actually printed).

    **Deliberately scoped narrow**: this is the one code path currently
    being live-debugged, not a sweep of every `Command::new(...).status()`
    call in `tun.rs` (e.g. `route_self_loopback` has the identical
    blind-exit-code pattern and is not touched here) — logged as a
    follow-up, not done now, since widening scope here would slow down
    the actual diagnosis this exists to unblock.

    Found: 2026-07-18, live-debugging `MACOS-001` on real Apple Silicon
    hardware (M1 MacBook Pro) after the fix alone didn't restore
    connectivity across a down/up cycle. Root cause of *why* the daemon's
    own route add doesn't take effect is still open — this requirement
    only adds the visibility needed to find it.
    """
    req_id = "MACOS-002"


# --------------------------------------------------------------------------
# MULTISEG-008: member-side NetworkState subnet still defaulted to the
# node-wide subnet — one MULTISEG-004 call site the original sweep missed
# --------------------------------------------------------------------------

class MemberJoinNetworkStateSubnetFixed(Requirement):
    """REQUIREMENT-ID: MULTISEG-008

    `MACOS-002`'s new logging found the actual root cause behind
    `MACOS-001` still not restoring IPv4 connectivity across a `tetron
    down`/`up` cycle on macOS: `route_peer_range` was correctly threading
    through whatever subnet it was given, but the subnet it was *given*
    was wrong. `daemon/mesh/join.rs`'s `build_member_state` — the
    function that builds a joining/reconnecting **member**'s live
    `NetworkState` — still constructed it with `subnet:
    crate::config::node_subnet()` (the node-wide default), a leftover
    from before multi-segment TUN existed (its own comment said so
    explicitly: `"SUBNET-010: single-TUN node — subnet comes from the
    persisted node cache ... not the network record"`).

    Every *other* `NetworkState` construction site was updated during
    `MULTISEG-004`'s sweep to use the network's own resolved subnet
    instead (`create_network_inner`, `restore_coordinator_network`, the
    DHT-fallback and try-fetch member paths in `create_join.rs`) — this
    one, reached only via the live member join/reconnect path
    (`join_mesh_shared` → `build_member_state`), was missed. Not
    macOS-specific at all: this is a data-model bug in the daemon's
    in-memory state, present on every platform. It went unnoticed until
    now for two independent reasons: (1) on Linux, IPv4's connected route
    is installed automatically by the kernel from the interface's own
    address/netmask — `route_peer_range`'s Linux variant never reads its
    `subnet` parameter at all, so a wrong `NetworkState.subnet` had no
    IPv4 symptom there; (2) IPv6's routing (`IPV6-003`) derives its
    prefix from `network_key`, never from `subnet`, so it was unaffected
    either way. `MACOS-001` was the first code path on any platform to
    actually *read* `NetworkState.subnet` for something user-visible
    outside of admission bookkeeping, which is what finally surfaced this.

    **Fix:** `JoinParams` gained a `network_subnet: crate::membership::Subnet`
    field, populated from `JoinContext.network_subnet` (already correctly
    resolved by the caller before dialing, per `MULTISEG-007`'s `my_ip`
    fix — the exact same pattern, same missing thread, same root cause
    class). `build_member_state` now takes `subnet` as a parameter
    instead of computing its own default.

    Found: 2026-07-18, live-debugging `MACOS-001`/`MACOS-002` on real
    Apple Silicon hardware (M1 MacBook Pro) — a `tetron down`/`up` cycle
    on a member of a subnet-diverging network installed a route for the
    *node's default* subnet instead of that network's actual one, so
    outbound IPv4 traffic had no working route (inbound still worked,
    since the forwarder's inbound path has no destination/routing
    dependency). Not yet re-verified live after this fix — that's the
    immediate next step, same M1 hardware, same reproduction (join a
    subnet-diverging network, `down`, `up`, confirm the route now matches
    the network's real subnet and IPv4 connectivity survives the cycle).
    """
    req_id = "MULTISEG-008"


# --------------------------------------------------------------------------
# STATUS-001: expose each network's OS TUN interface name in `tetron status`
# --------------------------------------------------------------------------

class StatusShowsTunInterfaceName(Requirement):
    """REQUIREMENT-ID: STATUS-001

    Found 2026-07-18 auditing the CLI/IPC command surface now that a node
    can belong to several real, isolated networks (multi-segment TUN,
    `MULTISEG-002..007`): `NetworkHandle.tun_name` has existed in the
    daemon since that work landed, but was never put on the `NetworkStatus`
    wire type or printed by `tetron status`. With one network this never
    mattered; with several, there was no way to know which OS interface
    (`tun0`, `tun1`, ...) belongs to which network without guessing from
    `ip link show` order or grepping daemon logs — and that matters for
    writing host-firewall rules per network (see `STATUS-001`'s companion
    docs fix for the previously-fictional `iifname "tetron"` example).

    **Fix:** `tetron-proto::ipc::NetworkStatus` gained a `tun_name: String`
    field (`#[serde(default)]` so an older daemon's response — one built
    before this field existed — still decodes against a newer CLI, and a
    stored/replayed old test fixture still deserializes). `diagnostics.rs`'s
    `network_status()` populates it from `h.tun_name.lock().unwrap().clone()`
    at both construction sites (the normal path and the state-lock-poisoned
    fallback). `tetron status`'s text renderer (`cli/status.rs::print_network`)
    prints it as an `interface <name>` line alongside the existing `id`
    line, suppressed while the value is still the pre-attach placeholder
    (`"pending"`) or empty. `--json` gets it for free since `networks` in
    the JSON status output is `NetworkStatus` serialized directly.

    Found: 2026-07-18, same audit pass as the other "Multi-network
    command-surface follow-ups" items logged in `DO-NOT-COMMIT/TODO.md`.

    **Addendum, 2026-07-18 — companion docs fix**: `AGENTS.md`'s
    `MINIMAL-010` note and `docs/HOWTO.md`'s port-restriction example both
    showed `nft add rule inet filter input iifname "tetron" ...` — but
    `tun::create()` never calls `.name(...)` on the `tun` crate's
    `Configuration`, so the OS always auto-assigns `tun0`/`tun1`/etc. This
    predates multi-segment TUN entirely (the single old shared device was
    never actually named `tetron` either); with N networks there are now N
    auto-named interfaces and no fixed name to reference even in
    principle. Both docs now show a real `tun0` example and point at
    `tetron status`'s new `interface` line (this requirement) or
    `ip link show` for finding the right interface per network, instead of
    a name that was always fictional.
    """
    req_id = "STATUS-001"


# --------------------------------------------------------------------------
# ADMIN-ADD-NETWORK-SCOPE: resolve_peer_name scoped to the target network
# --------------------------------------------------------------------------

class AdminAddResolvePeerNameNetworkScoped(Requirement):
    """REQUIREMENT-ID: ADMIN-ADD-NETWORK-SCOPE

    Re-examined 2026-07-18 while auditing the CLI/IPC command surface for
    multi-segment TUN: `resolve_peer_name(name: &str)` (`daemon/mesh/
    runtime.rs`) searched *every* joined network's roster for a hostname
    match and returned the first hit — it had no `network` parameter at
    all, even though its only caller, `admin_add(network: &str, peer_str:
    &str)` (`daemon/mesh/admin.rs`), already has the target network in
    scope and never passed it through. Hostnames are only guaranteed
    unique *within* one network's roster (`resolve_collision` at
    admission), so with two joined networks each having an `alice`,
    `tetron admin <net-A> add alice` could resolve to network-B's `alice`
    instead of network-A's.

    **Not a silent-wrong-grant security bug**: `admin_add` looks up the
    resolved identity in `network`'s *own* `PeerTable`
    (`h.peers.peers_for_network_with_conn(network)`, MULTISEG-002's
    per-network table) before sending the `AdminGrant`, and errors with
    "could not find an active connection to `<identity>` on `<network>`"
    if that identity isn't actually connected there. A cross-network
    mis-resolution fails closed, not silently — but it is a real
    usability bug: if network-A's real, currently-connected `alice`
    exists, but `resolve_peer_name` happened to hit network-B's `alice`
    first (DashMap iteration order), the command failed with a confusing
    "could not find an active connection" error even though the intended
    target was right there and reachable, with no indication the wrong
    identity was resolved behind the scenes.

    Same root category as the short-id prefix-collision bug fixed
    2026-07-17 in `resolve_short_id_any_network` (that one now rejects
    ambiguous/too-short matches instead of guessing — see
    `ADMIN-ADD-EASY-ID`'s addendum). This fix is smaller: no separate
    "collect all matches, error on >1" step is needed the way the
    short-id fix needed one, because scoping the search to one network's
    roster makes cross-network ambiguity structurally impossible rather
    than something to detect after the fact.

    **Fix:** `resolve_peer_name` now takes `network: &str` and looks up
    the hostname match only in that network's own roster
    (`self.networks.get(network)`), instead of iterating `self.networks`.
    `admin_add`'s call site now passes its own `network` parameter
    through — the CLI already requires `tetron admin <network> add
    <peer>`, so the value was always available, just unused for this
    lookup. The short-id fallback (`resolve_short_id_any_network`) stays
    cross-network and unchanged: it already rejects ambiguous/too-short
    prefixes rather than guessing, so it was never the unsafe half of
    this function.

    Found: 2026-07-16 (original, less precise write-up). Root cause
    re-examined and narrowed 2026-07-18. Fixed: 2026-07-18.
    """
    req_id = "ADMIN-ADD-NETWORK-SCOPE"


# --------------------------------------------------------------------------
# STRANDED-COORDINATOR-WARN: warn before a sole-coordinator leave strands members
# --------------------------------------------------------------------------

class LeaveWarnsWhenSoleCoordinatorHasOtherMembers(Requirement):
    """REQUIREMENT-ID: STRANDED-COORDINATOR-WARN

    Found live 2026-07-18 auditing the CLI/IPC command surface for
    multi-segment TUN: `leave_network` only tears down the *caller's* own
    participation — correct, and the only sane behavior for a command
    that by definition can't act on other nodes. But if the caller was
    the network's only coordinator, every other member is left in a
    network with no one able to admit joiners, mint invites, or kick —
    and had no signal this happened beyond eventually noticing the
    (former) coordinator shows "offline" forever. Live-confirmed this
    exact state 2026-07-18 (a second node still showed a test network
    with the departed sole coordinator as `offline` after it left).

    Not fixable in the sense of a guaranteed farewell broadcast — leave
    is a local, unilateral action, and a coordinator can't force delivery
    of a message to peers who may be offline anyway. What matters more
    than a farewell, though, is that this state was *permanent*: once
    the sole coordinator is gone, no remaining member can ever recover
    coordination capability on their own — `admin add`, `kick`, `invite
    create`, and `nuke` all require holding the network's secret key,
    and there is no path to obtain it after the fact. A warn-and-`--force`
    design (this requirement's first cut, superseded below same day)
    undersold that: it read like "some inconvenience," not "irreversible
    loss of governance for everyone else."

    **Design, revised same day (USER's call, 2026-07-18): don't just warn
    about the strand, actively prevent it where possible.** Before
    leaving, a sole coordinator with other members now auto-promotes
    every member reachable *right now* to co-coordinator, the same
    `AdminGrant` mechanism `tetron admin add` uses (already the
    project's own recommended practice — README/HOWTO tell users "every
    fully trusted member should be a co-coordinator to avoid a single
    point of failure"; this makes that happen automatically at the exact
    moment it matters most instead of requiring the leaver to have
    already done it). This is strictly better than an earlier
    `--transfer-to <peer>` idea (pick one successor) — it doesn't
    require the leaver to decide who the "right" successor is, and
    spreads trust across everyone present rather than creating a new
    single point of failure.

    **The one irreducible limit:** the network's secret key only ever
    travels over a live authenticated connection (`AdminGrant`) — never
    the public signed blob, since that would defeat the point of it
    being secret. A member who is offline at the exact moment `tetron
    leave` runs cannot be promoted, full stop; there is no way to
    pre-stage a grant for them. So the command still refuses by default
    (destructive-adjacent action, same `has_other_members && !force`
    shape `NUKE-CONSENSUS` already established) — but only for the
    residual case: members that auto-promotion could not reach. Anyone
    who *was* reachable is promoted regardless of whether the command
    ultimately proceeds or is blocked on someone else.

    **Fix:** `admin_add`'s `AdminGrant`-sending logic was factored out of
    `daemon/mesh/admin.rs` into `MeshManager::grant_admin_key(network,
    identity) -> Result<(), String>` — identity-only, no hostname
    resolution — so `leave_network` can call it directly for each other
    member without going through `admin_add`'s full IPC-message
    round-trip. `leave_network(&self, network: &str, force: bool)`, when
    `!force`, computes the sole-coordinator check as before
    (`coordinator_count(&roster) <= 1` while the caller itself
    `is_coordinator`), then — only if that's true and other members
    exist — partitions those other members into "currently connected"
    (via `handle.peers.peers_for_network_with_conn`) and not, calls
    `grant_admin_key` for each connected one, and only returns an error
    (naming the short ids of whoever remains unreachable, and how many
    were already promoted) if any member couldn't be saved from
    stranding. If every other member ends up promoted, the leave
    proceeds and the success message reports how many were promoted.
    `tetron leave --force` still bypasses the entire check (no
    auto-promotion attempted either) — an explicit, informed choice to
    abandon the network as-is, matching `nuke --force`'s existing
    semantics of "I know, don't check." Internal callers that already
    made the leave decision elsewhere still always pass `force: true`
    and so skip auto-promotion too: `nuke_network`'s own self-leave
    (tombstone already published — promoting anyone right before
    destroying the network is pointless) and
    `handle_removed_from_network` (reacting to an already-applied
    roster change — kicked or pruned — where granting the key out
    doesn't make sense either).

    **Covered by a unit test** (`leave_blocks_on_sole_coordinator_with_
    unreachable_members`, `daemon/mod.rs`'s `headless_tests`): a
    bare-bones sole-coordinator network with two other members, neither
    connected, confirms the leave is blocked with both short ids named
    in the message and the network handle left intact, then confirms
    `--force` bypasses it. **The "successfully auto-promoted a reachable
    member" happy path is not covered by an automated test** — it needs
    a real, live QUIC connection between two endpoints (`grant_admin_key`
    calls `conn.open_bi()` on an actual `iroh::endpoint::Connection`),
    which this codebase has no lightweight in-process test harness for;
    every other real-connection scenario in this project is verified via
    live multi-machine testing instead, not unit tests.

    Found: 2026-07-18, same audit pass as `STATUS-001` and
    `ADMIN-ADD-NETWORK-SCOPE`. Fixed: 2026-07-18 (warn+force cut);
    redesigned same day to auto-promote before blocking.

    **Not yet live-tested on real multi-machine hardware as of this
    writing** — verified via `reconcile.py` (build/clippy/test green)
    and the unit test above only. **Resolved same day — see the
    live-testing addendum below.**

    **Addendum, 2026-07-18 — `--force` is a deliberate, irreversible
    choice; document it as one.** USER's follow-up questions (is
    kick-everyone-then-leave the only way to force-close a network? is a
    zombie network ever desirable? is there still a way to make one?)
    surfaced that `--force` is in fact the *only* remaining path to a
    zombie network (an unreachable member blocks by default; `--force`
    is the sole override), and that this state is irrecoverable — no
    command or recovery flow can ever regenerate a lost network key, so
    once the last coordinator is gone the roster is frozen forever.
    `docs/HOWTO.md` gained a new "Create a zombie network
    (intentionally)" section: what a zombie actually is, the one
    deliberate way to make one (`--force`) plus the one *accidental* way
    (`sudo tetron uninstall` without `tetron leave`-ing first — uninstall
    never attempts a handoff), an explicit "not reversible" callout, and
    three legitimate reasons to want one (deliberately freezing
    membership as a security ceiling, grace-period wind-down without
    forcing an immediate decision on remaining members, throwaway/test
    networks) — plus a pointer to `nuke` for when the actual goal is
    destroying the network rather than merely orphaning it. The `--force`
    flag's own `--help` text (`Command::Leave` in `main.rs`) and the
    daemon's blocking-error message (`leave_network`, when some members
    remain unreachable) both gained an explicit "NOT REVERSIBLE" /
    "not reversible" callout too, so the warning is visible at the
    point of decision, not just in a doc a user may never open.

    **Addendum, 2026-07-18 — live-tested on 3 bare-metal machines
    (590i-aorus-ultra as sole coordinator, xps-17-9720 and x10sra as
    members), both scenarios the original caveat above named.**

    *All members reachable:* aorus created a fresh network, xps and
    x10sra joined, aorus ran `tetron leave` with no `--force` — reply
    was exactly "promoted 2 other member(s) to co-coordinator, then
    left network '...'". Verified both promotions were real, not just a
    local flag flip: `tetron admin <net> list` on each showed itself as
    a key-holder, and each independently minted a working invite
    (`tetron invite <net> create`) after aorus was gone — proof of a
    genuinely usable key, since minting requires a real, valid
    coordinator secret. Bonus check: with two coordinators now, one of
    them (xps) leaving proceeded immediately with no promotion message
    at all, confirming `coordinator_count <= 1` correctly gates the
    whole mechanism.

    *One member offline:* same setup, then `sudo systemctl stop tetron`
    on x10sra to take it offline. `tetron leave` with no `--force` on
    aorus refused, naming x10sra's exact short id and confirming xps
    was already promoted despite the overall block — matching the
    designed message precisely. Verified xps's promotion was still real
    (same admin-list + invite-mint proof) even though the command as a
    whole failed. `tetron leave --force` then proceeded, deliberately
    stranding x10sra. Restarting x10sra's daemon reproduced the exact
    zombie symptom this requirement exists to prevent by default: it
    still showed the network with aorus permanently offline, while its
    connection to the promoted xps stayed live and direct. Confirmed
    x10sra itself — never a coordinator — could still `tetron leave`
    freely with no block, since the check only ever applies to the
    caller's own coordinator status.

    No bugs found in either scenario; behavior matched the design
    exactly on the first live run. `reconcile.py` remained the gate for
    build/clippy/test throughout, matching the discipline established
    for every other destructive-adjacent feature in this project
    (`NUKE-CONSENSUS`, the `CONVERGE-*` fixes).
    """
    req_id = "STRANDED-COORDINATOR-WARN"


# --------------------------------------------------------------------------
# STANDBY-PER-NETWORK: per-network data-plane standby via --network
# --------------------------------------------------------------------------

class UpDownAcceptOptionalNetworkScope(Requirement):
    """REQUIREMENT-ID: STANDBY-PER-NETWORK

    Found 2026-07-18 auditing the CLI/IPC command surface for
    multi-segment TUN: `tetron up`/`tetron down` (`activate()`/
    `deactivate()`, `daemon/mesh/runtime.rs`) were daemon-wide — one
    `MeshManager.active: Arc<AtomicBool>`, every loop over every joined
    network unconditionally. There was no way to take e.g. a "work"
    network's TUN offline at end of day while keeping "home" active, the
    way you'd physically unplug one of two NICs — a real gap once
    multi-segment TUN (`MULTISEG-002..007`) made "several genuinely
    isolated networks on one node" a normal, live configuration rather
    than a theoretical one.

    **Design (the "not yet scoped" gap this requirement closes):**
    `MeshManager.active` is a single flag, but the actual per-packet data
    gate needed to move to be per-network for `--network` to mean
    anything — the daemon-wide flag alone can't represent "net-a is up,
    net-b is on standby" at the same time. `NetworkHandle` gained its own
    `active: Arc<AtomicBool>`, and `forward::spawn_tun_writer` (the
    function that actually gates whether a received packet gets written
    to a TUN device) is now handed each network's own flag
    (`handle.active.clone()`, `attach_tun`) instead of the daemon-wide
    one. `MeshManager.active` survives, repurposed: it now only seeds a
    brand-new network's initial state at create/join/restore time
    (`create_and_attach_network_tun`'s existing "if the VPN is already
    active, bring this new network straight up" check) and is what an
    *unscoped* `activate()`/`deactivate()` call sets across the board —
    an unscoped `tetron up`/`down` is unchanged in effect (every network
    moves together) even though the mechanism underneath is now N
    independent per-network flags rather than one shared flag every
    writer read.

    **`activate`/`deactivate` signatures** gained `network: Option<&str>`.
    `Some(name)` restricts the loop to that one network (erroring if the
    name isn't a currently-joined network, rather than silently
    activating nothing) and uses that network's own `handle.active.swap`
    for idempotency (skip work if already in the target state) instead of
    the old single daemon-wide swap-guard. `None` preserves the original
    behavior exactly: it still flips `MeshManager.active` (for future new
    networks) and iterates every joined network. Both `IpcMessage::Up`
    and `IpcMessage::Down` gained a `#[serde(default)] network:
    Option<String>` field (defaulting to `None` so an old client's
    request still decodes as daemon-wide); `Command::Up`/`Command::Down`
    gained a matching `--network <name>` clap flag (network's local
    display name, as shown by `tetron status`).

    **Status visibility:** `NetworkStatus` gained `active: bool`
    (`#[serde(default)]` for wire back-compat), populated from each
    network's own `handle.active`. `tetron status`'s per-network line
    prints a `·standby·` marker when it's `false`. The top-level
    `StatusResponse.active` (the "up"/"standby" banner) changed from
    mirroring the single daemon-wide flag directly to "is at least one
    network's data plane up" (`statuses.iter().any(|s| s.active)`) — the
    banner's pre-existing meaning ("up" unless everything is on standby)
    is preserved without a wire-format change to that field, now just
    computed instead of stored.

    **Persistence, deliberately unchanged:** per-network standby state is
    not persisted to config, matching the pre-existing daemon-wide
    behavior — `run_daemon` always calls `activate(None, None)`
    unconditionally at boot (before this requirement and after), so a
    daemon restart already brought the whole VPN back up regardless of
    any prior `tetron down`; per-network standby inherits that same
    "doesn't survive a restart" property rather than introducing new
    persisted state.

    **Internal call sites updated:** `run_daemon`'s boot-time
    `activate(None)` → `activate(None, None)`; the shutdown handler's
    `deactivate()` → `deactivate(None)` (`bootstrap.rs`). Every
    `NetworkHandle` construction site (create/join/restore, plus the
    bare-bones test fixture in `attach_tun_is_self_healing_on_reattach_
    and_double_attach`) gained `active: Arc::new(AtomicBool::new(false))`
    — a freshly constructed handle always starts inactive;
    `create_and_attach_network_tun` is the one place that decides whether
    to immediately flip it, based on the daemon's current default state.

    **Live-tested:** not yet on real multi-machine hardware (same caveat
    as `STRANDED-COORDINATOR-WARN`, found in the same audit pass) —
    verified via a new unit test (`activate_deactivate_scope_to_one_
    network_when_given`, `daemon/mod.rs`'s `headless_tests`) that inserts
    two bare-bones networks and exercises scoped activate/scoped
    deactivate/unscoped activate/unscoped deactivate/unknown-network-name
    against real `activate()`/`deactivate()`, asserting exactly which
    network's `active` flag moved at each step. The real OS calls
    (`tun::set_link_up`/`route_peer_range`) fail against the test
    fixture's placeholder device name, which is expected and harmless —
    they're non-fatal everywhere in this codebase (logged as warnings,
    never propagated as an error), so the test's assertions are about
    the flag-scoping logic itself, not real TUN state. `reconcile.py`
    green (build/clippy/test, 216 tests — 215 prior + this one).

    Found: 2026-07-18, same audit pass as `STATUS-001`,
    `ADMIN-ADD-NETWORK-SCOPE`, and `STRANDED-COORDINATOR-WARN`. Fixed:
    2026-07-18.

    **Addendum, 2026-07-18 — live-tested on 4 real machines (3 bare-metal
    Linux: 590i-aorus-ultra/xps-17-9720/x10sra, plus an M1 MacBook Pro
    over macOS).** aorus coordinated two networks on distinct subnets
    simultaneously (`standby-a`, xps as the other member; `standby-b`,
    subnet `10.66.0.0/24`, the Mac as the other member) — `tetron
    status` showed two distinct interfaces (`tun0`/`tun1`) as expected
    (`STATUS-001`). `tetron down --network standby-a` on aorus dropped
    ping to xps to 100% loss while `standby-b`'s ping to the Mac stayed
    at 0% loss throughout, confirming real isolation, not just a status
    flag; `tetron up --network standby-a` recovered it to 0% loss.
    Separately, the Mac ran `tetron down --network standby-b` /
    `tetron up --network standby-b` **on its own side**, specifically to
    exercise the platform-specific route code (`route_peer_range`,
    `set_link_up`/`set_link_down`) `MACOS-001`/`MACOS-002` lived in —
    confirmed the route disappeared from macOS's own routing table on
    down (100% ping loss from aorus) and reappeared cleanly on up (0%
    loss). Finally, unscoped `tetron down`/`tetron up` (no `--network`)
    on aorus was confirmed to still move both networks together,
    matching the pre-existing daemon-wide behavior exactly. No bugs
    found. `reconcile.py` remained the gate for build/clippy/test
    throughout; this run is the real-hardware confirmation the spec
    entry above flagged as outstanding.
    """
    req_id = "STANDBY-PER-NETWORK"


class InstallOutputNamesConcreteAction(Requirement):
    """REQUIREMENT-ID: INSTALL-OUTPUT-001

    `sudo tetron install` used to run entirely silently until "waiting
    for daemon…" -- `ensure_service_installed` wrote the systemd unit /
    launchd plist with no output, and `install_and_start_service` ran
    `systemctl enable/restart` or `launchctl load` via `run_cmd`, which
    itself only ever prints on failure. So a user watching the command
    run saw nothing about what was actually happening on their machine
    (a privileged install writing a system service file and enabling
    it) until it was already done. Flagged live-testing macOS
    (2026-07-19): don't hide privileged/system-level actions just
    because the command that triggers them is short -- the command
    being short is not a reason for its output to be vague about what
    actually happened.

    Fix: `ensure_service_installed` (`src/cli/service.rs`) now prints
    the concrete unit/job name and the exact path being written before
    writing it -- `installing systemd service 'tetron' -> /etc/systemd/
    system/tetron.service` on Linux, `installing launchd job
    'com.tetron.vpn' -> /Library/LaunchDaemons/com.tetron.vpn.plist` on
    macOS. `install_and_start_service` similarly announces the
    enable/restart or load step before running it. Both functions have
    exactly one caller each (`cmd_install`, i.e. `sudo tetron install`),
    so this adds no noise to any other command path (`restart` uses its
    own `restart_service_and_wait`, which was already explicit about
    "restarted").
    """
    req_id = "INSTALL-OUTPUT-001"


class LeaveAcceptsNetworkKey(Requirement):
    """REQUIREMENT-ID: LEAVE-NETWORK-KEY-001

    `tetron leave` previously resolved its network argument only by
    exact match against the local display name (`self.networks.get`,
    a plain map lookup, no dedicated resolver) -- unlike `nuke`/`kick`,
    which both resolve by network key. A user who only has the invite
    key or room id handy (e.g. at uninstall time, having never noted
    the locally-assigned display name) had no way to `leave` at all.

    Fix: new `MeshManager::resolve_network_name_or_key` (`src/daemon/
    mod.rs`), tried at the top of `leave_network`
    (`src/daemon/mesh/runtime.rs`) before any of its existing logic --
    every downstream use of the network argument (the sole-coordinator
    check, connection teardown, config removal, response messages) now
    operates on the resolved local name either way, so behavior for the
    existing local-name path is unchanged byte-for-byte. Tries the exact
    local name first (preserves today's only path untouched); falls back
    to `resolve_network_short_id` (same >=10-char-minimum, ambiguous-
    prefix-rejected rules already used by `nuke`/`kick`) only if that
    fails. Deliberately **not** the same trust posture as `nuke`/`kick`'s
    key-only resolution: `leave` only ever tears down the caller's own
    participation, never mutates another node's roster, so there is no
    destructive-action argument for refusing a local-name match the way
    `resolve_network_short_id`'s own doc comment explains for those two.
    On failure, the combined error names both things that were tried
    (not a known local name, and not a valid/unambiguous key) rather
    than surfacing `resolve_network_short_id`'s raw wording, which
    assumes -- correctly for `nuke`/`kick`, not for `leave` -- that the
    caller was attempting key resolution in the first place.

    `--help`, `AGENTS.md`, `README.md`, and `docs/HOWTO.md` updated to
    document the fallback.
    """
    req_id = "LEAVE-NETWORK-KEY-001"


class StatusOutputRedesign(Requirement):
    """REQUIREMENT-ID: STATUS-002

    `tetron status` is the primary information surface end users have, and
    Erik flagged it as difficult to read and ambiguous: unlabeled fields
    inconsistent with the labeled ones next to them, a bare `id` line with
    no indication of what it identified, and a `join <64-char-hash>` line
    duplicating that same value in full under a stale, actively misleading
    label (a bare room id/public key was never sufficient to join even
    before `LIVE-001`, and is explicitly discovery-only after it).

    Redesigned through iterative mockups in the (gitignored, not shipped)
    `DO-NOT-COMMIT/MOCKUP_tetron_status_output_redesign.md`, landing on:

    - **Daemon header**: `tetron v<version>  state <active|standby>
      endpoint <short>`, plus a `traffic` line (`bytes_tx`/`bytes_rx`,
      previously computed and sent over IPC but discarded by the text
      renderer -- `let _ = (packets_rx, packets_tx, bytes_rx, bytes_tx);`).
      `packets_rx`/`packets_tx` remain unused in text mode, still available
      via `--json`.
    - **Per-network header**: `network <name>   subnet <cidr>   admins
      <online>/<total>   members <online>/<total>   interface <tun_name>`.
      `subnet` is a new `NetworkStatus` field (CIDR string, formatted
      daemon-side from `membership::Subnet`'s bare `(Ipv4Addr, u8)` tuple,
      which has no serde/Display impl of its own) -- previously not
      exposed anywhere, despite subnet collision being an
      explicitly named troubleshooting category in this project
      (`SUBNET_COLLISION.md`, `SUBNET-BUG-001`). `admins online/total`
      needs no new wire field beyond the `is_coordinator` addition below --
      computed client-side in `status.rs` from `net.role.is_coordinator()`
      (self) plus each peer's `is_coordinator` + `connection.is_some()`.
    - **`network_key`**: kept, but truncated to a short prefix (~10 chars,
      matching `resolve_network_short_id`'s own `>=10`-char minimum -- both
      `nuke`/`kick` already accept a prefix, nothing lost), and shown only
      when the viewer's own role for that network is admin/coordinator. A
      plain member can't act on it regardless (`nuke`/`kick` would reject
      them independent of whether they know the value), so showing it to
      them was pure clutter. The NUKE-CONSENSUS pending-proposal hint's
      actionable `tetron nuke <key> ...` suggestion is likewise only
      included when the viewer has that value; a non-admin still sees a
      proposal exists, just without a command they couldn't use anyway.
    - **Peer table**: real column-aligned `role / host / ip / via`, the
      local node included as its own first row (`via` = `(you)`), rendered
      by a new `render_aligned_table` helper (`src/cli/status.rs`) that
      computes real per-column max width across all rows including the
      header -- the pre-existing `table()` helper explicitly does *not* do
      this ("No column alignment in plain mode"), so a new helper was
      needed rather than reusing it. `role` is `admin`/`member`, driven by
      a new `PeerStatus.is_coordinator: bool` field (the data already
      existed internally on `membership::Member`, just never threaded onto
      the wire type). `via` is `direct`/`relay`/`tor`/`offline`/`(you)` --
      covers every `ConnType` plus self plus disconnected, decided
      sufficient with no further states needed.
    - **Deliberately dropped from the default text view**: per-peer IPv6
      (own and peers'), and per-peer connection health (rtt/tx/rx byte
      counts). Both remain fully available via `--json`. IPv6 in
      particular was a real, discussed tension -- dual-stack is a shipped,
      deliberate feature (`IPV6-001..003`), and never showing it anywhere
      risks the feature becoming invisible by default permanently, not
      just hidden from casual users. A middle option (show only the
      viewer's own IPv6 once, since that's a single line regardless of
      peer count, while still dropping *peer* IPv6 from the table where
      the real per-row width cost lives) was raised and rejected in favor
      of the simpler full drop -- Erik's call, made knowingly rather than
      by default.
    - **`coordinator` -> `admin`, display string only.** `tetron admin
      <net> add/list` already used "admin" as the CLI command name for
      this exact concept (granting/listing the network key) while
      `tetron status`, error messages, and docs called it "coordinator" --
      an existing internal inconsistency, not a new term being
      introduced. Scoped narrowly: only `NetworkRole`'s `derive_more::
      Display` output (`#[display("coordinator")]` -> `#[display("admin")]`
      on the `Coordinator` variant) changed. The variant name itself,
      `is_coordinator`, `coordinator_count()`, and every spec requirement
      ID/prose referencing "coordinator" (`NUKE-CONSENSUS`,
      `STRANDED-COORDINATOR-WARN`, etc.) are unchanged -- same decoupling
      already used successfully this session for `resolve_network_short_id`'s
      internal `short` parameter staying put while user-facing labels moved.

    **Bundled in the same implementation pass**: `StatusResponse.
    pending_networks` removed as dead code (found while surveying
    available-but-unshown fields for this redesign, unrelated to it
    otherwise) -- its own doc comment claimed to reflect `AppConfig.
    pending_joins`, which `LIVE-001` removed entirely; the one
    construction site (`diagnostics.rs`) always built it as `Vec::new()`
    with a comment already admitting as much; zero consumers in either
    text or `--json` output. Exact same shape as `NetworkStatus.
    pending_requests`, already found and removed under `LIVE-001`'s own
    addendum -- this was that fix's twin, missed by the same cleanup
    pass. Bundled here rather than as a separate change since it lives on
    the exact `StatusResponse` struct this redesign already edits.

    Wire changes: `PeerStatus.is_coordinator: bool` (new, `#[serde(default)]`),
    `NetworkStatus.subnet: String` (new, `#[serde(default)]`),
    `StatusResponse.pending_networks` (removed). All three are
    `#[serde(default)]`-compatible or outright removed, so an old daemon's
    response still decodes against a new CLI (missing fields default;
    the removed field is simply never read, whether or not an old daemon
    still sends it).
    """
    req_id = "STATUS-002"


class SubnetDriftOnRestart(Requirement):
    """REQUIREMENT-ID: SUBNET-DRIFT-001

    Found live-testing `STATUS-002` on real hardware (2026-07-20): exposing
    a network's subnet in `tetron status` for the first time immediately
    surfaced that a real, long-running test network's two peers disagreed
    about their own shared network's subnet, and about each other's IP.
    Confirmed as an actual data-plane break, not cosmetic: the coordinator's
    real TUN device (`ip addr`) was on a completely different subnet than
    either peer's roster-recorded IP, and `ping` between them showed 100%
    loss both ways -- despite `tetron status` (both before and after
    `STATUS-002`) showing "direct" connectivity with real, non-zero byte
    counters, because that traffic was control-channel/QUIC-transport
    chatter, not application-level TUN-forwarded packets, which don't
    exercise the same code path at all.

    **Root cause, two independent bugs, one shared design flaw.** Both
    `NetworkConfig.subnet` (local per-network config) and `GroupBlob.subnet`
    (the signed, network-wide DHT record every peer trusts) used the same
    convention: `None` means "the compiled `default_subnet()`," kept so a
    default-subnet network's config/blob stays byte-identical
    (`MULTISEG-001`). This is lossy the moment a node's own subnet
    preference can differ from the compiled default *and* can drift
    independently over time (true since `MULTISEG-002..007` let each
    network keep an independent subnet) -- `None` can no longer distinguish
    "this network genuinely wants the compiled default" from "this network
    wants whatever the node's default happened to be back when it was
    created."

    1. **Coordinator restart** (`restore_coordinator_network`,
       `src/daemon/mesh/runtime.rs`): resolved a restored network's subnet as
       `net_config.subnet.unwrap_or_else(default_subnet)` -- falling back to
       the compiled constant whenever the local config's `subnet` was
       `None`, without ever consulting the node's actual current subnet
       setting, let alone the network's *original* one. A network created
       while the node's default was e.g. `10.77.0.0/24` (so `subnet: None`
       was correctly persisted at the time, matching what was then the
       node's default) gets silently repinned to the compiled
       `10.88.0.0/24` on every subsequent restart.
    2. **Member restart when the DHT/blob is transiently unreachable**
       (`fallback_blob_from_config`, `src/daemon/mesh/create_join.rs`):
       synthesized a fallback blob with `subnet: Some(config::node_subnet())`,
       reasoning (per its own comment) that this was "safe per the
       SUBNET-BUG-001 invariant: an already-joined member's node subnet
       already matches its network's." That invariant held only in the
       pre-multi-segment world where a node ran one shared TUN/subnet for
       everything; `AGENTS.md` itself documents that `SUBNET-010`'s
       node-wide coherence check was removed once each network could keep
       its own subnet. Worse, both `create_network_inner` and
       `finalize_join` *mutate* the node's global default subnet as a side
       effect of every create/join (`config::set_node_subnet`), so the
       invariant breaks the moment a node's *second* network uses a
       different subnet -- every previously-joined network relying on this
       fallback silently inherits whatever unrelated network was created or
       joined most recently.

    **Compounding, not just repeating:** `NetworkState.subnet`'s only path
    into the signed blob is `blob_subnet()` (`src/daemon/mod.rs`), which had
    the identical "`None` for the compiled default" collapse. So bug 1
    firing on a coordinator doesn't just corrupt that coordinator's own
    local state -- `seal_and_publish` immediately afterward republishes the
    now-wrong subnet into the canonical blob (as `None`, since the
    in-memory value now equals the compiled default), spreading the
    corruption to every peer that fetches the blob fresh afterward,
    independent of whether they hit bug 2 themselves.

    **Fix, three parts, per due-diligence discussion with Erik (chose
    "always persist explicitly" over an IP-address-inference self-heal,
    which would have to assume a prefix length the project's own history
    doesn't guarantee -- default_subnet() was `/16` before an earlier
    project-wide change to `/24`):**

    1. **Stop omitting the value, everywhere.** `blob_subnet()` now always
       returns `Some(self.subnet)`. `create_network_inner`'s config save,
       `restore_coordinator_network`'s config save, and `persist_join_config`
       (`src/daemon/mesh/join.rs`, which previously hardcoded `subnet: None`
       unconditionally on every fresh join) all persist the actual resolved
       subnet explicitly now, never conditionally collapsed. `fallback_blob_from_config`
       reads the network's own persisted `nc.subnet` instead of the
       unrelated node-wide `config::node_subnet()`. Removes the ambiguity at
       the source for anything created/joined/restored under this fix;
       self-heals a legacy `None` the first time it successfully restores,
       since the now-validated, correctly-resolved value gets persisted
       back explicitly.
    2. **Hard-fail instead of silently drifting, as a safety net independent
       of (1).** New `membership::validate_subnet_matches_roster(subnet,
       roster, self_identity)`: checks the resolved subnet against this
       identity's own already-signed roster IP (still reliably correct even
       when the top-level subnet cache has drifted, since member IPs are
       always persisted as absolute values, never conditionally omitted).
       A no-op if the identity isn't in the roster yet (a fresh join, not a
       restore -- nothing to check). Called in `restore_coordinator_network`
       (before `seal_and_publish`, so a bad resolution is never written back
       anywhere) and in `join_network_inner`'s restore/reconnect path, both
       returning a clear error naming both values instead of proceeding to
       attach a TUN that cannot route to any peer. Unit-tested directly
       (`validate_subnet_matches_roster_{ok_when_consistent,
       rejects_mismatch, ok_when_identity_absent}`, `src/membership.rs`).
    3. **Self-heal via IP inference: considered, not implemented.** Would
       need to assume a prefix length to back out a subnet from an existing
       roster IP alone, which isn't guaranteed sound given the project's own
       history (the compiled default itself changed prefix length once).
       (2) converts an already-corrupted network from "silently broken" to
       "loudly refuses to restore, names the inconsistency" -- sufficient
       without guessing. The one specific network found broken live-testing
       this is disposable test infrastructure; recreating it fresh (not a
       code change) is the pragmatic remediation for that specific instance.

    **Not yet re-verified live** after this fix (the bug was found via, but
    fixed after, the `STATUS-002` live-testing session) -- `cargo build`/
    `clippy`/`test` (220 tests, +3 new) and `reconcile.py` green. Redeploying
    to the same real hardware to confirm (2) actually catches the existing
    broken network and that a clean network's restart round-trips its
    subnet correctly is the natural next step.

    **Addendum, live-tested 2026-07-20:** verified end-to-end on real
    hardware (aorus, xps) exactly as planned above. Removed both machines'
    broken `systray-func-test` config (found along the way: `tetron leave`
    can't remove a network that failed to restore -- both its resolution
    paths only scan currently-*loaded* networks, never the full persisted
    config list; logged as a separate, low-urgency gap in
    `DO-NOT-COMMIT/TODO.md`, not fixed here), recreated it fresh, and
    confirmed: subnet persists identically on both sides across a create +
    join + restart cycle, real data-plane traffic (`ping`, then 20MB `scp`
    with a SHA-256 check) works with 0% loss, and it survives a second
    restart on both machines with no drift. Also joined a fourth machine
    (a MacBook Pro, Apple Silicon/macOS -- built natively there, synced
    from this repo directly over SSH rather than GitHub) to the same
    network with identical results, confirming the fix holds across
    architectures and operating systems, not just Linux x86_64.
    """
    req_id = "SUBNET-DRIFT-001"


class EachNetworkGetsADistinctSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-UNIQUE-001

    Found immediately during the `SUBNET-DRIFT-001` live-test follow-up:
    creating a second network on a node that already had one, without an
    explicit `--subnet`, silently gave it the *exact same* subnet as the
    first (`create_network_inner`'s unspecified-subnet path just resolves
    to the node's one persisted/compiled default, with no awareness of what
    other networks that same node already has). Concretely: the same node
    ended up with the identical address (`10.77.0.200`) on two supposedly-
    independent networks -- harmless given per-network TUN isolation
    (`MULTISEG-002..007`), but defeating a real purpose of configurable
    subnets, confusing to read, and a foreseeable source of firewall/
    routing-rule mistakes for anyone trying to distinguish networks by IP
    range. Erik: "MUST be a new subnet, always."

    **Fix:** new `membership::next_available_subnet(candidate, existing)` --
    given a starting candidate and every subnet already in use, advances by
    one full block (`2^(32-prefix)` addresses) per collision, prefix length
    fixed, until it finds one that overlaps nothing in `existing` (capped at
    4096 attempts, far beyond any real node's network count, after which it
    gives up and returns the last candidate rather than looping forever).
    Wired into `create_network_inner` (`src/daemon/mesh/create_join.rs`):

    - **No explicit `--subnet`** (the common path): the resolved default
      candidate is silently advanced past any collision with an existing
      network's persisted subnet (`config::load()?.networks[].subnet` --
      always populated now thanks to `SUBNET-DRIFT-001`'s "persist
      explicitly, never omit" fix, so this list is reliable). "Silently" as
      far as the resolution logic goes, but never silent to the *caller*:
      `IpcMessage::Created` gained a `subnet: String` field (both
      `create_network_inner` and `restore_coordinator_network`'s success
      responses), and `tetron create`'s own CLI output now prints a
      `subnet <cidr>` line unconditionally -- the actual chosen value is
      always visible, whether or not it's the one a caller might have
      expected.
    - **Explicit `--subnet`**: honored exactly, never silently substituted
      -- but rejected outright with a clear error if it collides with a
      network this node already has. An explicit request deserves a
      correction, not a silent override to something else.

    Unit-tested directly (`next_available_subnet_returns_candidate_when_free`,
    `_advances_past_one_collision`, `_advances_past_several_collisions_in_order`,
    `_keeps_prefix_length`, `src/membership.rs`) -- the "several collisions
    in order" case exercises exactly the reported scenario (candidate
    already taken, verifies it lands on the correct next free block, not
    just *some* free block).

    **Bundled discovery while fixing this, unrelated to the feature
    itself:** several existing unit tests across `src/membership.rs`,
    `src/config.rs`, `src/control.rs`, `src/packet.rs`, `src/peers.rs`, and
    `src/forward.rs` used `100.64.x.x` (the pre-fork default subnet,
    inherited from upstream and never updated after this fork changed the
    default to `10.88.0.0/24`) as test-fixture data. Most were harmless --
    arbitrary placeholder IPs where the specific value never mattered to
    what was being tested -- but three were genuinely **vacuous**, passing
    for an unintended reason rather than testing what they claimed to:
    `test_derive_ip_avoids_reserved` compared derived IPs (always inside
    `10.88.0.0/24`) against the *wrong* subnet's reserved addresses, so the
    assertion was vacuously true for every input, testing nothing;
    `validate_member_rejects_mismatched_ip`, `validate_member_rejects_
    reserved_addresses`, `validate_approved_rejects_mismatched_ip`, and two
    `decode_group_blob_rejects_*` tests all used out-of-`10.88.0.0/24`
    addresses where an *in-range* one was needed to actually exercise the
    specific rule each test was named for (mismatch / reserved-address
    rejection), instead accidentally passing via the unrelated out-of-range
    check every time. Fixed all of these to use addresses actually inside
    `default_subnet()`; mechanically swapped the remaining, genuinely
    arbitrary occurrences to `10.88.x.x` for consistency (`sed`-scoped to
    each file's test module, verified by rerunning every affected module's
    tests before and after). Left untouched: doc comments/prose correctly
    citing the real Tailscale range, and the two tests in `membership.rs`
    that deliberately compare against `100.64.0.0/10` on purpose
    (`subnets_overlap_detects_both_directions_but_not_disjoint`,
    `ensure_in_range_respects_custom_subnet`) -- changing either of those
    would have broken the actual thing they're testing.
    """
    req_id = "SUBNET-UNIQUE-001"


class InviteListRevokedNotUsed(Requirement):
    """REQUIREMENT-ID: INVITE-STATUS-001

    Found live bug-hunting after `SUBNET-UNIQUE-001`: `tetron invite <net>
    revoke <id>` followed by `tetron invite <net> list` showed the just-
    revoked invite's status as `used` -- indistinguishable from one someone
    had actually redeemed. `InviteInfo.used` (`tetron-proto/src/ipc.rs`) was
    populated as `used: entry.revoked` (`src/daemon/mesh/invite_handler.rs`),
    with a comment claiming "revoked flag means consumed."

    That's not just misleadingly named -- `InviteEntry` has no field that
    could ever represent "actually redeemed" in the first place. An invite
    that's genuinely used is removed from the blob entirely on successful
    redemption (`src/daemon/mesh/accept.rs`'s "burn the invite" step), so
    it's never listed again at all once that happens. The only thing
    `InviteEntry.revoked` can ever mean, for any entry still present to
    list, is "an admin explicitly revoked this" -- calling it `used` claimed
    a distinction (redeemed vs. cancelled) the data model was never capable
    of drawing, and actively misled anyone auditing which invites were
    manually revoked vs. genuinely consumed by a joiner.

    **Fix:** renamed `InviteInfo.used` -> `revoked` (wire field), the
    daemon's construction site, `tetron invite list`'s `--json` key and text
    `status` column (now prints `revoked` instead of `used` for that case;
    `active`/`expired` unchanged), and `docs/HOWTO.md`'s `jq` example.
    """
    req_id = "INVITE-STATUS-001"


class StatusMemberCountExcludesAdmins(Requirement):
    """REQUIREMENT-ID: STATUS-003

    Found live on a real multi-admin network (USER's "shallows" network,
    2026-07-22): `tetron status`'s per-network header line showed `admins
    2/2   members 4/5`, but the peer table right below it listed only 4
    non-admin members total (one, `air`, offline) -- the "members" total
    should have read `3/4`, not `4/5`.

    **Root cause:** `print_network` (`src/cli/status.rs`, added by
    `STATUS-002`) computed the `members` column from *all* peers, admins
    included:

    ```rust
    let online = net.peers.iter().filter(|p| p.connection.is_some()).count();
    ...
    "members {online}/{}", net.peers.len()
    ```

    `net.peers` (the wire `PeerStatus` list) holds every peer regardless of
    role, so an admin peer (in the reported case, a co-coordinator with a
    live connection) was counted into both the numerator and denominator of
    "members" -- on top of already being counted in `admins` just to its
    left. Both header numbers were inflated by exactly one for each online
    admin peer; a network with only one admin (self, never in `net.peers`)
    would never have shown the bug, which is why `STATUS-002`'s own
    live-testing pass didn't catch it.

    **Fix:** `online` and the denominator both filter to `!p.is_coordinator`
    before counting, matching the `admins_online`/`admins_total` pair's own
    care to count each role exactly once. `--json` output was never
    affected -- `PeerStatus.is_coordinator` and `connection` were already
    correct per-peer; only the derived text-mode aggregate was wrong.
    """
    req_id = "STATUS-003"


class AdminAddHostnameResolutionCaseInsensitive(Requirement):
    """REQUIREMENT-ID: STATUS-004

    Found live immediately after `STATUS-003`, same "shallows" network:
    `tetron admin shallows add erikk-ThinkPad-P1` failed with `could not
    resolve peer 'erikk-ThinkPad-P1'`, even though that exact host was
    listed in `tetron status` moments earlier -- as `erikk-thinkpad-p1`.

    **Root cause:** every hostname a member can ever have is lowercased at
    creation (`hostname::sanitize_hostname`, called from `generate_hostname`
    and any explicit `--hostname`) -- OS hostnames especially are routinely
    mixed-case (`erikk-ThinkPad-P1` was this host's actual OS hostname), so
    a user recalling or retyping it from memory has every reason to type it
    back with its original casing. `MeshManager::resolve_peer_name`
    (`src/daemon/mesh/runtime.rs`) compared with a case-sensitive `==`,
    so the mismatch was silently a no-match rather than a resolvable typo.

    **Why this is safe, not just convenient:** because every stored
    hostname is already guaranteed lowercase, two roster entries can never
    differ *only* by case -- there is no real hostname a case-insensitive
    match could confuse for another. Loosening the comparison forgives
    exactly one thing: a user's own capitalization habits, never a
    genuinely ambiguous choice between two peers.

    **Fix:** `resolve_peer_name`'s hostname branch now compares with
    `str::eq_ignore_ascii_case` instead of `==`. Scoped narrowly to this one
    resolver -- `resolve_short_id_any_network` (short id / endpoint id
    prefix matching, used by `kick`/`nuke --second`) is unaffected and
    correctly stays exact: those are cryptographic identifiers a user is
    expected to copy from `tetron status` output verbatim, not recall from
    memory, and hex ids carry no meaningful capitalization ambiguity to
    begin with.
    """
    req_id = "STATUS-004"

