# tetron - e2e test plan

Manual test plan for the minimal torpedo fork (tetron). This is a **manual**
checklist, not part of `cargo test` / `reconcile.py`, so it does not gate the
build. Run it after producing a distributable binary to prove the fork works
end to end.

## What tetron does (and does not do)

tetron provides one thing: a P2P mesh VPN with stable IPv4/IPv6 addresses
derived from cryptographic identity. There is no userspace firewall, no Magic
DNS, no file transfer, no embedded SSH, no self-update, no diagnostic tools, no hostname renaming, no ephemeral mode, no declarative
apply, and no permissionless ("open") network creation. Packet filtering is
the host firewall's job; name resolution is `/etc/hosts`'s job; file copying
is `scp`/`rsync`'s job; remote shells are `sshd`'s job.

## Prerequisites

### Available machines

| Machine | Location | Default hostname |
|---|---|---|
| **node-1** (node-1) | LAN 1 (this desk) | `node-1` |
| **node-2** (node-2) | LAN 2 (remote) | `node-2` |
| **node-3** / node-3 | LAN 3 (remote) | `node-3` |

Stages 1-9 work with 2 machines (node-1 + node-2). Stages 10+ (multi-coordinator)
need all 3 machines on **different LANs** so NAT traversal, relay fallback,
and DHT propagation latency are exercised realistically.

### Test overview

| Stage | What | Machines | Topology | Needs cross-LAN? |
|---|---|---|---|---|
| 1 | Daemon start | node-1 + node-2 | Same or different | No |
| 2 | Create network | node-1 alone | N/A | No |
| 3 | Join via invite | node-1 + node-2 | Any | No |
| 4 | Ping connectivity | node-1 + node-2 | Direct preferred | No |
| 5 | Restart stability | node-1 + node-2 | Any | No |
| 6 | Leave / rejoin | node-1 + node-2 | Any | No |
| 7 | Kick | node-1 + node-2 | Any | No |
| 8 | Down / up | node-1 + node-2 | Any | No |
| 9 | Invite burn | node-1 + node-2 | Any | No |
| 10 | AdminGrant + B admits C | A, B, C | Cross-LAN | Yes |
| 11 | Co-coordinator restart | A, B, C | Cross-LAN | Yes |
| 12 | Original coordinator restart | A, B, C | Cross-LAN | Yes |
| 13 | Control listener respawn | A, B, C | Cross-LAN | Yes |
| 14 | Admin list divergence | A, B, C | Any | No |
| 15 | Concurrent invites | A, B, C (D opt) | Cross-LAN | Yes |
| 16 | Kick barrier | A, B, C | Any | No |
| 17 | All coordinators offline | A, B, C | Cross-LAN | Yes |
| 18 | Peer address cache | A, B, C | Cross-LAN | Yes |
| 19 | Nuke cleanup | A, B, C | Any | No |

**Why cross-LAN matters for multi-coordinator tests:**
- Same-LAN QUIC connects directly every time — you never exercise relay
  fallback or STUN, which is the actual path across the internet
- DHT (pkarr) propagation latency affects how fast coordinators see blob
  updates; on LAN it is near-instant, across internet the 30-60s poll
  window is real
- Coordinator crash on a remote machine is a genuine dropped connection,
  not a local `sudo systemctl stop` — the reconnect loop and backoff
  behave differently

### Build and deploy

Build on node-1, copy binary to all machines:

```bash
cargo build --release
sudo install -m 755 target/release/tetron /usr/local/bin/tetron
scp target/release/tetron node-2:/tmp/
# on node-2:
sudo install -m 755 /tmp/tetron /usr/local/bin/tetron
# repeat for node-3
scp target/release/tetron node-3:/tmp/
# on node-3:
sudo install -m 755 /tmp/tetron /usr/local/bin/tetron
```

## Stage 1 - Daemon start (both machines)

```bash
sudo tetron up
tetron status
```

- [ ] Daemon starts without error.
- [ ] `tetron status` shows daemon reachable, no networks.
- [ ] TUN interface exists: `ip addr show tun0` shows an IPv4 in the default
      `10.88.0.0/16` range and an IPv6 in `200::/7`.
- [ ] Tailscale is unaffected if running (`tailscale status` still works).
- [ ] No `/etc/resolv.conf` changes (tetron has no Magic DNS).

## Stage 2 - Create a network (node-1)

```bash
tetron create --name testnet --hostname node-1
```

- [ ] A room id (network public key) and an invite key are printed.
- [ ] `tetron status` shows 1 member (self), role `coordinator`.
- [ ] Network subnet is `10.88.0.0/24` by default.

## Stage 3 - Join via invite key (node-2)

tetron is invite-only (LIVE-001). Bare room-id joins are denied. A coordinator
mints an invite key; the joiner presents it for auto-admission.

