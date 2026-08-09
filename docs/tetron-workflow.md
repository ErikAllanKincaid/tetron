# tetron development workflow

> Referenced from `AGENTS.md`, which has the short version under
> "Spec-first workflow (libspec + reconcile.py)". This file is the fuller,
> step-by-step reference — read it before starting a non-trivial change,
> not just when something goes wrong.

## Checklist

1. Branch.
2. Spec first — define/amend requirement(s) under `spec/`.
3. Work out dependency ordering between requirements, if more than one.
4. `uv run libspec diff` — mandatory before writing any code.
5. TDD — write tests first, per requirement.
6. Implement, in dependency order — make the tests pass.
7. `python3 reconcile.py` green.
8. `tetron-testsuite` regression pass (core changes only).
9. Commit — one requirement at a time, conventional subject, no
   authorship trailer.
10. Docs — `AGENTS.md`/`docs/CLI_REFERENCE.md`/`docs/ARCHITECTURE.md`/
    `README.md`/`CHANGELOG.md`, whichever actually changed.
11. Cross-repo follow-up, if any — separate, later work; do not bundle
    into the same branch.

## 1. Branch

All work happens on a feature branch — never directly on `main`. Name it
`<type>/<slug>`, matching this repo's conventional-commit types
(`feat`/`fix`/`docs`/`chore`/…) and its existing branch history (e.g.
`feat/portability-005-android-active-gate`). The slug should name the
requirement(s) the branch carries, not the ticket/conversation that
prompted it.

## 2. Spec first

Every change is driven by a requirement (or several) under `spec/` —
modular Python, one requirement per class, decomposed into granular,
single-responsibility pieces rather than one monolithic block (e.g.
`PATHBLEED-STATUS-001`/`-002` are two separate classes, not one). The
requirement ID is the first line of the class's docstring. Pick the
module by what code the requirement actually touches, not by an abstract
theme — existing requirements are grouped by the function/subsystem they
modify (`PATHBLEED-STATUS-*` sits in `security.py` because that's where
`choose_path_index` already lived, not because path selection is
inherently a security topic).

Before writing a new requirement, check whether an existing one already
covers — or explicitly forbids — the same territory. This fork has a
"KEEP-ON-PURPOSE" list and a set of `MINIMAL-*` removal requirements
(things deliberately stripped during the original minimalism pass); a new
requirement that quietly reintroduces what one of those removed needs to
say so explicitly and explain why this is a different, in-scope case
(see `MTU-DIAG-001`'s own docstring for the pattern: passive surfacing of
already-computed state is in scope even though `MINIMAL-006` removed
active probing commands).

## 3. Dependency-ordering constraints

When a change decomposes into more than one requirement, state the
ordering dependencies between them explicitly, in the docstrings
themselves — which ones must land before which others, and why (a later
requirement needing state/data a specific earlier one introduces is the
usual reason). Do this at spec-writing time, not implementation time:
the actual build/commit order should be derived from these stated
dependencies, not from the requirements' numeric order or the order they
happened to be written in. If two requirements have no dependency on each
other, say so too — it's the difference between "must be sequential" and
"can land in either order or in parallel."

## 4. `libspec diff`

`uv run libspec diff` (or the `libspec_diff` MCP tool) before writing any
implementation code. Review the diff for exactly the requirements
intended — nothing unrelated should show as changed.

## 5. TDD

Tests first, per requirement, before implementation exists. Existing
precedent: `PATHBLEED-STATUS-001`/`-002`'s unit tests
(`src/daemon/mod.rs`) were written alongside/ahead of the logic they
cover, colocated near the code under test rather than in a separate test
tree.

## 6. Implement

In the dependency order worked out in step 3. Make the tests from step 5
pass; nothing more.

## 7. `reconcile.py`

The fast, local, per-commit gate — sixteen checks, must exit `0` before
each commit. This is separate from, and prior to, step 8 — it is not a
substitute for testsuite coverage, and testsuite is not a substitute for
it.

## 8. testsuite

Any change to tetron core requires a `tetron-testsuite` pass to verify no
regression — the heavier, VM-based, cross-network check that `reconcile.py`
does not attempt. Not required for addon-only changes.

