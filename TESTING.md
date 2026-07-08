# torpedo — Manual Test Plan (Phase 7)

Live two-machine acceptance test for the fork. This is a **manual** checklist,
not part of `cargo test` / `reconcile.py`, so it does not gate the build. Run it
after producing a distributable binary to prove the fork works end to end and,
above all, that torpedo **coexists with Tailscale**.

Command reference: `AGENTS.md`. Requirement set: `spec/design_spec.py`.

## Prerequisites

| Machine                      | Role                       | Default hostname in this plan |
| ---------------------------- | -------------------------- | ----------------------------- |
| **AORUS** (590I-AORUS-ULTRA) | Control node (coordinator) | `aorus`                       |
| **xps-17-9720**              | Member (joins by invite)   | `xps`                         |

Roles are swappable; the plan assumes AORUS coordinates because the binary is
built there.

Before starting, decide and record two things, because they change what is
actually under test:

- **Tailscale:** run this with Tailscale **up on at least one node**. Coexistence
  is the fork's headline feature (upstream refuses to start next to Tailscale),
  so a test without Tailscale running skips the point.
- **Network placement:** same-LAN gives a fast **direct** path via mDNS and never
  exercises NAT traversal; different networks (e.g. xps on a hotspot) test
  hole-punching and relay fallback. Ideally run the connectivity stages **once
  each way**. `torpedo ping` reports `direct` vs `relay` so you can see which you
  got.
- **Debian trixie / minimal installs rewrite `/etc/resolv.conf`.** On a host
  with no systemd-resolved, NetworkManager, resolvectl, or resolvconf (e.g. a
  minimal Debian trixie netinst — this is the common case there), `sudo torpedo
  up` falls through to directly taking over `/etc/resolv.conf` as a last
  resort: it backs up the original to `/etc/resolv.conf.before-torpedo`, prints
  a warning naming the backup and restore path, and restores it automatically
  on `torpedo down`/`uninstall` or after a crash. See the README's "DNS on
  hosts without a resolver manager" section for the full mechanism, and Stage
  13 below (13b/13d) for the guided repro/verification of it.

Install the binary on both machines first (glibc build needs the target on
glibc >= 2.39; otherwise rebuild with `--target x86_64-unknown-linux-musl`):

```bash
# AORUS (built here)
sudo install -m 755 target/release/torpedo /usr/local/bin/torpedo
# xps (after scp'ing the binary to /tmp/torpedo)
sudo install -m 755 /tmp/torpedo /usr/local/bin/torpedo
```

Legend: `[ ]` = to do, `[x]` = passed, `[!]` = failed (record the finding).

---

## Stage 0 — Init + subnet (both machines)

**Goal:** the daemon starts (coexisting with Tailscale) and — for the
configurable-subnet test — **every** node is put on the SAME non-default subnet
*before* any network exists. The subnet is a node-wide setting the TUN is built
from at daemon start, so it must be set and applied by a restart up front; the
`--subnet` flag on `create`/`join` only persists it and warns you to restart.

```bash
sudo torpedo up                               # install + start daemon; grants operator
# Non-default-subnet test: run on EVERY node, then restart so the TUN rebuilds on it.
# (The default 10.88.0.0/16 already avoids Tailscale — skip these two to test on it.)
sudo torpedo config set subnet 10.99.0.0/16
sudo torpedo restart
torpedo status                                # daemon reachable
ip -4 addr show tun0                           # inet is 10.99.x (10.88.x if you skipped)
```

- [x] Daemon starts on both (no "refusing to run next to Tailscale").
- [x] `ip addr show tun0` shows the chosen subnet on **both** nodes (must match).
      AORUS `10.99.49.50/16`, xps `10.99.3.155/16`.
- [x] Tailscale is still fully functional on the node(s) where it runs.
      `tailscale status` shows both nodes' own tailnet address active and
      peers visible, unaffected by torpedo's `10.99.0.0/16` overlay.

