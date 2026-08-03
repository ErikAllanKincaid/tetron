# tetron — Agent Guide

> `AGENTS.md` is the source of truth; `CLAUDE.md` is a symlink to it (so Claude Code auto-loads it while the portable `AGENTS.md` name serves every other agent tool). Edit `AGENTS.md`, never the symlink. **This file is kept small on purpose — it is loaded in full every session.** Deeper reference material lives in `docs/CLI_REFERENCE.md` and `docs/ARCHITECTURE.md` and gitignored `DO-NOT-COMMIT/` for planning and research docs; read those only when the task actually touches what they cover (see "Reference docs" below).

All work must be in a branch.
Any changes to tetron core require testing in the testsuite to verify no regression.
**All work must use libspec workflow.**

> **THIS REPOSITORY IS `tetron`**, a standalone P2P mesh VPN. This file is the canonical reference; read it before doing anything. The requirements in `spec/` govern the work (superseding the old `docs/PROPOSAL.md`/`docs/PLAN.md`, retired 2026-07-27 once this file covered the same ground; still available gitignored under `DO-NOT-COMMIT/`). Full tetron (the feature-rich fork of rayfish) lives in its own repository; `origin` points at it for cherry-picks — not wire-compatible with this fork (D1 severed by RENAME-M02, see `docs/ARCHITECTURE.md`).

## What tetron is (and is not)

Tetron is a fork **derivative of [rayfish](https://github.com/rayfish/rayfish)**. tetron, a P2P mesh VPN powered by [iroh](https://iroh.computer), exists as a standalone product: connect machines into private overlay with stable identity-derived addresses. It follows a **"do one thing well"** Linux philosophy. It defaults to `10.88.0.0/24` (10.x slice avoids Tailscale's `100.64.0.0/10`). Tetron can have multiple terton networks, each with different IP range.

The binary is **`tetron`**. The Cargo **package/library is `tetron`** (`[package] name = "tetron"`, `[[bin]] name = "tetron"`), with internal use tetron::…` paths and the `info,tetron=debug` log filter. The helper crate is `tetron-proto`. 

Tetron has a growing list of addons. ../tetron-mobile/ ../tetron-relay/ ../tetron-systray/ ../tetron-testsuite/ ../tetron-webui/ etc.  Each has it's own README.md and workflow.

## KEEP-ON-PURPOSE — do NOT rename these

**An agent must not "finish the rename" on any of these:**

- **Relay/discovery presets** — the `"rayfish"` config keyword and its preset URLs (`relay.iroh.rayfish.xyz`, `dns.iroh.rayfish.xyz`). Protected by **CON-001**; `reconcile.py`'s `relay_preset_untouched` check requires the literal `"rayfish" => Ok(preset.to_string())` in `src/config.rs`. Renaming would point nodes at nonexistent infrastructure.

**Author attribution:** `Cargo.toml`/`tetron-proto/Cargo.toml`'s `authors` is `["Dario", "ErikAllanKincaid"]` — Security reports go through GitHub private reporting (`SECURITY.md`), not email.

## Spec-first workflow (libspec + reconcile.py)

Changes are **spec-driven and committed one requirement at a time**. Full
step-by-step version, including branch naming, dependency-ordering
between requirements, and testsuite/docs/cross-repo follow-up:
**[`docs/tetron-workflow.md`](docs/tetron-workflow.md)**. Summary:

1. **Edit spec** — define/amend requirements under `spec/`. Decompose broad requirements into granular, single-responsibility classes (e.g. `HelpCommandReq`) rather than monolithic blocks.
2. **Diff spec (mandatory before coding)** — run `uv run libspec diff` (or the `libspec_diff` MCP tool) to review drift/dependencies before writing code.
3. **TDD** — write tests for the components first.
4. **Implement** — make the tests pass.
5. **Commit** — conventional subject, no authorship trailers of any kind; present the message to the user.
- **`spec/`** — modular Python specification package; each requirement/constraint is a Python class in a domain module (`core.py`, `addressing.py`, `branding.py`, `membership.py`, `cli.py`, `security.py`, `constraints.py`). The ID is the first line of the docstring (e.g. `SUBNET-001`, `CON-007`). `Requirement` subclasses are structural/design; `Constraint` subclasses carry Jinja expressions for automated verification.
- **`reconcile.py`** — the per-commit gate. `python3 reconcile.py` must exit `0`; it runs sixteen checks and prints a JSON context.
- **libspec** — `libspec diff` previews spec changes. There is no `build` command.

**The loop for a change:** amend/add the requirement class in the right `spec/` module → implement → `python3 reconcile.py` until green → commit.

## Build

```bash
cargo -q build                 # add --features tor for Tor transport
cargo -q check
cargo -q test
cargo -q clippy
cargo bench                    # Criterion microbenchmarks of the per-packet data path (benches/forward.rs)
cargo build --release          # distributable binary at target/release/tetron
```

The crate splits into a library (`src/lib.rs`, daemon modules as `pub mod`) and a thin binary (`src/main.rs`, the `tetron` CLI/IPC client, `use tetron::…`). The split lets benchmarks (`benches/`) and integration tests reach the internal data path.

**justfile:** `just cross`/`just deploy`/`just deploy-dev` identity was fixed (`binary := "tetron"`, `groupadd tetron`, `systemctl restart tetron`) — safe to use. `ray-mobile` was removed in MINIMAL-016 (no Android build in tetron).

## Reference docs

Read these whenever work is going to be done on the core architecture, not preemptively — they exist so this file can stay small:

- **[`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md)** — every `tetron <cmd>`, removed-feature history (`MINIMAL-*`), the privilege/access model. Read before touching `src/cli/`, `src/main.rs`, or any command's behavior.
- **[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)** — addressing scheme, architecture diagram, per-network TUN/multi-segment design, a module-by-module map of `src/`, and key flows (create/join/leave/kick/nuke/…). Read before touching `src/daemon/`, `src/membership.rs`, `src/transport.rs`, or anything under `src/daemon/mesh/`.
- Many files are in `DO-NOT-COMMIT/` including `TODO.md` and reasearch and planning files.