## 9. Commit

Conventional subject (`feat`/`fix`/`docs`/…), matching `git-cliff`'s
release-note rendering. **No authorship trailer of any kind** — the
commit author is already set by git config. One commit per requirement by
default, even when several were designed together in the same sitting
(`PATHBLEED-STATUS-001`/`-002` and `SUBNET-COLLISION-001`/`-002` both
shipped this way) — bundle only when a reviewer explicitly decides the
requirements are too entangled to review separately.

**`.github/PULL_REQUEST_TEMPLATE.md` is deliberately minimal, 2026-08-09.**
Only two sections: "What this does" (commit subjects + bodies — the
`Why` used to be a separate heading, cut because it was the same commit
content split across two headings for no reason) and "Manual
verification beyond CI" (live/hardware/testsuite checks CI can't do —
always states either what was done or that nothing beyond CI was).
Dropped entirely, not shrunk: the checklist ("title is conventional",
"cargo build/test/clippy pass", docs/CHANGELOG/ALPN reminders) —
`cargo build/test/clippy` duplicates `ci.yml`'s unfiltered
`pull_request` trigger (already enforced before anyone could check a
box), and the rest duplicates this step and step 10, which should catch
those *before* a PR exists, not re-ask at PR time. `contrib/pr-body.py`
fills both remaining sections in from the branch's own commits — paste
its output into the PR description field. GitHub PRs are opened through
the web UI in this workflow, not `gh`; the script only ever prints to
stdout, never invokes `gh`.

## 10. Docs

Update whichever of `AGENTS.md`, `docs/CLI_REFERENCE.md`,
`docs/ARCHITECTURE.md`, `README.md` actually changed as a result — not a
blanket touch-everything pass. `CHANGELOG.md`'s `## [Unreleased]` section
gets an entry in Keep-a-Changelog form (`Added`/`Changed`/`Fixed`/
`Performance`), written from the user's perspective; skip it for changes
with no user-visible effect.

## 11. Cross-repo follow-up

If a change touches a shared wire type (most commonly `tetron-proto`) or
otherwise affects `tetron-webui`/`tetron-systray`/`tetron-testsuite`/
`tetron-mobile`, note the required follow-up (typically a `Cargo.toml`
bump, sometimes UI work) in that repo's own `DO-NOT-COMMIT/TODO.md` —
never assume it auto-follows just because the change is additive. Per the
standing priority order (core, then addons, then integration), this
follow-up is separate, later work, not bundled into the branch that
changed core.

## 12. Cross-repo dead-code sweep

Not a per-commit step like `reconcile.py` — run this before cutting a
release, and after any feature removal, not on every branch. Written up
here (rather than folded into an existing step) because the same method
generalizes to any crate with a public library surface consumed by more
than one downstream repo, so it is worth preserving as a portable
procedure, not just a tetron-specific note.

**Why a normal build/lint pass cannot catch this class of dead code:**
rustc's `dead_code` lint only fires on private items. Anything `pub` in a
library crate's surface is exempt by design, because the compiler cannot
tell "reachable from a real external consumer" apart from "just never
cleaned up." tetron's own `src/lib.rs` is consumed three different ways —
`tetron-mobile` depends on the full `tetron` crate directly and calls into
it as a library; `tetron-webui`/`tetron-systray` depend only on the
separate `tetron-proto` crate; `src/main.rs`/`src/cli/` (the binary) call
into the lib crate for everything else. A `pub` item can be real (reachable
from any one of those three) or genuinely dead (reachable from none) — only
cross-repo grepping settles which, an in-repo-only check is not enough.
This is exactly the gap that let a fully-orphaned cluster in `src/dht.rs`
(the `_tetron_certgen` cert-floor record — `CERT_FLOOR_RECORD_NAME`,
`encode_cert_floor_record`/`decode_cert_floor_record`,
`publish_cert_floor`/`resolve_cert_floor`, plus their own tests) survive
two prior dedicated dead-code sweeps (`TREE-SHAKE-001..005`) and a tagged
release before being found — see
`DO-NOT-COMMIT/ANALYSIS_external-PR12-dht-leak-claim_2026-08-07.md` for the
discovery and `TODO_DETAILS.md#certfloor-dead-code-cleanup` for the
follow-up, the worked example this procedure is written from.

