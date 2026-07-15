# tetron: execution plan

One requirement per commit, reconcile.py green at each step, `libspec link` after each commit (see AGENTS.md workflow). Each removal commit also: trims the reconcile.py checks and tests/ harness scripts that exercised the removed feature, updates AGENTS.md/README.md/CHANGELOG.md, and retires (deletes) any inherited spec class the removal moots, noting the retirement in the commit message.

Cherry-pick channel: `origin` points at the local full-torpedo repository. `git fetch origin && git log origin/main` to review upstream torpedo fixes; cherry-pick the ones that touch surviving files.

## Phase 0: project scaffolding (this commit)

- PROPOSAL.md, PLAN.md, MINIMAL-* / CON-M* spec classes appended to spec/design_spec.py, AGENTS.md variant banner.
- `libspec link` the commit: the ledger continues torpedo's history, so inherited SUBNET/RENAME/CON components stay tracked.

## Phase 1: independent leaves

Order is free; each is self-contained and low risk. All are pure deletions plus small call-site cleanup.

| Commit | Req | Scope |
|---|---|---|
| 1 | MINIMAL-002 | Self-update: src/update.rs, src/cli/update.rs, deps reqwest/rustls/self-replace/sha2/semver, the `update`/`auto-update`/`install --auto-update` CLI. Retires CON-006 (the gate is moot once the code is gone); reconcile.py `self_update` check becomes an absence grep. |
| 2 | MINIMAL-003 | Embedded SSH: src/ssh.rs, the 22<->30022 NAT in forward.rs, `firewall ssh` CLI, ssh config keys, deps russh/pty-process/uzers, tests/e2e/ssh. |
| 3 | MINIMAL-004 | Files + pairing: daemon/mesh/files.rs, daemon/file_service.rs, cli/files.rs, cli/pair.rs, onepassword.rs, revocation.rs, DeviceUserMap, FILES_ALPN/PAIR_ALPN, _torpedo_certgen. iroh-blobs stays (GroupBlob transport). Firewall peer rules keyed on user identity fall back to device identity until MINIMAL-010 deletes them anyway. |
| 4 | MINIMAL-005 | Direct connect: daemon/connect_service.rs, daemon/mesh/connect.rs, cli/connect.rs, CONNECT_ALPN, _torpedo_contact publisher, contact_secret_key. |
| 5 | MINIMAL-006 | Diagnostics: `torpedo ping`/`netcheck` CLI + daemon/mesh/diagnostics.rs. Keep a passive Pong reply to ControlMsg::Ping for wire compat (D1). |
| 6 | MINIMAL-007 | mDNS: spawn_mdns_discovery, `torpedo mdns` CLI, mdns_enabled config, iroh-mdns-address-lookup dep. |
| 7 | MINIMAL-008 | Peripherals: otel cargo feature, deeplink.rs + cli/open.rs, audit.rs. The `tor` feature and the per-network `--tor` flag are KEPT unchanged (D7/TOR-M01: QUIC/UDP can not ride Tor externally, the in-endpoint glue is the only integration; off by default, zero cost in default builds). |
| 8 | MINIMAL-009 | Observability: stats.rs Prometheus export, `torpedo report` + build_report, cli/update-style presentation of metrics. Keep the plain drop counters forward.rs needs for logs, or inline them. |

## Phase 2: firewall (the big cut)

| Commit | Req | Scope |
|---|---|---|
| 9 | MINIMAL-010 | Firewall enforcement: firewall.rs, cli/firewall.rs, daemon/mesh/firewall.rs, reject.rs, picker.rs, firewall.toml handling, auto_accept_firewall config, benches/forward.rs firewall benches. forward.rs keeps only the anti-spoof ingress check. GroupBlob keeps `suggested_firewall` (D1): reconverge ignores it, republish preserves it. ray-proto keeps policy.rs types. tests/e2e/firewall removed. |
| 10 | MINIMAL-011 | Apply layer: apply.rs, cli/alias.rs, daemon/mesh/alias.rs, `identityof`, EXAMPLE_SPEC. Depends on commit 9. |
| 11 | CON-M01 + CON-M02 | Add the two new reconcile.py checks: `dependency_absence` (Cargo.toml [dependencies] must not name the removed deps) and `wire_compat` (MESH_PROTOCOL_VERSION == 1; GroupBlob retains suggested_firewall/reusable_keys fields). Added here because phases 1-2 create the conditions they gate. Added before Phase 6 verification (2026-07-13). |
| 18 | RENAME-M01 | Crate identity rename: `rayfish` -> `tetron`, `ray-proto` -> `tetron-proto`. Add CON-M03 to reconcile.py. Internal only, D1 preserved. Done 2026-07-13. |

