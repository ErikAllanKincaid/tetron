from libspec import Requirement, Constraint, UserStory

# --------------------------------------------------------------------------
# Requirements: rebrand rayfish -> torpedo (RENAME-*)
# --------------------------------------------------------------------------

class BinaryRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-001

    The `ray` binary is renamed `torpedo` (Cargo.toml [[bin]], build output,
    contrib/rayfish.service's ExecStart path).
    """
    req_id = "RENAME-001"


# class ServiceRenamed(Requirement):
#     """REQUIREMENT-ID: RENAME-002

#     systemd service, unit file, and all systemctl invocations referring to
#     "rayfish" are renamed to "torpedo" (src/cli/service.rs, src/cli/update.rs,
#     src/update.rs, contrib/rayfish.service renamed to contrib/torpedo.service).
#     """
#     req_id = "RENAME-002"


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


class GitShaEnvVarRenamed(Requirement):
    """REQUIREMENT-ID: RENAME-018

    The env-var name that carries the git short SHA from `build.rs` into the
    Rust binary is renamed from `RAY_GIT_SHA` (a pre-fork upstream identifier)
    to `TETRON_GIT_SHA`. Two sites:

    1. `build.rs:20` -- `cargo:rustc-env=RAY_GIT_SHA={sha}` becomes
       `cargo:rustc-env=TETRON_GIT_SHA={sha}`.
    2. `src/cli/service.rs:257` -- `env!("RAY_GIT_SHA")` becomes
       `env!("TETRON_GIT_SHA")`.

    No wire format, on-disk storage, or external contract depends on the env
    var's name -- only its value (the SHA string). Not covered by CON-007's
    curated-token scan (which only catches lower-case `rayfish`/`ray ` host
    artifacts, not ALL_CAPS env-var names) -- pure manual cleanup.
    """
    req_id = "RENAME-018"


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


# --------------------------------------------------------------------------
# Requirements: cross-distro portability (PORTABILITY-*)
#
# Three ideas logged 2026-07-27, consolidated and dependency-ordered in
# DO-NOT-COMMIT/PLAN_CrossDistroPortability.md (human-language plan, not
# yet a Constraint gate for any of these -- each is structural/behavioral,
# verified by the existing build/clippy/test gates in reconcile.py, same
# reasoning CLI-VOCAB-006/STATUS-005 already used).
# --------------------------------------------------------------------------

class NonSystemdDetection(Requirement):
    """REQUIREMENT-ID: PORTABILITY-002

    `install`/`restart`/`start`/`stop`/`uninstall` (`src/cli/service.rs`)
    all shell out to `systemctl` on Linux unconditionally -- no detection
    of whether it is even present. On a non-systemd Linux system (Alpine/
    OpenRC, Void/runit, Devuan/Artix/sysvinit, Gentoo with OpenRC, ...)
    every one of those commands failed with a raw "systemctl: command not
    found" instead of a clear explanation of what to do instead.

    **The daemon itself has no systemd dependency at all** -- `tetron
    daemon` (the hidden foreground-run subcommand) already runs under any
    supervisor with zero code change needed. Only the convenience
    service-management commands are systemd-only, so the fix is detection
    plus a clear message, not new service-management code.

    **Fix:** `systemd_available()` checks `/run/systemd/system` -- the
    canonical "is systemd actually running as PID 1" test, not just
    whether a `systemctl` binary happens to exist on `PATH` (some minimal/
    container environments stub or partially install one without systemd
    genuinely running). `require_systemd()` calls this and exits with a
    clear message pointing at the documented fallback (`sudo tetron
    daemon`, plus a new `contrib/tetron.openrc` reference unit) if it's
    false. Wired into all five service-management entry points
    (`cmd_install`, `cmd_restart`, `cmd_stop`, `cmd_start`,
    `cmd_uninstall_service`) rather than a single shared call site, since
    each is independently reachable and none of them funnel through one
    common function on Linux.

    Documented in `README.md`'s new "Non-systemd Linux" section. First-
    class OpenRC/runit/s6 service management (parallel install/start/
    stop/restart/uninstall code paths, the same shape macOS's launchd
    branch already has) is explicitly out of scope here -- this is the
    detect-and-message fix, not that bigger version.
    """
    req_id = "PORTABILITY-002"


class ConfigurableInstallDirectories(Requirement):
    """REQUIREMENT-ID: PORTABILITY-003

    Checked the actual code before touching anything: `config::config_dir()`
    already had a `TETRON_CONFIG_DIR` override, but it was deliberately
    gated to `cfg(any(target_os = "android", test))` only -- the doc
    comment said so explicitly, production Linux/macOS installs never
    checked it. `logdir::log_dir()` and `tetron-proto::ipc::socket_path()`
    had no override mechanism at all, not even the restricted one
    `config_dir` had.

    **Fix:** all three now accept an env var override on every platform,
    same override-then-fixed-defaults shape:
    - `config::config_dir()`: `TETRON_CONFIG_DIR`'s gate widened to every
      build. An install that never sets it resolves the exact same path as
      before this existed -- purely additive, not a behavior change for
      anyone not using the override.
    - `logdir::log_dir()`: new `TETRON_LOG_DIR`.
    - `tetron_proto::ipc::socket_path()`: new `TETRON_SOCKET_PATH`. Both
      the daemon's own listener (`daemon/mesh/bootstrap.rs::serve_ipc`)
      and every client call `socket_path()` directly rather than
      duplicating the path -- confirmed by grep, not assumed -- so one
      override point covers the daemon and every client, including
      `tetron-webui`/`tetron-systray` in their own separate repos, with no
      changes needed on their side.

    **Motivating case, not just a general preference:** a direct
    prerequisite for the Nix/NixOS compatibility question logged the same
    day (see `PLAN_CrossDistroPortability.md`) -- Nix's store-based layout
    doesn't follow the FHS paths tetron's install assumed unconditionally
    before this. Also matches the standing project preference
    (`feedback_configurable_with_sensible_defaults` in auto-memory): a real
    trade-off, configurable with a sensible default, not a single hardcoded
    value.

    Verified by the existing `build`/`clippy`/`test` gates (a `Requirement`,
    not a `Constraint` -- structural/behavioral, no curated-token gate
    needed, same reasoning `CLI-VOCAB-006`/`STATUS-005` already used), plus
    new unit tests for `log_dir`/`socket_path`'s override behavior
    specifically, since neither had any prior test coverage for this at
    all (`config_dir`'s override was already exercised under every test
    build regardless of this change, so no new test was needed there).
    """
    req_id = "PORTABILITY-003"


class MuslReleaseTargets(Requirement):
    """REQUIREMENT-ID: PORTABILITY-001

    Checked the actual release pipeline before touching anything: every
    published Linux binary was `*-unknown-linux-gnu` only
    (`.github/workflows/release.yml`/`nightly.yml`'s build matrix). No
    musl target existed anywhere.

    **Why it matters, not just "more targets":** a fully static musl
    binary has no dynamic libc dependency at all, so it runs unchanged
    across host glibc versions -- the exact class of bug
    `tetron-testsuite` already hit once (`lib/topology.sh`'s own comment:
    `GLIBC_2.39 not found` against an older-glibc VM box, which forced
    switching the default box to `bento/ubuntu-24.04`). It is also the
    real fix for genuine Alpine support, which ships musl only, not
    glibc.

    **First verification pass only checked `--version`/`--help` -- that
    was not enough, and saying so plainly rather than letting the earlier
    claim stand.** `cross build --release --target
    x86_64-unknown-linux-musl` compiled cleanly (every dependency in the
    tree builds under musl with no source changes) and the result was
    confirmed genuinely static (`ldd`: "statically linked"). But
    `--version`/`--help` never exercise the daemon's real networking
    code path, and a second pass -- installing and actually running the
    binary on a real Rocky Linux 9 VM as part of verifying `D1` (see
    `tetron-testsuite`'s own TODO) -- found the daemon panicked and
    coredumped within seconds of starting:

    ```
    thread 'tokio-rt-worker' panicked at .../noq-udp-1.1.0/src/cmsg/mod.rs:81:5:
    assertion failed: align_of::<T>() <= align_of::<C>()
    ```

    **Root cause, confirmed not guessed:** `noq-udp` (a transitive
    dependency via `iroh` -> `netwatch`/`noq`, handling QUIC's UDP
    control-message/ECN plumbing) asserted `align_of::<T>() <=
    align_of::<C>()` (`C` = `libc::cmsghdr`) before using `ptr::read`/
    `ptr::write`. glibc's `cmsghdr` uses an 8-byte `size_t cmsg_len` on
    x86_64; musl's uses a 4-byte `socklen_t cmsg_len` -- a real,
    documented ABI difference (the same class of issue as a known
    `quinn-udp` musl report), not a bug in `noq-udp`'s overall design. A
    perfectly valid payload like `libc::timespec` (8-byte aligned) fails
    that assertion under musl even though nothing is actually unsound
    about reading it.

    **Fixed with a local patch, vendored in-tree:**
    `vendor/noq-udp-1.1.0/` (a copy of the crate with `decode`/`push`
    switched from `ptr::read`/`ptr::write` to `ptr::read_unaligned`/
    `ptr::write_unaligned`, dropping the now-unnecessary alignment
    assertion), wired in via `[patch.crates-io]` in the workspace root
    `Cargo.toml`. See `vendor/noq-udp-1.1.0/PATCH.md` for the exact
    diff and rationale. Not yet reported upstream to
    `github.com/n0-computer/noq`.

    **Re-verified end to end after the patch, on a fresh Rocky Linux 9
    VM:** `tetron install` (systemd unit installs, daemon starts),
    `tetron create` (TUN device created, network published), `tetron
    status --json` (healthy, `active: true`) -- no crash, no coredump.
    Previously coredumped (`code=dumped, status=6/ABRT`) at the same
    point with the unpatched crate. This is the actual bar this
    requirement is held to now: a musl binary that runs the real daemon,
    not one that only answers `--version`.

    **Also found live in the same verification pass, worth recording
    even though it's not this requirement's fix to make:** the
    glibc-linked binary this project already ships could not run on
    Rocky Linux 9 at all (`GLIBC_2.39 not found`) -- direct, live proof
    of the exact problem this requirement exists to solve, using a
    locally-built binary (this dev machine's own glibc 2.39 toolchain,
    not the actual CI-published release binary, which targets an older
    `ubuntu-22.04`/glibc-2.35 baseline -- whether *that* specific
    combination also fails against Rocky 9's glibc 2.34 remains a
    separate, still-open question, not conflated with this finding).

    **aarch64-unknown-linux-musl was attempted the same way and is not
    equivalently confirmed.** It failed in this local environment with a
    glibc-version mismatch inside `cross`'s own Docker image, affecting
    host-side build-script binaries (`serde`/`libc`/`parking_lot_core`'s
    build scripts, not tetron's own code) -- a tooling issue in the local
    Docker-based cross-compilation path, not a musl-compatibility problem
    in tetron's dependency tree. The real CI pipeline builds aarch64 on a
    **native** ARM runner already (the existing `aarch64-unknown-linux-
    gnu` entry uses `runner: ubuntu-22.04-arm`, not `cross`/Docker at
    all), which this specific local hiccup would not obviously reproduce
    on -- but that combination (native-runner aarch64-musl) remains
    unconfirmed, stated plainly rather than assumed fine by extension
    from the x86_64 result.

    **Added to both `release.yml` and `nightly.yml`'s matrix**
    (`x86_64-unknown-linux-musl`/`aarch64-unknown-linux-musl`, same
    runners as the matching gnu entries), with a new conditional
    `musl-tools` install step (`if: contains(matrix.target, 'musl')`) --
    the standard `musl-gcc` linker Rust's musl target needs, not present
    on GitHub's default Ubuntu runner images. `ci.yml` (the PR-gating
    workflow) was deliberately left untouched -- it has no existing
    per-target build matrix to extend the same way, unlike
    release.yml/nightly.yml, and adding one is a separate, bigger change
    than this requirement's scope.

    **Addendum, 2026-07-30: a second local patch was added to the same
    vendored crate on an initial (wrong) diagnosis, then correctly
    demoted once the real cause was found -- recorded in full so the
    mistake isn't silently lost.** A live Android embedder run (in the
    separate `tetron-mobile` repo, not this one) crashed at daemon
    startup, before any network activity: `failed to bind iroh endpoint`
    -> `Failed to bind sockets` -> `Operation not permitted (os error
    1)`. First hypothesis: `UdpSocketState::new`'s Linux/Android block
    sets `IP_MTU_DISCOVER`/`IPV6_MTU_DISCOVER` to `IP_PMTUDISC_PROBE`,
    and `man 7 ip` was misread as requiring `CAP_NET_ADMIN` for it,
    which a sandboxed Android app never holds. `set_socket_option_supported`
    (the helper these calls go through) was patched to tolerate `EPERM`
    the same way it already tolerated `ENOPROTOOPT`/`EOPNOTSUPP`.

    **That patch did not fix the crash, and the diagnosis was wrong.**
    Rebuilding and redeploying with the patch confirmed present in the
    shipped `.so` reproduced the identical error. Re-checking `man 7 ip`
    properly: `CAP_NET_ADMIN` is required for `IP_TRANSPARENT` and high
    `IP_TOS` priorities, never for `IP_MTU_DISCOVER` -- confirmed
    empirically too, the same `setsockopt` call succeeds unprivileged on
    an ordinary Linux host. The `EPERM` arm was kept anyway as harmless
    defensive hardening (an option only used to suppress fragmentation
    should never be fatal on any errno), but is explicitly not credited
    with fixing anything.

    **Real root cause, found with a precise diagnostic instead of more
    guessing:** a temporary probe (since removed) walked the exact
    `socket()`/`bind()`/`setsockopt()` sequence `netwatch::udp::
    SocketState::bind` performs and folded a full report into the bind
    error itself (an embedder this early in startup may have no tracing
    sink wired up, so logging the report was unreliable -- folding it
    into the already-working on-screen error display was not). One
    redeploy showed `socket()` itself failing with `EPERM`, for both
    IPv4 and IPv6, before any `bind()` or `setsockopt()` was even
    reached -- matching Android's `cgroupsock/inet_create` eBPF hook,
    which denies `socket()` outright for an app UID lacking the
    `INTERNET` permission in the kernel's own UID permission map (a
    separate, later-synced thing from the manifest simply declaring the
    permission).

    **Resolution: not a code bug at all.** The test app's UID had been
    reinstalled (`adb install -r`) many times across the debugging
    session, starting from an early build with no `INTERNET` permission
    declared. Android's netd permission bitmap is keyed by UID and a
    plain `-r` reinstall keeps the same UID, so the kernel-level map was
    never resynced after the permission was added in a later build. A
    full `adb uninstall` (fresh UID) plus clean install fixed it
    immediately, with zero code changes -- the endpoint bound and the
    daemon started successfully (`tetron node started`, entitlement
    check ran). This was stale local test-device state produced by the
    test methodology itself, not a bug in `tetron`, `iroh`, `netwatch`,
    or this vendored crate, and would not recur on a real first-time
    install of a properly-signed release build.

    **Separately, a real, unrelated correction landed on the same
    vendored copy in the same pass:** its declared version (`1.1.0`) had
    drifted behind genuine upstream's actual latest release (`1.1.1`),
    which silently stops Cargo's `[patch]` from ever being selected once
    anything re-resolves the dependency graph fresh (Cargo prefers the
    newest semver-compatible version and only consults `[patch]` for
    whichever version it actually wants) -- confirmed live, a fresh
    resolve from a separate consuming crate silently used plain
    unpatched `1.1.1` from crates.io with no warning either patch was
    missing. Diffed this vendor copy against the real `1.1.1` tarball
    before relabeling (rather than just bumping the version string) and
    found one genuine content difference beyond this file's own two
    patches: real `1.1.1` re-disables `SO_TIMESTAMPNS`
    (`https://github.com/n0-computer/noq/issues/774`), a bug fix this
    vendor copy predates and did not have. Adopted that fix and bumped
    the version to match, re-verified via `diff -rq` against the genuine
    crates.io `1.1.1` source that only this file's own two documented
    patches remain as differences.

    See `vendor/noq-udp-1.1.0/PATCH.md` for the full account of both the
    wrong turn and the eventual resolution. `cargo -q check`/`clippy`
    clean throughout; the real on-device fix was verified live in the
    separate `tetron-mobile` repo, not here.
    """
    req_id = "PORTABILITY-001"


class InstallPathOverrides(Requirement):
    """REQUIREMENT-ID: PORTABILITY-004

    The three existing environment-variable path overrides
    (`TETRON_CONFIG_DIR`, `TETRON_LOG_DIR`, `TETRON_SOCKET_PATH`,
    PORTABILITY-003) are read at daemon startup by
    `config::config_dir()`/`logdir::log_dir()`/`socket_path()`, but the
    systemd unit and launchd plist have no `Environment=` / `EnvironmentVariables`
    lines for them — so a user who wants nonstandard paths must either set
    them globally (systemd manager environment, `/etc/environment`, etc.) or
    fork the unit file by hand.

    **Fix:** `tetron install` gains three optional flags:
    `--config-dir`, `--log-dir`, `--socket-path`. When provided, the
    service unit/plist gets the corresponding `Environment=` /
    `EnvironmentVariables` entry injected (same pattern as
    tetron-webui's `TETRON_WEBUI_PORT` / `install --port`). The user who
    passes none of them gets the exact same unit as before — no default
    changes, purely additive.

    The env vars themselves remain the single source of truth at runtime:
    this just wires the service unit to pass them through. A user who
    sets the env vars globally (or runs `tetron daemon` directly under a
    non-systemd supervisor) is unaffected and never needs the flags.

    Motivating case: a systemd/Linux or launchd/macOS user who wants to
    relocate tetron's config/log/socket paths without writing a custom
    service unit. Previously the only option was to set the env var in
    the systemd manager environment (`systemctl set-environment
    TETRON_CONFIG_DIR=...`) or edit the unit file — both more friction
    than `sudo tetron install --config-dir /custom/path`.

    ENFORCEMENT: structural (a `Requirement`, not a `Constraint` — no
    curated-token gate needed, same reasoning as PORTABILITY-003).
    Verified by existing build/test gates and the new e2e test.
    """
    req_id = "PORTABILITY-004"
