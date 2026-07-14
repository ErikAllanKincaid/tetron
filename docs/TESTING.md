# tetron - e2e test plan

Manual test plan for the minimal torpedo fork (tetron). This is a **manual**
checklist, not part of `cargo test` / `reconcile.py`, so it does not gate the
build. Run it after producing a distributable binary to prove the fork works
end to end.

## What tetron does (and does not do)

tetron provides one thing: a P2P mesh VPN with stable IPv4/IPv6 addresses
derived from cryptographic identity. There is no userspace firewall, no Magic
DNS, no file transfer, no embedded SSH, no self-update, no diagnostic tools,
no invite minting, no hostname renaming, no ephemeral mode, no declarative
apply, and no permissionless ("open") network creation. Packet filtering is
the host firewall's job; name resolution is `/etc/hosts`'s job; file copying
is `scp`/`rsync`'s job; remote shells are `sshd`'s job.

## Prerequisites

| Machine | Role | Default hostname |
|---|---|---|
| **AORUS** (590I-AORUS-ULTRA) | Coordinator | `aorus` |
| **xps-17** (xps-17-9720) | Member | `xps` |

Build on AORUS, copy binary to xps:

```bash
cargo build --release
sudo install -m 755 target/release/torpedo /usr/local/bin/torpedo
scp target/release/torpedo xps-17:/tmp/
# on xps:
sudo install -m 755 /tmp/torpedo /usr/local/bin/torpedo
```

## Stage 1 - Daemon start (both machines)

```bash
sudo torpedo up
torpedo status
```

- [ ] Daemon starts without error.
- [ ] `torpedo status` shows daemon reachable, no networks.
- [ ] TUN interface exists: `ip addr show tun0` shows an IPv4 in the default
      `10.88.0.0/16` range and an IPv6 in `200::/7`.
- [ ] Tailscale is unaffected if running (`tailscale status` still works).
- [ ] No `/etc/resolv.conf` changes (tetron has no Magic DNS).

## Stage 2 - Create a network (AORUS)

```bash
torpedo create --name testnet --hostname aorus
```

- [ ] A room id (network public key) is printed.
- [ ] `torpedo status` shows 1 member (self), role `coordinator`.
- [ ] Network subnet is `10.88.0.0/16` by default.

## Stage 3 - Join via live approval (xps)

```bash
# xps: join using the room id from Stage 2
torpedo join <room-id> --hostname xps
torpedo status    # shows "pending: 1 peer waiting"
```

- [ ] xps shows "pending" status after join attempt (tetron networks are always
      closed - admission requires live approval).

```bash
# AORUS: approve the pending join
torpedo requests testnet
torpedo accept testnet <short-id>
```

- [ ] AORUS shows the pending request via `torpedo requests`.
- [ ] `torpedo accept` admits the member.

```bash
# xps: re-dial after approval
torpedo status
```

- [ ] xps shows 2 members, its own mesh IP in `10.88.x.x`, peer IP for AORUS.
- [ ] AORUS's `torpedo status` also shows 2 members.

## Stage 4 - Connectivity test

```bash
# raw ICMP over the TUN (the only ping tetron supports - no control-plane ping)
ping -c 3 <peer-mesh-ip>
```

- [ ] ICMP ping succeeds both directions (AORUS -> xps, xps -> AORUS).
- [ ] Round-trip time indicates `direct` (same LAN) or `relay` (cross-NAT).

## Stage 5 - Restart stability

```bash
sudo torpedo restart       # on either node
torpedo status
```

- [ ] Node rejoins automatically with the same mesh IP (stable addressing).
- [ ] Both sides show 2 members after restart.
- [ ] Ping still works.

## Stage 6 - Leave and rejoin

```bash
# xps: leave the network
torpedo leave testnet
torpedo status      # shows 0 networks
```

- [ ] xps shows no networks.
- [ ] AORUS shows 1 member (self only).

```bash
# xps: rejoin (needs a new room id + accept, or reuse the old if not nuked)
torpedo join <room-id> --hostname xps
torpedo status      # pending again
# AORUS:
torpedo accept testnet <short-id>
```

- [ ] Rejoin works with the same room id.
- [ ] A new mesh IP is assigned (different `collision_index`).

## Stage 7 - Kick

```bash
# AORUS: kick xps
torpedo kick testnet xps
torpedo status
```

- [ ] AORUS shows 1 member (self only).
- [ ] xps's connection is torn down (status shows 0 or connection lost).

## Stage 8 - Down/up (data plane toggle)

```bash
torpedo down
torpedo status    # daemon still reachable, "standby"
```