```bash
# node-1: mint an invite key
INVITE_KEY=$(tetron invite testnet create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")
echo $INVITE_KEY

# node-2: join using the invite key
tetron join "$INVITE_KEY" --hostname node-2
```

- [ ] Joiner is auto-admitted (no separate accept step needed).
- [ ] node-2 shows 2 members, its own mesh IP in `10.88.x.x`, peer IP for node-1.
- [ ] node-1's `tetron status` also shows 2 members.

## Stage 4 - Connectivity test

```bash
# raw ICMP over the TUN (the only ping tetron supports - no control-plane ping)
ping -c 3 <peer-mesh-ip>
```

- [ ] ICMP ping succeeds both directions (node-1 -> node-2, node-2 -> node-1).
- [ ] Round-trip time indicates `direct` (same LAN) or `relay` (cross-NAT).

## Stage 5 - Restart stability

```bash
sudo tetron restart       # on either node
tetron status
```

- [ ] Node rejoins automatically with the same mesh IP (stable addressing).
- [ ] Both sides show 2 members after restart.
- [ ] Ping still works.

## Stage 6 - Leave and rejoin

```bash
# node-2: leave the network
tetron leave testnet
tetron status      # shows 0 networks
```

- [ ] node-2 shows no networks.
- [ ] node-1 shows 1 member (self only).

```bash
# node-1: mint a fresh invite for the rejoining member
INVITE_KEY=$(tetron invite testnet create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")
```

```bash
# node-2: rejoin using the new invite key
tetron join "$INVITE_KEY" --hostname node-2
```

- [ ] Rejoin works with a fresh invite key.
- [ ] A new mesh IP is assigned (different `collision_index`).

## Stage 7 - Kick

```bash
# node-1: kick node-2
tetron kick testnet node-2
tetron status
```

- [ ] node-1 shows 1 member (self only).
- [ ] node-2's connection is torn down (status shows 0 or connection lost).

## Stage 8 - Down/up (data plane toggle)

```bash
tetron down
tetron status    # daemon still reachable, "standby"
```

- [ ] TUN is down: `ip link show tun0` shows `DOWN` or absent.
- [ ] Daemon is still connected to peers (control plane alive).

```bash
tetron up
tetron status    # data plane restored
ping -c 3 <peer-ip>
```

- [ ] TUN is back up with the same IP.
- [ ] Ping works again.

## Stage 9 - Invite key admission (single-use)

Requires the invite key feature (Phase 1-4). The coordinator mints invite keys;
a joiner auto-admitted by presenting one — no approval queue.

```bash
# node-1: mint an invite key
tetron invite testnet create
# prints invite_key and invite_id
```

- [ ] `tetron invite testnet create` returns an invite key and invite id.
- [ ] `tetron invite testnet list` shows the invite as `active`.

```bash
# node-2: join using invite key (replaces the old room-id + accept flow)
tetron join <invite-key>
```

- [ ] Joiner is auto-admitted (no `tetron accept` needed).
- [ ] `tetron status` on both sides shows 2 members.

```bash
# Verify connectivity
ping -c 3 <peer-mesh-ip>
```

- [ ] Ping works both directions (node-1 -> node-2, node-2 -> node-1).

```bash
# Verify the invite was burned (single-use)
tetron invite testnet list
```

- [ ] The used invite shows status `used`.
- [ ] A second `tetron join` with the same key is rejected.

```bash
# Verify mint+list output with --json
tetron invite testnet create --json
tetron invite testnet list --json
```

- [ ] `--json` output is valid JSON with expected fields.

## Stage 10 - AdminGrant (promote B to co-coordinator, 3 machines)

Requires a third machine (node-3/node-3). node-1 = A, node-2 = B, third machine = C.

Setup: A creates a network, B joins via invite key.

```bash
# A: create network
tetron create --name multicoord --hostname node-1
# save room id and invite key from output

# B: join via invite
tetron join <invite-key> --hostname node-2
```

- [ ] A and B both see 2 members in `tetron status`.

```bash
# A: promote B to co-coordinator
# Get B's short-id from `tetron status` on A
tetron admin multicoord add <B-short-id>
```

- [ ] `tetron admin multicoord add` succeeds with an OK message.
- [ ] `tetron admin multicoord list` on A shows B as admin.

```bash
# B: verify it is now a coordinator
tetron status --json | python3 -c "import sys,json; d=json.load(sys.stdin); print([n for n in d['networks'] if n['name']=='multicoord'][0]['role'])"
```

- [ ] B's role shows `coordinator` (not `member`).

```bash
# B: mint an invite (only coordinators can — verifies B has the network key)
tetron invite multicoord create --json
```

- [ ] B can mint invites (proof it holds the network key).