- Note: `torpedo config get subnet` is root-only — use `sudo torpedo config get subnet`
  (a non-root call now hints instead of printing a misleading `<default>`).

## Stage 1 — First control node (AORUS)

**Goal:** create a closed network with a chosen hostname (the subnet was already
set in Stage 0, so no `--subnet` here — the node is already on it).

```bash
torpedo create --name testnet --hostname aorus   # closed by default; --open for public
```

- [x] A room id (network public key) is printed. `cdcbbcfe…3935`. Hint text
      correctly shows `torpedo join …` / `torpedo up` (RENAME-011 confirmed
      live in this exact command).
- [ ] Without `--hostname` the node auto-generates a random name (e.g. `hill`); pass
  
      `--hostname` to control it, or fix it later with `torpedo hostname testnet aorus`.
      (Not exercised this run — we always passed `--hostname` explicitly.)

## Stage 2 — Check parameters (AORUS)

```bash
torpedo status --json
```

- [x] Our mesh IPv4 is inside `10.99.0.0/16` (NOT `100.64.x.x`). `10.99.49.50`.
- [x] Network role is `coordinator`; a `200::/7` IPv6 is assigned.
      `role: "Coordinator"`, ipv6 `21f:cfa6:2bd9:7d75:8002:c6b9:29cb:85ee`.

## Stage 3 — Enroll xps by invite

**Goal:** closed-network admission via a single-use invite; the joiner picks its
own hostname.

```bash
# AORUS — invite takes ONLY the network name in this build
# (no --hostname/--expires/--qr/--reusable/list/revoke).
torpedo invite testnet                             # prints a single-use invite code
# xps — must have finished Stage 0 (up + same subnet + restart) first.
torpedo join <invite-code> --hostname xps-17-9720
```