## Conventions

- Use `cargo -q` for all cargo commands; `tracing` for logging. `main::init_tracing` composes layers (console + file) with **split filters**: console/CLI stays at `info`, rolling daily files under `logdir::log_dir()` capture our crate at `debug` (`info,tetron=debug`; dependencies stay at `info`). `RUST_LOG` overrides both. Returns a `LogGuard` that must stay alive for the process.
- Tracing carries spans: network lifecycle handlers use `#[tracing::instrument]`; the per-peer reader + reconnect loop wrap tasks in `info_span!("peer"/"reconnect", …)` so the rolling-file logs are correlatable.
- Panics are fail-fast in the daemon: `main::install_panic_hook` (set only for `tetron daemon`) records the panic, appends it to `panic.log`, then `std::process::abort()`. The service unit restarts it.
- Never share I/O resources (TUN, sockets, streams) behind a Mutex — split into read/write halves. Avoid Mutex generally: prefer channels, atomics, or `RwLock`/`ArcSwap`.
- CLI subcommands carry short `visible_alias`es (clap): `create`→`new`, `leave`→`rm`, `status`→`st`/`ls`, `version`→`ver`; action verbs `list`→`ls`, `remove`→`rm`/`del`, `show`→`ls`/`list`, `add`→`a`, `revoke`→`rm`, `approve`→`ok`. Aliases must be unique within each `#[derive(Subcommand)]` enum.
- ALPN per network is `tetron/net/<version>/<pubkey-prefix>` — see `docs/ARCHITECTURE.md`. The version segment is that protocol's compatibility gate; each protocol versions independently, bumped in the same change that breaks its wire format.
- TUN MTU 1280 (IPv6 minimum link MTU, RFC 8200 §5; matches WireGuard/Tailscale). Wire format (control + IPC): 4-byte BE length + msgpack body.
- Room id = per-network public key string (discovery only, never a credential). Networks are always closed: joining needs an invite key (LIVE-001), or against a full-tetron coordinator, a reusable key. Local aliases (adjective-noun-noun) are display-only.
- Config under `config::config_dir()` (`/etc/tetron` on Linux, `~/.config/tetron` on macOS): `secret_key`, `settings.toml`, `networks/<name>.toml` (one per network). Pre-migration installs auto-split the old `networks.toml`. On Linux the tree is `root:tetron`; secret-bearing files are `0600 root:root`. CLI commands that write identity directly need root on Linux since the tree is under `/etc`.
- Keep commit subjects conventional (`feat`/`fix`/`docs`/`style`/`ci`/…) so git-cliff can render release notes. **Commit messages contain ONLY a description of what was done — never an authorship trailer (`Co-Authored-By`, `Author`, etc.) of any kind.**
- **Cutting a release:** bump `version` in both `Cargo.toml` and `tetron-proto/Cargo.toml` together (kept in lockstep), then run a real `cargo build` (not just edit the TOMLs) so `Cargo.lock` picks up both version changes before committing. `release.yml`'s build matrix runs `cargo build --release --locked`, which hard-fails if the lockfile doesn't match — this is exactly what broke `v0.9.1` (every matrix target failed identically at "Build binary" because the version bump commit never regenerated `Cargo.lock`; `create-release` itself still succeeded, so a binary-less release got published). Commit as `chore: bump version to X.Y.Z`, then `git tag vX.Y.Z && git push origin main && git push origin vX.Y.Z`.
- Always update docs (this `AGENTS.md`, `docs/CLI_REFERENCE.md`, `docs/ARCHITECTURE.md`, `README.md`) after finishing a feature or significant change, and amend/add the requirement in the relevant `spec/` module, keeping `reconcile.py` green.
- Keep `CHANGELOG.md` current as part of every change (`## [Unreleased]`, Keep a Changelog format: `Added`/`Changed`/`Fixed`/`Performance`), describing behavior from the user's perspective. Skip pure-internal churn with no user-visible effect.