```bash
# C: join using invite minted by B (not A)
tetron join <B-invite-key> --hostname usbos
```

- [ ] C is auto-admitted (no separate accept needed).
- [ ] All 3 nodes show 3 members in `tetron status`.
- [ ] Ping works between all pairs (A<->B, A<->C, B<->C) in both directions.
- [ ] C's mesh IP is within the network's subnet.

## Stage 11 - Co-coordinator restart persistence

```bash
# B: restart daemon
sudo tetron restart
sleep 5
tetron status
```

- [ ] B reconnects with the same mesh IP (stable addressing).
- [ ] B still shows role `coordinator` in status (network key persisted to config).

```bash
# B: mint an invite after restart
tetron invite multicoord create
```

- [ ] Invite minting works after restart (key survived daemon restart).

```bash
# C: connectivity is unaffected by B's restart
ping -c 3 <A-mesh-IP>
```

- [ ] Ping from C to A still works after B's restart (A was online throughout).

## Stage 12 - Original coordinator restart, co-coordinator independence

```bash
# A: restart the original coordinator
sudo tetron restart
sleep 5
tetron status
```

- [ ] A reconnects with the same mesh IP.
- [ ] B and C show 3 members (A returned, no one lost).

```bash
# B: still functions as coordinator after A's restart
tetron invite multicoord create --json
```