- [ ] xps joins and reports success.
- [ ] If xps was NOT on the network's subnet before joining, `join` prints a
  
      `⚠ subnet … takes effect after sudo torpedo restart` — restart xps, then it
      lands on the shared subnet. (Do Stage 0's subnet step first to avoid this.)

## Stage 4 — Check parameters (both)

```bash
torpedo status          # on AORUS and on xps
```

- [ ] Each side lists **2** members and shows the other's hostname + `10.99.x.x` IP.
- [ ] Connection type is `direct` (same-LAN) or `relay` (cross-NAT); note which.

## Stage 5 — Negative: uninvited join is refused

**Goal:** the closed-network gate actually gates.

```bash
# a third machine, or xps with the room id but NO invite:
torpedo join <room-id>
```

- [ ] Join is **denied or held pending**, never auto-admitted.

## Stage 6 — Testing the connections

**Goal:** separate "mesh is up" (control plane) from "forwarding works" (data
plane), in both directions.

```bash
torpedo ping xps-17-9720    # from AORUS: RTT + loss + direct/relay path
torpedo ping aorus          # from xps: the reverse direction
ping 10.99.<xps>            # raw ICMP through the TUN (default fw allows in icmp)
torpedo netcheck            # endpoint diagnostics on each node
```

- [ ] `torpedo ping` succeeds **both** directions with low loss.
- [ ] Raw `ping` to the peer's mesh IP works.
- [ ] (Cross-NAT run) path is `direct` after hole-punching, or `relay` as fallback.

## Stage 7 — Firewall

**Goal:** prove the **FW-001** mode-dependent inbound default, then that an
explicit rule still overrides it, before SSH/send depend on the firewall.
`testnet` (Stage 1) is a **closed** network (created without `--open`), so per
FW-001 it seeds a network-scoped `allow in any` (`RuleOrigin::ClosedDefault`,
`src/firewall.rs`) the moment it is created/joined-by-invite — connectivity is
open like a normal LAN, and host-service auth (SSH keys, etc.) is the gate.
**Open** networks get no such rule and keep the secure default-deny inbound.
Run a listener on the target and probe it from the peer, on **both** a closed
and an open network, to see the contrast.

```bash
# xps: start a throwaway TCP listener on 8080
python3 -m http.server 8080

# AORUS: closed network (testnet) — should SUCCEED with no explicit rule (FW-001 default-allow)
curl --max-time 5 http://xps.ray:8080/
torpedo firewall show   # confirm the seeded "closed-net default" allow-in-any rule for testnet

# xps: add an explicit deny for aorus on testnet, confirm it overrides the default-allow
torpedo firewall add in deny -p tcp -P 8080 --peer aorus
curl --max-time 5 http://xps.ray:8080/   # from AORUS: should now FAIL
torpedo firewall show                    # note the new rule's index
torpedo firewall remove <index>          # clean up before Stage 8+ (remove takes an index, not a selector)

# --- contrast: an OPEN network stays default-deny ---
# AORUS: create a second, open network and have xps join by room id (no invite)
torpedo create --open --name opennet --hostname aorus
# xps:
torpedo join <opennet-room-id> --hostname xps-open
# xps: start a listener bound the same way, then probe from AORUS over opennet
python3 -m http.server 8081
# AORUS: should FAIL under the default inbound-deny (no FW-001 rule on an open network)
curl --max-time 5 http://xps-open.ray:8081/
torpedo firewall show   # no "closed-net default" rule for opennet
```

- [ ] On **closed** `testnet`, the probe **succeeds with no explicit rule** —
      `firewall show` lists the seeded `ClosedDefault` allow-in-any rule for
      `testnet`, appended at the back.
- [ ] An explicit `deny` rule on `testnet` still **overrides** the default-allow
      (explicit rules win over `ClosedDefault`, since it sits at the back).
- [ ] On the **open** `opennet`, the probe is **blocked** by default (no
      `ClosedDefault` rule is seeded for an open network) — confirms FW-001 is
      closed-network-only.
- [ ] `firewall show` distinguishes rule origins clearly (`closed-net default`
      vs. explicit `Local`/`Network` rules).

## Stage 8 — Magic DNS

**Goal:** `.ray` names resolve, normal DNS still works, dual-stack, and Tailscale
DNS is not broken by ours.

```bash
ping xps.ray            # resolves via .ray TLD to the mesh IP
ping6 xps.ray           # AAAA over 200::/7
ping github.com         # normal (non-.ray) DNS must still resolve
```

- [ ] `xps.ray` resolves to the `10.99.x.x` mesh IP (A) and a `200::` address (AAAA).
- [ ] `github.com` resolves (upstream passthrough intact).
- [ ] Tailscale MagicDNS (`*.ts.net`) still resolves on nodes running Tailscale.

## Stage 9 — Mesh SSH

**Goal:** keyless SSH over the mesh, gated by the allow list, coexisting with any
host sshd.

```bash
# xps (login target)
torpedo firewall ssh on
torpedo firewall ssh allow testnet aorus --user <youruser>
torpedo firewall ssh show
# AORUS
ssh <youruser>@xps.ray
```

- [ ] `ssh <user>@xps.ray` logs in with **no SSH key exchanged**.
- [ ] A user NOT in the allow list (or root, under the default) is refused.

## Stage 10 — torpedo send

**Goal:** content-addressed file transfer, small then large.

```bash
# AORUS
echo "hello torpedo" > /tmp/small.txt
torpedo send /tmp/small.txt xps
# large file for throughput
head -c 500M /dev/urandom > /tmp/big.bin
torpedo send /tmp/big.bin xps
# xps
torpedo files                       # note the offer id(s)
torpedo files accept <id> --output /tmp
```

- [ ] Small file arrives and matches (`diff`/`sha256sum`).
- [ ] Large file transfers and verifies (hash-checked on accept).

## Stage 11 — Lifecycle

**Goal:** restarts keep the same IP; standby vs offline behave; leave/kick tear
down cleanly.

```bash
sudo torpedo restart            # a node: rejoins automatically
torpedo status                  # confirm SAME mesh IP as before
torpedo down; torpedo up        # data plane standby then active (still connected)
sudo torpedo stop; sudo torpedo start   # fully offline then online
torpedo leave testnet           # on xps, then rejoin via a fresh invite
torpedo kick testnet xps        # on AORUS (closed net): mesh-wide teardown
```

- [ ] After `restart`, the node rejoins with the **same** mesh IP (stable addressing).
- [ ] Peers distinguish `down` (still online) from `stop` (offline) in `status`.
- [ ] `leave` prunes the member on the coordinator; rejoin works.
- [ ] `kick` removes xps mesh-wide; a kicked node does not churn back in.

## Stage 12 — Add another control node (failover)

**Goal:** a second coordinator is real, and admission survives the first one going
offline. Promotion alone is not the test; failover is.

```bash
# AORUS: promote xps to co-coordinator (grants the network key)
torpedo admin add testnet xps
torpedo admin list testnet
# xps: prove it can now admit — mint an invite FROM xps for a 3rd machine
torpedo invite testnet --hostname node3
# AORUS: go offline, then confirm the 3rd machine can still join via xps
sudo torpedo stop
```

- [ ] xps shows as a key-holder in `admin list`.
- [ ] A third machine joins through xps's invite.
- [ ] With AORUS stopped, the mesh keeps working and xps still admits members.

## Stage 13 — DNS takeover, backup, and clean restore

**Goal:** the DNS integration is transparent, preserves the original file, and
never blackholes the host on teardown or crash. Safety stage, and DNS is the main
Tailscale conflict surface. Which path a host takes depends on its DNS backend
(`detect_and_configure` tries systemd-resolved -> NetworkManager -> resolvectl ->
resolvconf -> direct `/etc/resolv.conf` takeover), so test both classes.

### 13a — Split-DNS host (systemd-resolved / NetworkManager, e.g. stock Ubuntu)

```bash
cat /etc/resolv.conf            # BEFORE
sudo torpedo up                 # watch the output
ping github.com                 # normal DNS still resolves
sudo torpedo uninstall
ping github.com                 # still resolves after teardown
```

- [ ] `/etc/resolv.conf` is NOT rewritten (split-DNS, no takeover).
- [ ] `torpedo up` prints **no** resolv.conf takeover warning on this host.
- [ ] Normal DNS resolves during and after.

### 13b — Direct-takeover host (no systemd-resolved/NM/resolvconf, e.g. default Debian server)

This is the path the field report hit, and the one **DNS-001** now warns about.

```bash
sudo torpedo up                             # EXPECT the DNS-001 takeover warning
ls -l /etc/resolv.conf.before-torpedo       # backup was created
cat /etc/resolv.conf                        # "# Added by torpedo", nameserver <subnet>.100.53 (e.g. 10.99.100.53; subnet-derived)
ping github.com                             # captured upstreams forward normal DNS
sudo torpedo uninstall
cat /etc/resolv.conf                        # restored to the pre-torpedo original
ping github.com                             # resolves after restore
```

- [ ] `torpedo up` shows the **DNS-001** warning naming
  
      `/etc/resolv.conf.before-torpedo` and the restore command.

- [ ] Backup exists while up; the live file carries the `# Added by torpedo`
  
      marker and points at the subnet-derived resolver (e.g. `10.99.100.53`).

- [ ] Normal (non-`.ray`) DNS still resolves while up (upstream passthrough).

- [ ] After uninstall, `/etc/resolv.conf` matches the original, the backup file is
  
      gone, and `github.com` resolves.

- [ ] No leftover NetworkManager `dns=none` drop-in or torpedo routes.

### 13c — Symlinked resolv.conf + crash recovery

```bash
ls -l /etc/resolv.conf                      # note if it is a symlink (systemd stub)
sudo torpedo up
sudo systemctl kill -s SIGKILL torpedo      # hard kill (no clean revert runs)
ping github.com                             # daemon auto-restarts (Restart=on-failure)
sudo torpedo status                         # confirm it came back
```

- [ ] A symlinked `/etc/resolv.conf` is not left dangling or pointing at a dead
  
      resolver after teardown.

- [ ] After a hard kill, DNS recovers on the daemon's auto-restart
  
      (`restore_stale_backups` on start; the panic path also runs
      `emergency_restore_resolv_conf`). Note any window where DNS was down.

### 13d — Dedicated tier-5 reproduction (single machine, no mesh)

The field-report scenario and the direct **DNS-001** verification. Needs only
**one** host that lands on tier 5 (no systemd-resolved, no split-capable
NetworkManager, no resolvconf), for example a minimal Debian trixie VM (netinst,
no desktop task selected). No second peer and no network are required: `torpedo
up` triggers the takeover on its own.

**First, confirm the host takes tier 5** (before installing torpedo). The chain
is systemd-resolved -> NetworkManager(`dnsmasq`|`systemd-resolved` mode) ->
resolvectl -> resolvconf -> direct `/etc/resolv.conf` takeover; first hit wins,
so all four absent means tier 5.

```bash
systemctl is-active systemd-resolved                     # must NOT be "active"
NetworkManager --print-config 2>/dev/null | grep -A3 '\[main\]' | grep -i '^dns='  # absent, or a non dnsmasq/systemd-resolved mode ⇒ NM skipped
ls /sbin/resolvconf /usr/sbin/resolvconf 2>/dev/null     # must be absent
ls -l /etc/resolv.conf                                   # a plain DHCP-managed file, not a resolved/NM symlink
```

- [ ] All four split-DNS backends are absent (host will take tier 5).

**Then reproduce and verify:**

```bash
cp /etc/resolv.conf /tmp/resolv.conf.orig      # independent copy to diff against
sudo torpedo up                                # EXPECT the DNS-001 takeover warning
ls -l /etc/resolv.conf.before-torpedo          # backup created
cat /etc/resolv.conf                           # "# Added by torpedo", nameserver <subnet>.100.53 (e.g. 10.99.100.53; subnet-derived)
ping github.com                                # captured upstreams still forward normal DNS
sudo torpedo uninstall
diff /etc/resolv.conf /tmp/resolv.conf.orig    # empty ⇒ restored to the pre-torpedo original
ping github.com                                # resolves after restore
```

- [ ] `torpedo up` prints the DNS-001 warning naming `/etc/resolv.conf.before-torpedo`
  
      and the restore command.

- [ ] Backup exists while up; the live file carries the `# Added by torpedo` marker
  
      and points at the subnet-derived resolver (e.g. `10.99.100.53`).

- [ ] Non-`.ray` DNS resolves while up (upstream passthrough).

- [ ] After uninstall, `diff` is empty (original restored) and the backup file is gone.

- [ ] This ran with no peers, confirming DNS-001 is validated in isolation from the mesh.

---

## Priority

Treat as **mandatory** (the fork's purpose or the worst failure modes):

- Stage 0/8 Tailscale coexistence, Stage 1/2 subnet configurability,
  Stage 11 stable IP on restart, Stage 13 clean uninstall / DNS restore.

The rest are strong "should" tests; prioritize by how much you trust each
subsystem. Also confirm once that **self-update stays disabled**:

```bash
torpedo update --check          # must no-op / refuse (SELF_UPDATE_ENABLED = false)
```

## Results log

Record date, machines, Tailscale on/off, network placement (LAN vs cross-NAT),
and any `[!]` findings with the `torpedo report` bundle path.

### Run 2026-07-07, attempt 1 (superseded)

- Machines: **AORUS** (Ubuntu 24.04, systemd-resolved / **tier-1 split-DNS**,
  coordinator) + **xps-17-9720** (LMDE trixie, **tier-5 direct `/etc/resolv.conf`
  takeover**, member).
- Tailscale: **on** (both nodes).
- Stopped after the **CRITICAL `create --subnet` data-plane corruption** finding
  below. Both machines fully wiped (binary, `/etc/torpedo`, `/var/log/torpedo`,
  systemd unit) and the run restarted from a clean slate as attempt 2, once
  SUBNET-014 + FW-001 landed to address the findings here.

Findings:

- [!] **DNS-001 delivery bug (confirmed).** On tier-5 xps the takeover, backup
  (`/etc/resolv.conf.before-torpedo`), and WARN log all worked, but `sudo torpedo up`
  printed only `already up` with **no** warning. Root cause: the daemon auto-activates
  the data plane at startup, so the takeover + warning happen there (logged), and the
  interactive `up` short-circuits before the `warnings` channel is populated. Fix:
  persist DNS mode/warning on the daemon; return it from `up` even on `already up`, and
  surface it in `torpedo status` (merges DNS-001-fix + DNS-002). **Still open** — see
  attempt 2, reproduced identically on a fresh install.
- [!] **Resolver IP is subnet-derived; docs are stale.** Magic DNS resolver is
  `10.88.100.53` (subnet-relative to the default subnet, by design — avoids Tailscale's
  `100.100.100.100`), NOT `100.100.100.53`. This plan and AGENTS.md still say
  `100.100.100.53`; correct them. Open question: does it move to `10.99.100.53` when xps
  joins the `10.99` network? (verify at the join / Magic DNS stage).
- [!] **resolv.conf re-assert storm at startup.** 3x `resolv.conf was overwritten;
  re-asserting torpedo DNS` within ~1s on xps (trample fight with NM/dhclient/Tailscale),
  settled after the NM `dns=none` drop-in applied. Watch for recurrence or sustained
  churn on other hosts.
- [~] **Tailscale-DNS interposition (works, note it).** torpedo captured Tailscale's
  `100.100.100.100` as its upstream and became the sole `nameserver`, preserving the
  `ts.<tailnet>` search domain. Non-`.ray` DNS still resolves (`github.com` 36 ms), so
  coexistence works, but torpedo is now the DNS chokepoint on a Tailscale host.
- [!] **CRITICAL — `create --subnet` corrupts the data plane.** `create --subnet
  10.99.0.0/16` derived the network roster/blob addresses in `10.99` but did NOT set
  the node's TUN/config subnet: on both nodes `config get subnet` = `<default>` and
  `ifconfig tun0` = `10.88.x`, while `status`/roster show `10.99.x`. Because peers are
  registered at roster addresses no TUN actually holds, **no IP forwarding works** —
  raw `ping` fails to BOTH the roster IP (`10.99.101.108`: no route, `tun0` is
  `10.88/16`) and the real TUN IP (`10.88.101.108`: no peer, registered at `10.99`).
  Only identity-based `torpedo ping` works (direct, 7 ms). Supported workaround:
  `torpedo config set subnet <cidr>` + `sudo torpedo restart` on EVERY node BEFORE
  create. Fix: `create --subnet` must set the node subnet (apply live or require a
  restart), or be rejected when it differs from the node's current subnet.
- [!] **`torpedo invite --hostname` not supported (doc mismatch).** `torpedo invite
  testnet --hostname xps-17-9720` errors `unexpected argument '--hostname'`; usage is
  just `torpedo invite <NETWORK>`. AGENTS.md (inherited from upstream) documents
  `--hostname`/`--expires`/`--qr`/`--reusable`/`list`/`revoke` on invite that this
  binary lacks. Audit AGENTS.md against the actual CLI and trim to what exists.

### Run 2026-07-07, attempt 2 (in progress)

- Both machines wiped clean first (binary, `/etc/torpedo`, `/var/log/torpedo`,
  systemd unit removed) and rebuilt from HEAD `6ffee52b` (includes FW-001 +
  SUBNET-014, the fixes prompted by attempt 1's findings above).
- Machines/roles unchanged: **AORUS** (tier-1 split-DNS, coordinator) +
  **xps-17-9720** (tier-5 direct takeover, member). Tailscale **on** on both.
- Subnet decision for this attempt: custom `10.99.0.0/16` (not the default),
  specifically to regression-test the SUBNET-014 fixes against the exact
  failure attempt 1 hit.
- Progress: **Stage 0 complete.** `torpedo up` run fresh on AORUS and xps,
  binary confirmed current at each step (`torpedo version` = `6ffee52b`
  initially, then rebuilt mid-run to `1c332832` after RENAME-011 landed —
  reinstalled + restarted on both nodes). Both nodes now on custom
  `10.99.0.0/16` (tun0: AORUS `10.99.49.50`, xps `10.99.3.155`), confirmed via
  `sudo torpedo config get subnet` on each. Tailscale confirmed still fully
  functional on both (`tailscale status` shows each node's own tailnet address
  active, peers visible) alongside the `10.99` overlay.
- Mid-run detour: found + fixed **RENAME-011** (41 leftover user-facing `ray`
  strings, incl. the `torpedo version` banner printing `ray` on one line and
  `torpedo` on the other) and the 1Password backup item title. Committed
  (`2dab79e`, `1c33283`), libspec-linked, pushed. Live-confirmed post-fix: the
  `config set subnet` hint now correctly says `sudo torpedo restart` (was
  `sudo ray restart`), and `torpedo version` prints `torpedo` consistently.

Findings so far:

- [!] **DNS-001 CLI-warning gap reproduces on a clean install (still open).**
  Identical to attempt 1: on fresh xps, the takeover fired correctly (backup
  `/etc/resolv.conf.before-torpedo` created, live file marked `# Added by
  torpedo`, and `journalctl` shows the WARN-level
  `took over /etc/resolv.conf directly … backup=/etc/resolv.conf.before-torpedo`
  at the exact timestamp of `torpedo up`), but the interactive `sudo torpedo up`
  output was only `torpedo service started. already up` — no warning text at
  all. Rules out "stale state from the old run" as an explanation; this is a
  real, deterministic gap in the current code, not fixed by SUBNET-014/FW-001
  (unrelated areas). Confirms the fix noted in attempt 1 is still needed.
- [!] **RENAME-011 — ~40 leftover user-facing `ray` strings (fixed mid-run).**
  Found while inspecting a `config set subnet` hint (`Run 'sudo ray restart'`).
  Broader sweep turned up CLI hints, error messages, an IPC message, the
  `apply --example` YAML, the version banner, and shell-completion
  registration all still hardcoding the pre-fork binary name. See
  `spec/design_spec.py`'s `RENAME-011` for the full list; fixed and verified
  live in this run (not just by `reconcile.py`).
- [!] **CRITICAL — DNS-003: mutual DNS forwarding loop with Tailscale on
  tier-5 hosts (top priority, NOT fixed yet).** Hit trying to run Stage 3's
  `torpedo join` on xps: failed immediately with "Service 'pkarr' failed".
  Root cause is not pkarr at all — it is total system DNS failure. torpedo's
  tier-5 `DirectResolvConf` takeover correctly captures Tailscale's
  `100.100.100.100` as upstream and rewrites `/etc/resolv.conf` to point at
  torpedo's own magic resolver (`10.99.100.53`) — but `tailscaled` ALSO
  watches `/etc/resolv.conf` to find its own forwarding upstream, so it then
  adopts torpedo's magic IP as *its* upstream. Every non-`.ray` query now
  bounces torpedo -> Tailscale -> torpedo forever, confirmed live in
  `journalctl -u tailscaled`: `dns udp query: waiting for response or error
  from [10.99.100.53]: context deadline exceeded`. `.ray` resolution itself is
  unaffected (instant, correct NXDOMAIN/SOA) — this is specifically the
  upstream-forwarding path. Full diagnosis, evidence, and why AORUS (tier-1)
  didn't show it: `spec/design_spec.py`'s `DNS-003` (`NoMutualDnsForwardingLoopWithTailscale`).
  This breaks the fork's headline coexistence promise on tier-5 hosts, which
  DNS-001 itself already documents as "the common case" for a minimal
  install — not an edge case. Blocking Stage 3+ on xps until fixed.
