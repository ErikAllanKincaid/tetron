# torpedo — TODO

Tracking for deferred work on the fork. See `DESIGN.md` for decisions,
`AGENTS.md` for agent guidance, and `spec/design_spec.py` for the requirement set.

## Upcoming (active agenda)

- [ ] **DNS-003 — CRITICAL, TOP PRIORITY: mutual DNS forwarding loop with
      Tailscale on tier-5 hosts.** Found 2026-07-08 live testing on xps-17-9720.
      torpedo's `DirectResolvConf` takeover rewrites `/etc/resolv.conf` to its
      own magic resolver; `tailscaled` also watches that file for its own
      upstream and adopts torpedo's IP, creating a forever query loop between
      the two (`torpedo -> Tailscale -> torpedo -> ...`). Breaks ALL system DNS
      (not just `.ray`), including torpedo's own pkarr discovery (so `torpedo
      join` fails with a misleading "Service 'pkarr' failed"). `.ray` names
      resolve fine — the bug is specifically the non-`.ray` upstream-forwarding
      path. Does not reproduce on tier-1 (systemd-resolved) hosts, since
      neither daemon touches `/etc/resolv.conf` there. See
      `spec/design_spec.py`'s `DNS-003` for full diagnosis + evidence.
      **Primary fix landed (DNS-005):** Magic DNS is now opt-in — `magic-dns`
      config (`off|auto|direct`, default `auto`) no longer seizes
      `/etc/resolv.conf` unless `direct`, so the loop cannot occur by default;
      reach peers by mesh IP (`torpedo status`). Still to do: verify live on
      xps + Tailscale, and implement `DNS-004` (100.64/10 loop-breaker +
      real-upstream recovery) as the safety net for the opt-in `direct` path.

- [x] Documentation: `README.md` (torpedo-focused fork intro + background/further
      reading + image) and `AGENTS.md` (canonical agent guide; `CLAUDE.md` symlink).
- [ ] Testing: build a distributable binary for the other test machines
      (`cargo build --release` → `target/release/torpedo`, or `just cross` for a
      portable build). Decide static (musl) vs dynamic (glibc >= 2.39 on target).
- [ ] Push `master` to `origin` (github.com/ErikAllanKincaid/torpedo) when ready.
- [ ] Manual Phase-7 live test: `torpedo up` + `create --subnet` / `config set
      subnet` on two machines, confirm mesh + Tailscale coexistence.
- [ ] Optional guardrail: add `RENAME-010` (build-tooling identity) + a
      `reconcile.py` check that greps `justfile`/`contrib/` for stale
      `ray`/`rayfish` tokens, so the justfile fix can not silently regress.
      (The justfile is not Rust, so CON-007 does not cover it.)
- [x] **RENAME-011 — user-facing CLI hint strings still say `ray`** (found in
      Phase-7 testing: `torpedo create` prints `ray join …` / `ray up`). Done:
      41 live/reachable sites swept `ray <subcommand>` -> `torpedo <subcommand>`
      across `src/main.rs`, `src/apply.rs` (incl. `EXAMPLE_SPEC`),
      `src/onepassword.rs`, `src/cli/{status,network,invite,pair,connect,alias,
      service,files,firewall}.rs`, `src/daemon/mod.rs`, `src/daemon/mesh/
      {runtime,create_join,files,firewall}.rs`, plus the dormant `APP_NAME`
      constant in `src/lib.rs`. Since this is pre-release WIP with no real
      backups to break, the 1Password item **title** default was also renamed
      `Rayfish Identity` -> `Torpedo Identity` (no back-compat lookup needed).
      See `spec/design_spec.py`'s `RENAME-011` for the full include/exclude list
      (the `.ray` Magic-DNS TLD and the internal `rayfish` crate name are
      correctly excluded). No `reconcile.py` guard added (see that class's
      docstring for why a token-count gate would false-fail on the
      deliberately-untouched comments and dead `cli/update.rs` code).
- [ ] **DNS-001-fix — warning not delivered in the real flow** (found in Phase-7 on
      tier-5 xps): the daemon auto-activates at startup, so the DNS takeover + warning
      happen there (log only) and the interactive `sudo torpedo up` short-circuits with
      `already up`, never populating the warnings channel. Fix: persist the active DNS
      mode + warning on the daemon (set in `DnsManager::configure`); return it from `up`
      even on the already-active path. Pairs with DNS-002.
- [ ] **DNS-002 — surface active DNS mode in `torpedo status`** (now necessary, not
      optional): the daemon exposes its DNS backend / takeover state; `status` (and
      `--json`) show it, covering the non-interactive (reboot / auto-activate) path where
      `up` prints nothing. Add `dns_mode` to `StatusResponse`; CLI renders it.
- [ ] **Doc fix — resolver IP is subnet-derived** (`10.88.100.53` on the default subnet,
      not `100.100.100.53`): correct AGENTS.md, TESTING.md, and any prose that hardcodes
      `100.100.100.53`.
- [ ] **Investigate — resolv.conf re-assert storm** (3x within ~1s at startup on xps):
      confirm it always settles; if some hosts sustain the trample fight, damp the
      re-assert loop or widen the quiet-window guard.
- [ ] **CRITICAL — `create --subnet` corrupts the data plane** (Phase-7): it sets the
      network roster/blob to the requested subnet but leaves the node's TUN/config
      subnet at default, so roster (`10.99.x`) and TUN (`10.88.x`) diverge and NO IP
      forwarding works between nodes (raw `ping` fails both ways; only `torpedo ping`,
      which is identity-based, works). The `--subnet` flag does only the roster half of
      what `config set subnet` + restart does. Fix: make `create --subnet` set the node
      subnet (rebuild the TUN live, or require/trigger a restart), or reject `--subnet`
      when it differs from the node's current subnet with a clear "run `config set
      subnet <cidr>` + restart first" message. See `create_join.rs` create path +
      `set_node_subnet` + `blob_subnet`.
- [ ] **Doc — audit AGENTS.md invite/CLI against the real binary**: `torpedo invite`
      has no `--hostname`/`--expires`/`--qr`/`--reusable`/`list`/`revoke` (usage is just
      `invite <NETWORK>`), yet AGENTS.md (inherited from upstream) documents them. Sweep
      AGENTS.md for other commands/flags the fork's binary does not actually implement.

## Platform rewrites (macOS, Android) — adapt to torpedo

Decision: adapt both to torpedo rather than rip out. Ripping out stays the
alternative only if torpedo becomes permanently Linux-only.

### macOS rewrite
- [ ] Make `route_peer_range` **subnet-agnostic** (`src/tun.rs:286`, `#[cfg(macos)]`):
      it hardcodes `route add -inet 100.64.0.0/10` (+ `-inet6 200::/7`). The Linux
      path already reads the network's configured subnet; the macOS path does not,
      so on macOS the fork routes the wrong /10 and ignores `--subnet`.
