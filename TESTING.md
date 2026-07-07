# torpedo — Manual Test Plan (Phase 7)

Live two-machine acceptance test for the fork. This is a **manual** checklist,
not part of `cargo test` / `reconcile.py`, so it does not gate the build. Run it
after producing a distributable binary to prove the fork works end to end and,
above all, that torpedo **coexists with Tailscale**.

Command reference: `AGENTS.md`. Requirement set: `spec/design_spec.py`.

## Prerequisites

| Machine | Role | Default hostname in this plan |
|---|---|---|
| **AORUS** (590I-AORUS-ULTRA) | Control node (coordinator) | `aorus` |
| **xps-17-9720** | Member (joins by invite) | `xps` |

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

## Stage 0 — Init (both machines)

**Goal:** daemon starts and the node gets an identity, with Tailscale running.

```bash
sudo torpedo up          # on BOTH machines
torpedo status           # confirm daemon is reachable
```

- [ ] Daemon starts cleanly on both (no "refusing to run next to Tailscale").
- [ ] `torpedo status` prints a contact id and "no networks".
- [ ] Tailscale is still fully functional on the node(s) where it runs.

## Stage 1 — First control node (AORUS)

**Goal:** create a closed network on a **non-default** subnet to prove
configurability and Tailscale-range avoidance.

```bash
torpedo create --name testnet --subnet 10.99.0.0/16
```

- [ ] A room id (network public key) is printed.

## Stage 2 — Check parameters (AORUS)

```bash
torpedo status --json
```

- [ ] Our mesh IPv4 is inside `10.99.0.0/16` (NOT `100.64.x.x`).
- [ ] Network role is `coordinator`; a `200::/7` IPv6 is assigned.

## Stage 3 — Enroll xps by invite

**Goal:** closed-network admission via a hostname-bound single-use invite.

```bash
# AORUS
torpedo invite testnet --hostname xps      # prints an invite code
# xps  (after its own `sudo torpedo up`)
torpedo join <invite-code> --hostname xps
```

- [ ] xps joins and reports success.

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
torpedo ping xps            # from AORUS: RTT + loss + direct/relay path
torpedo ping aorus          # from xps: the reverse direction
ping 10.99.<xps>            # raw ICMP through the TUN (default fw allows in icmp)
torpedo netcheck            # endpoint diagnostics on each node
```

- [ ] `torpedo ping` succeeds **both** directions with low loss.
- [ ] Raw `ping` to the peer's mesh IP works.
- [ ] (Cross-NAT run) path is `direct` after hole-punching, or `relay` as fallback.

## Stage 7 — Firewall

**Goal:** prove default-deny inbound, then an allow rule, before SSH/send depend
on it. Run a listener on the target and probe it from the peer.

```bash
# xps: start a throwaway TCP listener on 8080
python3 -m http.server 8080

# AORUS: should FAIL under the default inbound-deny
curl --max-time 5 http://xps.ray:8080/

# xps: allow it from aorus, then re-test from AORUS (should succeed)
torpedo firewall add in allow -p tcp -P 8080 --peer aorus
torpedo firewall show
```

- [ ] The probe is **blocked** before the allow rule (default inbound-deny holds).
- [ ] The probe **succeeds** after `firewall add in allow -p tcp -P 8080 --peer aorus`.
- [ ] `firewall show` lists the new rule at the front.

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
cat /etc/resolv.conf                        # "# Added by torpedo", nameserver 100.100.100.53
ping github.com                             # captured upstreams forward normal DNS
sudo torpedo uninstall
cat /etc/resolv.conf                        # restored to the pre-torpedo original
ping github.com                             # resolves after restore
```

- [ ] `torpedo up` shows the **DNS-001** warning naming
      `/etc/resolv.conf.before-torpedo` and the restore command.
- [ ] Backup exists while up; the live file carries the `# Added by torpedo`
      marker and points at `100.100.100.53`.
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
cat /etc/resolv.conf                           # "# Added by torpedo", nameserver 100.100.100.53
ping github.com                                # captured upstreams still forward normal DNS
sudo torpedo uninstall
diff /etc/resolv.conf /tmp/resolv.conf.orig    # empty ⇒ restored to the pre-torpedo original
ping github.com                                # resolves after restore
```

- [ ] `torpedo up` prints the DNS-001 warning naming `/etc/resolv.conf.before-torpedo`
      and the restore command.
- [ ] Backup exists while up; the live file carries the `# Added by torpedo` marker
      and points at `100.100.100.53`.
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

- Run: _____  Machines: _____  Tailscale: _____  Placement: _____
- Findings:
