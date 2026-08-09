from libspec import Requirement, Constraint, UserStory

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

    The touched-file list (`reconcile.py`'s `check_hardcoded_cgnat`) must
    track wherever the subnet math actually lives, not just its original
    location. Found stale 2026-08-06: `src/membership.rs`/`tun.rs`/`dns.rs`
    were listed, but MODULARIZE-001 (2026-08-05) had already moved the real
    `Subnet`/CIDR/derivation logic to `src/addressing.rs`, which was never
    added -- a hardcoded-literal regression there would have passed this
    gate clean. Fixed by adding `src/addressing.rs` to the list.

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


class CargoAuditClean(Constraint):
    """CONSTRAINT-ID: CON-013

    D-02 (security-audit follow-up, 2026-07-21): `cargo audit` scans the
    full dependency tree (including feature-gated deps, e.g. `torut` behind
    the optional `tor` feature) against the RustSec advisory database.
    Accepted, already-reviewed advisories are ignored via a dated,
    documented entry in `.cargo/audit.toml` -- currently `RUSTSEC-2024-0344`
    and `RUSTSEC-2022-0093` (old `curve25519-dalek`/`ed25519-dalek` pulled
    in transitively by `torut`/`iroh-tor-transport`; `torut` 0.2.1 is the
    latest version on crates.io as of 2026-07-23 and hasn't modernized that
    pin, so this isn't fixable from tetron's side -- revisit if it ever is).
    This constraint fails only on a genuinely new, not-yet-reviewed
    advisory, never on the tree being imperfect in general; it also fails
    distinctly (not silently as "0 vulnerabilities") if `cargo-audit` itself
    isn't installed, so a missing tool can't be mistaken for a clean scan.

    ENFORCEMENT (reconcile.py): cargo_audit.installed is true and
    cargo_audit.count equals 0.
    """
    constraint_id = "CON-013"
    enforcement_logic = "{{ cargo_audit.installed and cargo_audit.count == 0 }}"


# ---------------------------------------------------------------------------
# SUNSET CANDIDATE CLUSTER, tracked at DO-NOT-COMMIT/TODO_DETAILS.md
# #migration-era-tooling -- CON-007..012 below (contiguous) plus CON-M03
# (further down, marked at its own class) all detect leftover `rayfish`
# rename-residue tokens from the pitopi->rayfish->torpedo->tetron
# migration. Kept as of the 2026-08-08 audit (near-zero cost, no confirmed
# false negatives), but this whole cluster's reason to exist ends with the
# migration itself -- revisit for deletion once these have fired zero for
# several releases running. CON-001 (above) and CON-M04 (further down) are
# NOT part of this cluster -- they guard live features/invariants, not
# rename residue, and should stay permanently.
# ---------------------------------------------------------------------------
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


class NoDeadCiWorkflowReferences(Constraint):
    """CONSTRAINT-ID: CON-014

    Added 2026-08-08 as a direct result of the 2026-08-08 `reconcile.py`
    16-check audit — none of the existing checks could have caught the dead
    `android:` job (`.github/workflows/release.yml`/`nightly.yml`, referencing
    the removed `-p ray-mobile` workspace member and `android/app/src/main/
    java` directory, dead since MINIMAL-016) before it was found by hand
    during a dead-code sweep, two dedicated sweeps and a tagged release later
    (see CI-002's 2026-08-07 addendum). This is the reusable, cheap,
    scriptable half of the audit's angle-2 findings — unlike the cross-repo
    dead-pub-surface check (too slow/heavy for a per-commit gate, left as a
    manual `docs/tetron-workflow.md` §12 step), this one only ever greps
    `.github/workflows/*.yml` `run:` blocks for `-p`/`--package <crate>` and
    `cd <path>` references, then confirms each resolves against `cargo
    metadata`'s real workspace members / an existing filesystem path.

    ENFORCEMENT (reconcile.py): ci_workflow_references.unexpected_count
    equals 0.
    """
    constraint_id = "CON-014"
    enforcement_logic = "{{ ci_workflow_references.unexpected_count == 0 }}"


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

    Also bans `config` (D-01, `apply.rs`'s sole consumer, removed by
    MINIMAL-011) and `chacha20poly1305`/`argon2` (found 2026-07-23 while
    reading the unscoped "coordinator key security" idea in
    `DO-NOT-COMMIT/TODO.md`: both listed in `Cargo.toml` but never used
    anywhere in `src/`/`tetron-proto/src/` -- confirmed via `cargo tree -i`,
    each resolves only through tetron's own direct dependency, nothing
    transitive needs them either. Almost certainly dead scaffolding for the
    encryption-at-rest direction floated in that same TODO section, never
    wired up. Removing them doesn't foreclose that idea -- re-add if it's
    ever actually built).

    Also bans `serde_yml`, `qr2term`, and `async-trait` (removed by
    TREE-SHAKE-001, merged 2026-08-05 -- confirmed gone from `Cargo.toml`,
    but the banned-dep list here was never updated for them, so an
    accidental re-add of any of the three would have passed this gate
    clean; found and fixed 2026-08-06 during the same audit as CON-002).

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


# Part of the migration-era sunset cluster marked above NoResidualHostIdentityLeak
# (DO-NOT-COMMIT/TODO_DETAILS.md #migration-era-tooling) -- same
# rayfish-rename-residue rationale as CON-007..012.
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