- [ ] B can still mint invites (key not affected by A's restart).
- [ ] Ping works all directions.

## Stage 13 - Control listener respawn (AdminGrant survives coordinator restart)

Tests the fix from commit cb916df.

```bash
# A: restart, then promote C to co-coordinator while B is also online
sudo tetron restart
sleep 5

# A: get C's short-id
tetron status
tetron admin multicoord add <C-short-id>
```

- [ ] A's `admin add` to promote C succeeds.
- [ ] C shows role `coordinator` in status.
- [ ] C can mint invites.

```bash
# C: verify the grant works
tetron invite multicoord create
tetron admin multicoord list
```

- [ ] C can mint invites.
- [ ] C's `admin list` shows itself (local record).

```bash
# Now test the reverse: promote from B to A (B was promoted earlier, survived restarts)
tetron admin multicoord add <A-short-id>
```

- [ ] If A is not already an admin in B's local records, B can still grant to A (this may fail if A is already a key holder — the daemon may reject the add for an existing key holder). Expected result: either success (key re-sent) or a graceful error "already a coordinator".

## Stage 14 - Coordinators with different local admin lists

```bash
# A: its admin list may show only B (from the original grant)
tetron admin multicoord list
```

- [ ] A's list shows the admins it has granted (at least B).
- [ ] B's list shows the admins it has granted (at least C from Stage 13 if B did it, or empty).
- [ ] C's list shows the admins it has granted (at least A from Stage 13, or empty).

Adjacent observation: each node's `admins` list is local-only. Node A may not know about C's promotion if B promoted C. This is **by design** — authority comes from holding the key, not from the `admins` list. `admin list` is best-effort local history.

## Stage 15 - Concurrent join via invites minted by different coordinators

```bash
# A: mint an invite
INVITE_A=$(tetron invite multicoord create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")

# B: mint an invite
INVITE_B=$(ssh node-2 tetron invite multicoord create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")
```

If a third node is available (use a throwaway identity), join twice in quick succession:

```bash
# C: join using both invites sequentially
tetron join "$INVITE_A"   # first join gets auto-admitted
# wait for status to settle
tetron status
```

- [ ] C joins successfully using an invite from A.
- [ ] C can also join using an invite from B (may rejoin an existing network with the same identity, which should work as reconnect).

If a fourth machine D is available:

```bash
# D: join using invite from B
tetron join "$INVITE_B"
```

- [ ] D joins successfully using an invite minted by B.
- [ ] All nodes see 4 members (or 3 + D, depending on whether C stayed).

## Stage 16 - Coordinator kick barrier

```bash
# A: try to kick B (a coordinator)
tetron kick multicoord node-2
```

- [ ] Kick is rejected with an error message: "is a coordinator (holds the network key); kicking can not remove its access. Revoke the key instead."

```bash
# A: kick C instead (if C is a non-coordinator member)
tetron kick multicoord usbos
```

- [ ] If C is a non-coordinator member, kick succeeds: C's connection is torn down.
- [ ] A and B show 2 members (C removed).

## Stage 17 - All coordinators offline, join blocked

```bash
# A: take all coordinators offline
sudo systemctl stop tetron  # on A
ssh node-2 sudo systemctl stop tetron  # on B
```

- [ ] Network is unreachable to members (pending invites can not be validated, no coordinator to admit).

```bash
# C: try to join (no coordinator online)
tetron join <any-invite-key>
```

- [ ] Join attempt fails with a connection error (no coordinator to dial).

```bash
# A: bring one coordinator back online
sudo systemctl start tetron
sleep 5
```

- [ ] C's existing connection (if it was a member before Stage 17) reconnects automatically.
- [ ] If C was not yet a member, joining now works.

## Stage 18 - Daemon restart with stale peer address cache

```bash
# A: restart daemon (caches peer addresses to disk)
sudo tetron restart
sleep 5
tetron status
```

- [ ] A reconnects to all known peers using the cached addresses, without needing DHT discovery.

```bash
# Verify connectivity
ping -c 3 <B-mesh-IP>
ping -c 3 <C-mesh-IP>
```

- [ ] Ping succeeds to all peers (cache works).

## Stage 19 - Network nuke cleans up everything

```bash
# A: nuke the network (requires being online)
tetron nuke multicoord
tetron status
```

- [ ] Network is removed from A's status.
- [ ] DHT record for the network is overwritten with an empty record.

```bash
# B: verify the network is gone
tetron status
```

- [ ] B sees the network removed (or an error on reconnect attempt).
- [ ] Clean up: remove config directories on all machines if done testing.

## Results log

### Run 2026-07-13 (Phase 6 verification)

- Machines: node-1 (coordinator) + node-2 (member).
- Binary: tetron at `bf046e1`+ (post all MINIMAL removals, RENAME-M01, TOR-M01).
- Subnet: default `10.88.0.0/16`.

Results:

- [x] Stage 1 - Daemon starts, TUN has `10.88.x.x` IPv4 + `200::/7` IPv6.
- [x] Stage 2 - `torpedo create --name testnet --hostname node-1` prints room id.
- [x] Stage 3 - `torpedo join <room-id> --hostname node-2` shows pending;
      `torpedo accept` admits member.
- [x] Stage 4 - `ping -c 3 10.88.121.148` succeeds both directions.
- [x] Stage 5 - `sudo torpedo restart` preserves mesh IP.
- [x] Stage 6 - `torpedo leave`/rejoin works.
- [x] Stage 7 - `torpedo kick testnet node-2` removes member mesh-wide.
- [x] Stage 8 - `torpedo down`/`up` toggles data plane, ping restored.

### Run 2026-07-13 (Invite key admission, Phase 1-4)

- Machines: node-1 (coordinator) + node-2 (member).
- Binary: tetron at `f8ec05f` (invite key admission Phases 1-4).
- Subnet: default `10.88.0.0/16`.
- Old `torpedo` stopped and removed on both machines before starting.

Commands used:

```bash
# node-1: stop/remove old torpedo
sudo systemctl stop torpedo
sudo systemctl disable torpedo
sudo rm /usr/local/bin/torpedo
sudo rm /etc/systemd/system/torpedo.service

# node-1: build release binary
cargo build --release

# node-1: install tetron service
sudo cp target/release/tetron /usr/local/bin/tetron
sudo tetron install

# node-2: stop/remove old torpedo
ssh node-2 "sudo systemctl stop torpedo && sudo systemctl disable torpedo && sudo rm /usr/local/bin/torpedo && sudo rm /etc/systemd/system/torpedo.service"

# copy binary and install on node-2
scp target/release/tetron node-2:/tmp/tetron
ssh node-2 "sudo cp /tmp/tetron /usr/local/bin/tetron && sudo tetron install"

# node-1: create network (auto-mints first invite)
tetron create --name testnet

# node-1: mint another invite
tetron invite testnet create

# node-2: join with invite key (extracted via --json)
INVITE_KEY=$(tetron invite testnet create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")
ssh node-2 "tetron join '$INVITE_KEY'"

# verify status and connectivity
tetron status
ping -c 3 10.88.0.160      # node-1 -> node-2
ssh node-2 "ping -c 3 10.88.0.232"   # node-2 -> node-1

# verify invite was burned
tetron invite testnet list
```

Results:

- [x] Stage 9 - `tetron invite testnet create` prints invite key.
- [x] `tetron invite testnet list` shows invites with active/used status.
- [x] `tetron join <invite-key>` auto-admits (no accept needed).
- [x] Ping both ways: 0% loss, ~5ms RTT.
- [x] Used invite shows `used` in listing (single-use enforced).
- [x] `--json` output works for create and list.

### Run 2026-07-14 (3-machine invite key e2e, node-3 as third node)

- Binary: tetron at `f8ec05f`.
- Machines: node-1 (coordinator) + node-2 (member) + node-3 (member, remote).
- node-3 had stale `tun0` from old torpedo causing route conflict between two
  `10.88.0.0/24` entries. After removing the old TUN + torpedo, ping and SSH
  worked across all 3 nodes over direct connections.
- SSH from node-3 → node-1 via mesh IP: confirmed working.