- [ ] TUN is down: `ip link show tun0` shows `DOWN` or absent.
- [ ] Daemon is still connected to peers (control plane alive).

```bash
torpedo up
torpedo status    # data plane restored
ping -c 3 <peer-ip>
```

- [ ] TUN is back up with the same IP.
- [ ] Ping works again.

## Stage 9 - Invite key admission (single-use)

Requires the invite key feature (Phase 1-4). The coordinator mints invite keys;
a joiner auto-admitted by presenting one — no approval queue.

```bash
# AORUS: mint an invite key
tetron invite testnet create
# prints invite_key and invite_id
```

- [ ] `tetron invite testnet create` returns an invite key and invite id.
- [ ] `tetron invite testnet list` shows the invite as `active`.

```bash
# xps: join using invite key (replaces the old room-id + accept flow)
tetron join <invite-key>
```

- [ ] Joiner is auto-admitted (no `tetron accept` needed).
- [ ] `tetron status` on both sides shows 2 members.

```bash
# Verify connectivity
ping -c 3 <peer-mesh-ip>
```

- [ ] Ping works both directions (AORUS -> xps, xps -> AORUS).

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

## Results log

### Run 2026-07-13 (Phase 6 verification)

- Machines: AORUS (coordinator) + xps-17 (member).
- Binary: tetron at `bf046e1`+ (post all MINIMAL removals, RENAME-M01, TOR-M01).
- Subnet: default `10.88.0.0/16`.

Results:

- [x] Stage 1 - Daemon starts, TUN has `10.88.x.x` IPv4 + `200::/7` IPv6.
- [x] Stage 2 - `torpedo create --name testnet --hostname aorus` prints room id.
- [x] Stage 3 - `torpedo join <room-id> --hostname xps` shows pending;
      `torpedo accept` admits member.
- [x] Stage 4 - `ping -c 3 10.88.121.148` succeeds both directions.
- [x] Stage 5 - `sudo torpedo restart` preserves mesh IP.
- [x] Stage 6 - `torpedo leave`/rejoin works.
- [x] Stage 7 - `torpedo kick testnet xps` removes member mesh-wide.
- [x] Stage 8 - `torpedo down`/`up` toggles data plane, ping restored.

### Run 2026-07-13 (Invite key admission, Phase 1-4)

- Machines: AORUS (coordinator) + xps-17-9720 (member).
- Binary: tetron at `f8ec05f` (invite key admission Phases 1-4).
- Subnet: default `10.88.0.0/16`.
- Old `torpedo` stopped and removed on both machines before starting.

Commands used:

```bash
# AORUS: stop/remove old torpedo
sudo systemctl stop torpedo
sudo systemctl disable torpedo
sudo rm /usr/local/bin/torpedo
sudo rm /etc/systemd/system/torpedo.service

# AORUS: build release binary
cargo build --release

# AORUS: install tetron service
sudo cp target/release/tetron /usr/local/bin/tetron
sudo tetron install

# xps: stop/remove old torpedo
ssh xps-17-9720 "sudo systemctl stop torpedo && sudo systemctl disable torpedo && sudo rm /usr/local/bin/torpedo && sudo rm /etc/systemd/system/torpedo.service"

# copy binary and install on xps
scp target/release/tetron xps-17-9720:/tmp/tetron
ssh xps-17-9720 "sudo cp /tmp/tetron /usr/local/bin/tetron && sudo tetron install"

# AORUS: create network (auto-mints first invite)
tetron create --name testnet

# AORUS: mint another invite
tetron invite testnet create

# xps: join with invite key (extracted via --json)
INVITE_KEY=$(tetron invite testnet create --json | python3 -c "import sys,json; print(json.load(sys.stdin)['invite_key'])")
ssh xps-17-9720 "tetron join '$INVITE_KEY'"

# verify status and connectivity
tetron status
ping -c 3 10.88.165.160      # AORUS -> xps
ssh xps-17-9720 "ping -c 3 10.88.108.232"   # xps -> AORUS

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

### Run 2026-07-14 (3-machine invite key e2e, SB-OS as third node)

- Binary: tetron at `f8ec05f`.
- Machines: AORUS (coordinator) + xps-17-9720 (member) + SB-OS (member, remote).
- SB-OS had stale `tun0` from old torpedo causing route conflict between two
  `10.88.0.0/16` entries. After removing the old TUN + torpedo, ping and SSH
  worked across all 3 nodes over direct connections.
- SSH from SB-OS → AORUS via mesh IP: confirmed working.
