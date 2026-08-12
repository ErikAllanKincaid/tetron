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
(`feat`/`fix`/`docs`/`chore`/…). The slug should name the requirement(s)
the branch carries, not the ticket/conversation that prompted it.

## 2. Spec first

Every change is driven by a requirement (or several) under `spec/` —
modular Python, one requirement per class, decomposed into granular,
single-responsibility pieces rather than one monolithic block. The
requirement ID is the first line of the class's docstring. Pick the
module by what code the requirement actually touches, not by an abstract
theme.

Before writing a new requirement, check whether an existing one already
covers — or explicitly forbids — the same territory.

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

Tests first, per requirement, before implementation exists. Colocated
near the code under test, not in a separate test tree.

## 6. Implement

In the dependency order worked out in step 3. Make the tests from step 5
pass; nothing more.

## 7. `reconcile.py`

The fast, local, per-commit gate — must exit `0` before each commit
(check count lives in `AGENTS.md`, not duplicated here since it drifts).
This is separate from, and prior to, step 8 — it is not a substitute for
testsuite coverage, and testsuite is not a substitute for it.

## 8. testsuite

Any change to tetron core requires a `tetron-testsuite` pass to verify no
regression — the heavier, VM-based, cross-network check that `reconcile.py`
does not attempt. Not required for addon-only, nor documentation changes.
If `libvirtd` isn't active on the chosen `TESTSUITE_PHYSICAL_HOST`, it's
fine to just start it.

## 9. Commit

Conventional subject (`feat`/`fix`/`docs`/…), matching `git-cliff`'s
release-note rendering. **No authorship trailer of any kind** — the
commit author is already set by git config. One commit per requirement by
default, even when several were designed together in the same sitting —
bundle only when a reviewer explicitly decides the requirements are too
entangled to review separately.

`.github/PULL_REQUEST_TEMPLATE.md` is intentionally minimal — commit
messages already carry the real description (steps 2-6 ask for that);
GitHub's PR page shows the commit log natively.

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
bump, sometimes UI work) in that repo's own tracking — never assume it
auto-follows just because the change is additive. Per the
standing priority order (core, then addons, then integration), this
follow-up is separate, later work, not bundled into the branch that
changed core.