- [ ] Audit `route_self_loopback` and any other `#[cfg(target_os = "macos")]`
      block in `src/tun.rs` for hardcoded `100.64` / stale identity.
- [ ] Identity: launchd label already `com.torpedo.vpn` (RENAME-008); confirm no
      other `rayfish` host artifacts remain on macOS-only paths.
- [ ] **Must build + test on a real Mac** — cfg(macos) code is not compiled or
      type-checked on this Linux host, so all the above is compiler-unverified.

### Android torpedo rewrite
- [ ] **Deep-link scheme mismatch (broken):** `AndroidManifest.xml` registers
      `android:scheme="rayfish"` but the Rust side is now `torpedo://` (RENAME-007).
      Android deep links do not work until the manifest is updated to `torpedo`.
- [ ] Kotlin identity rename `rayfish` -> `torpedo`: package `xyz.rayfish.android`,
      `RayfishApp` / `RayfishTheme` / `RayfishVpnService.kt`, thread `rayfish-node-stop`,
      and the JVM package dir `android/app/src/main/java/xyz/rayfish/...`.
- [ ] `ray-mobile` crate (`lib.rs`, `android_tun.rs`, `diag.rs`): make the
      `VpnService` TUN setup **subnet-agnostic** (drop `100.64` assumptions). Decide
      whether to rename the crate/artifact (`ray-mobile` / `libray_mobile`) or keep
      it as an internal name like the `rayfish` library crate.
- [ ] Build prerequisites: `cargo-ndk`, the Android rust targets, JDK 17
      (`just apk`). Verify `just build`/`just apk` after the fixes.

## Deferred (decision made: not now)

### Adapt the multi-node test harnesses to torpedo
- `tests/e2e/*`, `tests/bench/*`, `tests/lib/common.sh` are upstream shell-based
  harnesses. They are **not** part of `cargo test` / `reconcile.py`, so they do
  not gate the build.
- They hardcode the old identity (`ray`/`rayfish`, `/etc/rayfish`) and the old
  `100.64.x.x` range, so they fail against torpedo as-is.
- Downstream of the "prepare a distributable binary" task (they run the binary).
- When adapting: make them **subnet-agnostic** — read each node's real assigned
  IP from `torpedo status` (e.g. `common.sh` `own_ip`) instead of assuming
  `100.64`, so they never rot on a default-subnet change again.
- Priority: after the fork is proven via the manual Phase-7 two-machine test;
  worthwhile for automated multi-node regression coverage of a P2P VPN.

## Notes
- Relay / discovery-DNS `rayfish` presets are **kept on purpose** (upstream
  infra, default is n0; honest for a fork). Do not rename — protected by CON-001.
- Self-update is neutralized (`SELF_UPDATE_ENABLED = false`); do NOT enable it —
  `REPO_SLUG` still points at upstream rayfish. Guarded by CON-006.
- Internal Cargo library crate name `rayfish` (`use rayfish::…`, `info,rayfish=debug`)
  is kept on purpose — renaming it is churn with no user-visible benefit.
