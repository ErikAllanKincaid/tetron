# tetron codebase examination -- 2026-07-16

Scope: static review of `~/code/tetron` (docs, spec, source layout). No Rust
toolchain is installed in this environment (`cargo`/`rustc` not on PATH), so
build/test/clippy/`reconcile.py` could not be executed; findings below are
from reading source and documentation only.

## What it is

tetron is a Rust P2P mesh VPN built on [iroh](https://iroh.computer): peers
are addressed by an Ed25519 identity, get a stable virtual IPv4 (default
`10.88.0.0/24`) and IPv6 (`200::/7`) derived from that identity, and a root
daemon (`tetron daemon`) tunnels IP packets over iroh QUIC to a TUN device.
Unprivileged `tetron` CLI commands talk to the daemon over a Unix socket
(`/var/run/tetron/tetron.sock`, mode `0666`, authorized per-request by
`SO_PEERCRED` UID, not file permissions).

Lineage: `pitopi` (initial commit `c49816e`) -> `rayfish` (upstream) ->
`torpedo` (a feature-rich fork, lives in a sibling repo, `origin` remote) ->
`tetron` (this repo, a further-stripped single-purpose fork of torpedo,
diverged at torpedo commit `4809edb`). 589 commits total, most recent
`cb916df` (2026-07-15). MPL-2.0, matching upstream.

## Size and structure

- Main crate: 9,845 lines across `src/` (`src/membership.rs` 2,339,
  `src/daemon/mod.rs` 1,299, `src/config.rs` 1,286 are the largest).
- Helper crate `tetron-proto` (wire types, IPC msgpack protocol): ~700 lines.
- Total ~11,400 lines, under the PROPOSAL.md success target of "~15,000".
- Module split matches what `AGENTS.md`/`CLAUDE.md` documents: `src/daemon/`
  holds `MeshManager` and per-domain IPC handlers
  (`create_join`/`runtime`/`diagnostics`/`invite`/`admin`), `src/daemon/mesh/`
  holds the machinery (`accept.rs`, `join.rs`, `bootstrap.rs`,
  `publish.rs`/`reconverge.rs`/`coordinator.rs`/`select.rs`). Spot-checked
  against actual file layout -- consistent.
- Spec: `spec/design_spec.py` defines 87 requirement/constraint classes,
  gated by `reconcile.py` (14 checks: build, clippy `-D warnings`, test, and
  11 curated-token/grep gates enforcing the torpedo->tetron rename and
  feature-removal boundaries). Could not run it here (no cargo).
- Tests: unit tests live inline (`#[cfg(test)]` in modules, e.g.
  `tetron-proto/src/ipc.rs`), a Criterion microbench
  (`benches/forward.rs`) for the packet-forward hot path, and a manual e2e
  suite (`tests/e2e/{closed-net,reliability,restore-offline}/run.sh` +
  `tests/e2e.sh`) plus a manual checklist in `docs/TESTING.md` -- these are
  not wired into `reconcile.py` and require real multi-machine runs.

## Notable finding: `AGENTS.md`/`CLAUDE.md`/`PROPOSAL.md` are stale on invite minting

`AGENTS.md` (== `CLAUDE.md`, a symlink, both hold canonical-agent-guidance
status per the file's own header) states under MINIMAL-013:

> "Admission is approval-only; invite minting is REMOVED... tetron deletes
> the whole `tetron invite` create/list/revoke CLI + `InviteAction`... the
> `InviteCreate`/`InviteList`/`InviteRevoke` IPC ops..."

This is false against current `main`. All of it is present and active:

- `src/cli/invite.rs:6-15` -- `ipc_invite()` dispatches
  `InviteAction::Create/List/Revoke` to `IpcMessage::InviteCreate/List/Revoke`.
- `src/main.rs:140,179` -- `InviteAction` enum is a live clap subcommand.
- `tetron-proto/src/ipc.rs:141-169` -- `InviteCreate`/`InviteCreated`/
  `InviteList`/`InviteListResponse`/`InviteRevoke`/`InviteInfo` all exist,
  with round-trip tests.

`README.md`, `CHANGELOG.md`, `docs/TODO.md`, and `SECURITY.md` all describe
invite-key minting as a real, currently-supported, actively-developed
feature -- `docs/TODO.md` records it completed in Phases 1-4, then extended
by BLOB-001 (2026-07-xx, invites moved from a file-based `InviteStore` into
the signed `GroupBlob` so any co-coordinator can mint/validate). So the
project clearly reintroduced invite minting after MINIMAL-013 removed it,
but `AGENTS.md`/`PROPOSAL.md` (the docs an AI agent is told to treat as
canonical and read "before doing anything") were never updated to reflect
the reversal. A few concrete symptoms of the same drift:

- `AGENTS.md` line 5: "Full **tetron** (the feature-rich fork of rayfish)
  lives in its own repository" -- should read "full **torpedo**"; the
  sentence right after it compares `tetron/net/...` against
  `tetron/net/...` (identical strings), which is meaningless -- it should be
  comparing against `torpedo/net/...`. Reads like a mechanical
  find-and-replace (`torpedo` -> `tetron`) that over-applied to a sentence
  meant to name the *other* project.
- `SECURITY.md` still describes "Invite ledgers are written `0600`" -- the
  file-based `InviteStore` ledger was superseded by the in-blob invites per
  the BLOB-001 changelog entry; there is no longer a standalone ledger file
  to describe.
- `docs/TESTING.md` Stage 9 and its 2026-07-13 results log mix `torpedo` and
  `tetron` command names in the same transcript (`torpedo create` next to
  `tetron invite testnet create`), consistent with the rename landing
  mid-way through that test run rather than a doc error, but worth
  reconciling into one consistent binary name for future runs.

Recommendation: update `AGENTS.md`'s MINIMAL-013 paragraph (and the
`PROPOSAL.md` "what is removed" table row "Open networks, invite minting,
reusable-key minting") to reflect that invite minting is back, now
blob-backed, and fix the "full tetron"/self-referential ALPN sentence at the
top of `AGENTS.md`. Since `AGENTS.md` is read first by any agent working in
this repo, the stale claim risks an agent deleting still-used code under
the belief it is dead, or mis-describing the admission model to a user.

## Open, documented bug (recent, unresolved)

`docs/TODO.md` records **SUBNET-BUG-001** (found 2026-07-15, one day before
this review): a node joining a network whose subnet (from the signed
`GroupBlob`) differs from the node's own locally configured subnet gets its
mesh IP assigned correctly (visible in `tetron status`) but its TUN device
is still built with the *local* subnet. Packets addressed to the correct
mesh IP arrive over QUIC but are silently dropped by the kernel because the
destination doesn't match the TUN's actual address range -- no error is
logged anywhere. Severity is marked medium (silent data-plane failure, only
triggered when node subnets disagree, which the TODO notes is "common when
subnet was changed after initial setup"). Three fix options are sketched
(reject-on-mismatch, auto-adopt, or per-network TUN/policy routing as the
long-term fix tracked separately under "Subnet collision" / `docs/SUBNET_COLLISION.md`).
This is unaddressed as of the latest commit.

## Security model, as documented

- No userspace firewall (`MINIMAL-010`, removed): any peer sharing a network
  reaches every port a local service binds. This is stated as an explicit,
  loud design decision (D4 in `PROPOSAL.md`) rather than an oversight --
  README and `AGENTS.md` both tell operators to use nftables/ufw on the
  `tetron` TUN interface for port-level restriction. Reasonable trade-off
  for a "minimal" fork, but worth the user re-confirming it matches actual
  deployment intent (this is a materially different security posture than
  Tailscale's default ACL model).
- Auth is UID-based over a world-writable Unix socket
  (`SO_PEERCRED`/`check_authorized`), matching the Tailscale operator model.
  Mutating commands require root or the configured `operator_uid`.
  Reasonable, standard pattern for this kind of daemon.
- Room id (network public key) is explicitly a discovery key, not an
  admission credential -- signed `GroupBlob` + blake3-hashed invite secrets
  gate actual admission. Suggested-firewall fields from a peer's `GroupBlob`
  are consumed only from the DHT-verified, signed blob, never from an
  unauthenticated peer control message -- good MITM hygiene, called out
  explicitly in `SECURITY.md`.
- Identity secret key is a single `0600` file with no built-in backup story
  beyond "back it up yourself"; there's a mention of argon2+chacha20poly1305
  "encrypted identity backups" in `SECURITY.md` but no corresponding CLI
  command for it was found in `src/main.rs`'s subcommand list -- worth
  checking whether that's a stale doc claim too or an internal/undocumented
  path (not confirmed either way in this pass; flagging as unverified rather
  than as a finding).

## Health signals

- `cargo build`/`cargo test`/`cargo clippy` could not be run here (no Rust
  toolchain installed in this sandbox) -- code compiles or not is unverified
  by this review; take the module-boundary description in `AGENTS.md` as
  accurate for structure but not as proof of green CI.
- Working tree is clean (`git status` empty) as of this review; no
  uncommitted work in progress.
- Commit hygiene is good: conventional commit subjects, spec-first workflow
  (one `spec/design_spec.py` requirement class per behavior change),
  `libspec` snapshot linked per commit, `CHANGELOG.md` kept current in
  Keep-a-Changelog format. This is unusually disciplined for a personal
  project.
- Dependency list (`Cargo.toml`) is deliberately small and each dependency
  carries an inline comment explaining why it's there -- good signal for
  auditability, matches the stated "smaller daemon is easier to audit"
  motivation in `PROPOSAL.md`.

## Suggested next steps (not performed)

1. Reconcile `AGENTS.md`/`PROPOSAL.md` invite-minting language against
   actual `src/` state (see finding above) -- this is the highest-value fix
   since it is the doc an agent is instructed to trust first.
2. Fix SUBNET-BUG-001 or at least land the cheap mitigation (reject join on
   subnet mismatch with a clear error) -- it is a silent failure with no
   log line, the worst kind of bug to leave open.
3. Install a Rust toolchain in this environment (or run this on a host that
   has one) to actually execute `reconcile.py` and confirm the 14 gates are
   green, since none of that was verifiable in this pass.