**Deliberately not automated in `reconcile.py`/CI.** This check is
addon-aware by nature — it has to grep `tetron-mobile`/`tetron-webui`/
`tetron-systray` to mean anything. `reconcile.py` and `ci.yml` are core's
own per-commit gate; they must not require sibling repos to exist on disk,
and core has no business depending on knowledge of its downstream
consumers' repo layout just to validate itself (the dependency direction
is the other way around — addons depend on core, not the reverse).
Scoped 2026-08-09: keep this manual, run-by-hand step, but automate its
mechanical half (steps 1-2 below) as a script in `contrib/` — the same
place this repo's other addon-aware tooling already lives
(`install-tetron-suite.sh`), never in the core gate.

**Method:**

1. Enumerate every top-level `pub fn`/`pub struct`/`pub enum`/`pub const`/
   `pub static`, plus `pub fn` methods inside inherent (non-trait) `impl`
   blocks, in the library crate(s) — for tetron, `src/*.rs` + `src/**/*.rs`
   (excluding `src/main.rs`/`src/cli/**`, which are the binary's own
   dispatch surface, not library API — but do check whether items *defined*
   in the lib and *used* by the binary have real callers there) plus
   `tetron-proto/src/**/*.rs`.
2. For each item, grep for usage — never the definition line itself, and
   never a `#[cfg(test)]` block referencing it (a test exercising dead code
   is not a real caller, it just proves the dead code still compiles) —
   across every consumer: the rest of this repo, and the full checkout of
   every downstream repo that depends on it (for tetron: `tetron-mobile`,
   `tetron-webui`, `tetron-systray`). Zero hits outside the item's own
   definition and its own tests marks it a dead-code candidate.

   **Steps 1-2 are automated:** `python3
   contrib/cross-repo-dead-code-sweep.py` (run from `~/code/tetron`, with
   `tetron-mobile`/`tetron-webui`/`tetron-systray` checked out as siblings —
   `--consumer <path>` overrides the default sibling-directory guess, and a
   missing consumer repo is reported as INCONCLUSIVE rather than silently
   treated as "no usage found"). It does not attempt steps 3-4 — its output
   is candidates to review, not confirmed findings, and it says so in its
   own output. `#cross-repo-dead-code-sweep-script` in
   `DO-NOT-COMMIT/TODO_DETAILS.md` has the worked example: a clean run
   against this repo's actual siblings reproduced both previously-known
   loose ends from the `_tetron_certgen` cleanup (`APP_NAME` in
   `src/lib.rs`, `remove_by_network` in `src/peers.rs`) with no other
   false positives.
3. Exclude known-legitimate categories before flagging anything, so the
   output stays trustworthy: trait-method implementations required by a
   trait signature even when never called directly; enum
   variants/struct fields that exist only for a derive macro's benefit
   (`clap` subcommand dispatch, `serde` (de)serialization) even when never
   referenced by name in ordinary Rust code; anything already documented
   elsewhere as deliberately-kept compatibility scaffolding (tetron's own
   `d1_wire_compat_audit` — `src/control.rs`'s `DeviceCert`/`PairMsg`/
   `CertRefresh`/`Unpaired` — is the standing example: real, deliberate,
   not a new finding, do not re-flag it every sweep).
4. For each confirmed candidate, get real provenance instead of guessing —
   `git log --follow -S<symbol> -- <file>` finds when it was introduced;
   diffing forward from there toward the commit that removed its last
   caller (usually a feature-removal commit) confirms *why* it went dead,
   not just *that* it did. This matters for scoping the eventual fix
   correctly — deleting a whole orphaned cluster cleanly, rather than just
   the one symbol that happened to get grepped first.
5. Complementary automated check, not a replacement for steps 1-4:
   `cargo-udeps`/`cargo-machete` if installed, for unused *dependencies*
   rather than unused *code* — same spirit, different axis, and cheap to
   run alongside.
6. Report confirmed-dead items separately from anything merely
   plausible-but-unverified — don't let a "probably dead" guess sit next to
   a grep-confirmed finding with the same weight.