## Phase 3: DNS

| Commit | Req | Scope |
|---|---|---|
| 12 | MINIMAL-012 | Magic DNS: dns.rs, dns_config.rs, dns_resolver.rs, dns_packet.rs, daemon/dns_manager.rs, the port-53 intercept in forward.rs, magic-dns/dns-upstreams config keys, deps zbus/inotify, panic-hook DNS restore, tests/e2e/dns. Hostnames stay in the roster (wire compat, status display). Retires the DNS-related lines of inherited RENAME-006 spec text; reconcile.py `grep_hardcoded_cgnat` drops its dns.rs scan. |

## Phase 4: admission and lifecycle trim

| Commit | Req | Scope |
|---|---|---|
| 13 | MINIMAL-013 | Admission: drop `--open` (create is always Restricted), invite.rs ledger, cli/invite.rs, daemon/mesh/invite.rs, invite gossip handling (InviteShare/InviteUsed tolerated on receive, never sent), reusable-key minting. Keep: joiner-side invite redemption, blob reusable-key validation, requests/accept/deny, admin add/list, kick. tests/e2e invite scripts trimmed to the approval flow. |
| 14 | MINIMAL-014 | Rename + ephemeral: `torpedo hostname`, daemon/mesh/rename.rs, pending_hostname config, `torpedo ephemeral` + spawn_stale_member_pruner. Hostname is fixed at join (collision still resolved by coordinator at admission). |

## Phase 5: presentation and workspace

| Commit | Req | Scope |
|---|---|---|
| 15 | MINIMAL-015 | Plain CLI: style.rs, layout.rs, progress.rs, deps indicatif/crossterm/unicode-width/humansize/mime_guess; keep NO_COLOR-free plain text and `--json`. |
| 16 | MINIMAL-016 | Workspace: remove ray-mobile member (and the android/ dir), trim benches/ to the surviving forward path, prune Cargo.toml features to default-only, sweep cliff.toml/justfile targets that reference removed surfaces. |
| 17 | docs | Final AGENTS.md rewrite describing tetron as it now is (module list, CLI surface, flows), README rewrite with the D4 security-posture note and nftables example, CHANGELOG entry. |

## Phase 6: full product identity rename (RENAME-M02) -- DONE

| Commit | Req | Scope |
|---|---|---|
| ddf2db7 | RENAME-M02 | Full product rename: binary `tetron`, service `tetron.service`, paths `/etc/tetron`, `/var/log/tetron`, `/var/run/tetron/tetron.sock`, ALPN `tetron/net/...`, all user-facing CLI/messages. Retires CON-M02 (D1 severed). Adds CON-M04 constraint for sweep completeness. Done 2026-07-13. |

## Phase 7: verification -- DONE

- Trimmed e2e harness green: create/approve/join/traffic/kick/leave between two tetron nodes.
- Line-count and dependency audit against the success criteria in PROPOSAL.md.

## Post-MINIMAL work (on demand, in progress)

Not part of the MINIMAL milestone; each item is its own decision after Phase 6 is green.

### Laptop fleet -- DONE

Three changes to make a network of laptop users work without an always-on member:

- **CACHE-001** (aa5715e): peer address cache to disk. Solves all-offline reconnection.
- **BLOB-001** (79375be): invites in signed GroupBlob. Any coordinator validates.
- **COORD-001** (this commit): docs update making multi-coordinator the default practice.

### Pending

- TOR-M01: flexible per-network Tor policy (`any` / `tor` / `tor-isolated`). Tiers 1-2 already work via the kept `--tor` flag; the new work is tier 3, a second Tor-only endpoint with its own key, relays disabled, onion-only discovery (the only leak-free tier). Node-local routing only; never a blob or protocol change.
- ANDROID-CLEANUP: strip stale android/ entries from .gitignore (6 lines).
- macOS port: fix hardcoded subnet references in tun.rs for macOS route_peer_range.

## Standing rules

- Delete whole files where possible; do not reorganize surviving modules (keeps cherry-picks from torpedo clean).
- reconcile.py must stay green on every commit; a check that references a removed file is trimmed in the same commit that removes the file.
- Commit subjects conventional, message body states the requirement ID, no authorship trailers.
