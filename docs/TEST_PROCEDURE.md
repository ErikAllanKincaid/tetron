# tetron Test Procedure

> Record of the first full end-to-end test session. Each command is tagged
> with the machine it runs on. Use this as a script skeleton for automated
> testing.

## Environment

| Alias | Hostname | IP | Role | Arch |
|-------|----------|----|------|------|
| `aorus` | 590I-AORUS-ULTRA.local | 192.168.1.43 | coordinator | x86_64 |
| `xps` | xps-17-9720.local | 192.168.1.111 | member | x86_64 |
| `x10sra` | X10SRA.local | 192.168.1.113 | member | x86_64 |
| `xeon40` | xeon40 (noisebridge) | 100.64.0.12 (via `xps` jumphost) | member | x86_64 |

Build machine: `aorus` (has cargo/rustup).

All machines run `tetron` on a configurable overlay subnet (`10.55.55.0/24`
for this session). The default subnet is `10.88.0.0/16`.

---

## Phase 0: Build

Run on: **aorus**

```bash
# From repo root
cargo build --release
# Binary at target/release/tetron
```

---

## Phase 0.5: Stale Interface Pre-check (per-machine)

**Before** any cleanup, check for leftover TUN devices from previous runs.
After repeated install/restart cycles, stale `tun0`, `tun1`, etc. may linger
with the wrong IP, causing confusion when `ip addr show tun0` shows a
different address than expected.

```bash
# List all TUN interfaces
ip -o link show | grep -oP '^\d+: tun\d+' | awk '{print $2}'

# For each tunX found, inspect its IPv4 address
for dev in $(ip -o link show | grep -oP 'tun\d+'); do
    ip -4 addr show "$dev" | grep inet
done
```

If multiple TUN interfaces exist, note them in the test log. The clean
install procedure (Phase 1) should remove them all.

---

## Phase 1: Clean + Install (per-machine)

Every machine starts from a clean slate: remove the old binary, config,
service unit, and all TUN devices.

### 1a. Leave any active networks

```bash
# List existing networks
tetron status

# Leave each network by name (or nuke from coordinator)
tetron leave <network-name>    # omit if fresh install
```

> **UX note**: `tetron leave` accepts network name only, not the network
> key. If you only have the invite code or room id, scan
> `/etc/tetron/networks/*.toml` to find the canonical name.

### 1b. Stop, uninstall, and remove all state

```bash
sudo tetron down                 # standby data plane
sudo tetron uninstall            # remove systemd unit
sudo rm -rf /etc/tetron /var/run/tetron /var/log/tetron
sudo rm -f /usr/local/bin/tetron
```

### 1c. Remove stale TUN devices

After removing the config, the old TUN device(s) may still exist if the
service was not running at the right moment to tear them down.

```bash
# List remaining tun devices
ip -o link show | grep -oP 'tun\d+' || echo "no tun devices"

# Delete each one
for dev in $(ip -o link show | grep -oP 'tun\d+'); do
    sudo ip link delete "$dev"
done

# Verify all gone
ip -o link show | grep -oP 'tun\d+' || echo "clean: no tun devices"
```

### 1d. Install fresh

```bash
# Copy new binary (via scp for remote machines)
sudo cp <path-to-tetron-binary> /usr/local/bin/tetron

# Run the built-in installer
sudo /usr/local/bin/tetron install

# Verify
tetron --version               # e.g. 0.1.5 (fa29ef9)
tetron status                  # expect: (no active networks)
```

---

## Phase 2: Test SUBNET-BUG-001 (subnet mismatch rejection)

This phase verifies that joining a network whose subnet differs from the
node's local subnet is rejected with a clear error message, instead of
silently creating a TUN with the wrong IP.

### 2a. Coordinator creates network with custom subnet

Run on: **aorus**

```bash
# Create a network on the custom subnet
tetron create --subnet 10.55.55.0/24 --name test-tetronnet

# Expected: prints invite code + room id + subnet warning
#   created test-tetronnet
#     address  10.55.55.73  ·  b109…4cd6
#     join <INVITE_CODE>
#   ⚠ subnet 10.55.55.0/24 takes effect after `sudo tetron restart`

# The subnet warning is SUBNET-014: the TUN was built at bootstrap with
# the old default subnet; the custom subnet is persisted and takes effect
# after restart. Run the restart now so the coordinator's TUN is correct.
sudo tetron restart
```

Record the invite code printed by `tetron create`. The invite code is a
single-use key that any coordinator validates against the signed GroupBlob.

```bash
# Verify coordinator is on the correct subnet
ip addr show tun0
# Expected: inet 10.55.55.xx peer 10.55.55.0/24 ...

# List invites (should show the auto-minted one)
tetron invite test-tetronnet list
```

If you need additional invites (one per joining node), mint them:

```bash
tetron invite test-tetronnet create
```

### 2b. Verify SUBNET-BUG-001: mismatch rejection

Run on: **each joining machine** (before setting the custom subnet)

With the node's default subnet (`10.88.0.0/16`, TUN IP in that range),
attempting to join a network on `10.55.55.0/24` must fail fast.

```bash
# Attempt join with default subnet -- EXPECTED TO FAIL
tetron join <INVITE_CODE> --hostname <HOSTNAME>
```

**Expected output:**

```
Error: node subnet is 10.88.0.0/16 but network 'test-tetronnet' uses 10.55.55.0/24;
run `sudo tetron config set subnet 10.55.55.0/24 && sudo tetron restart` and try joining again
```

Record this in the test log as **SUBNET-BUG-001 PASS**.

> **If the join succeeds anyway**, the fix was not deployed or the binary
> is stale. Verify the binary version (`tetron --version`) includes commit
> `fa29ef9`.

### 2c. Set subnet and restart

Run on: **each joining machine**

```bash
sudo tetron config set subnet 10.55.55.0/24
sudo tetron restart

# Verify TUN is now on the correct subnet
ip addr show tun0
# Expected: inet 10.55.55.xx/24 scope global tun0
```

### 2d. Join successfully

Run on: **each joining machine**

```bash
tetron join <INVITE_CODE> --hostname <HOSTNAME>
# Expected: prints "joined <network>" with assigned mesh IP
```

### 2e. Coordinator checks all members

Run on: **aorus**

```bash
tetron status
# Expected: 4 members total
```

---

## Phase 3: Verify TUN interface consistency

After all nodes are joined, verify that each node has exactly one TUN
device with the correct subnet IP.

```bash
# Count TUN devices
ip -o link show | grep -c 'tun[0-9]'   # expected: 1

# Verify the device name and IP
ip addr show tun0 | grep inet
# Expected: inet 10.55.55.xx/24 scope global tun0

# If multiple tun devices exist, log them:
ip -o link show | grep -oP 'tun\d+'
```

Multiple TUN devices indicate a cleanup bug (leftover interfaces from
a previous daemon instance not torn down on uninstall/restart).

---

## Phase 4: Feature Tests

### 4a. Cross-member connectivity (ping mesh IPs)

```bash
# On each machine, get mesh IP from `tetron status`
tetron status | grep -oP '\d+\.\d+\.\d+\.\d+'

# Ping the other machines' mesh IPs (10.55.55.x)
ping -c 3 10.55.55.<other-ip>
```

### 4b. Invite lifecycle

Run on: **aorus** (coordinator)

```bash
tetron invite test-tetronnet list
tetron invite test-tetronnet create --expires +7d
tetron invite test-tetronnet list         # should show new invite
tetron invite test-tetronnet revoke <CODE>
tetron invite test-tetronnet list         # should show revoked
```

### 4c. Admin (co-coordinator grant)

Run on: **aorus**

```bash
tetron admin test-tetronnet list          # just the creator
tetron admin test-tetronnet add <SHORT_ID>
tetron admin test-tetronnet list          # two admins now
```

### 4d. up/down (data plane toggle)

Run on: any member (e.g. **xps**)

```bash
tetron down                           # data plane off, still online
tetron status                         # shows "down"
tetron up                             # data plane back
tetron status                         # shows "up"
```

### 4e. Graceful leave + kick

Run on: **aorus**

```bash
# Have one member depart gracefully first
# (run on the member: tetron leave test-tetronnet)
# Coordinator checks:
tetron status                         # member gone

# Have another member re-join, then kick it
tetron kick test-tetronnet <SHORT_ID>
tetron status                         # kicked member gone
```

### 4f. Full daemon cycle

Run on: **aorus**

```bash
sudo tetron stop                      # fully offline
sudo tetron start                     # back online, reconnect
sudo tetron restart                   # bounce service
```

### 4g. Config commands

Run on: **aorus**

```bash
tetron config get                     # show all settings
tetron config get subnet              # show subnet
tetron config set subnet 10.77.0.0/24  # change (verify on next create)
tetron config unset subnet            # reset to default
```

---

## Phase 5: Teardown

### Option A: Graceful teardown (leave each network)

```bash
# On each member:
tetron leave test-tetronnet

# On coordinator:
tetron leave test-tetronnet
```

### Option B: Nuke (destroy network for everyone)

```bash
# Coordinator only:
tetron nuke test-tetronnet            # requires --force if other members
tetron status                         # network gone
```

---

## Results

| Test | Expected | Actual | Pass/Fail |
|------|----------|--------|-----------|
| create --subnet | invite code + warning | | |
| SUBNET-BUG-001: reject join (wrong subnet) | clear error message | | |
| subnet correct after restart | TUN IP in 10.55.55.0/24 | | |
| join (after subnet fix) | connected | | |
| all members visible | N members | | |
| single TUN device | exactly 1 tun device | | |
| ping mesh IPs | reachable | | |
| invite create | new invite code | | |
| invite list | shows invites | | |
| invite revoke | revoked | | |
| admin add | co-coordinator | | |
| admin list | 2 admins | | |
| up/down | data plane toggle | | |
| leave | graceful departure | | |
| kick | member removed | | |
| stop/start | full cycle | | |
| config get/set | settings read/write | | |

---

## Automation Notes

- All machines are x86_64, same binary.
- `aorus` is the build host (has Rust toolchain).
- `xps` doubles as jumphost for `xeon40`.
- `xeon40` is behind Tailscale (100.64.0.0/10), reached via `ssh -J 192.168.1.111`.
- Operator is auto-granted to `erik` on aorus/xps/x10sra. `xeon40` user is `noisebridge` -- sudo prompt differs.
- **TUN device cleanup** (Phase 1c) is critical before install: stale interfaces
  from a previous daemon instance persist after `sudo tetron uninstall` and
  will cause `tun0` to show the old IP even after a fresh install + restart.
  Always verify with `ip -o link show | grep tun` before and after.
- **SUBNET-BUG-001** ensures a node never joins a network whose subnet
  differs from the node's configured subnet. The check runs in
  `join_network_inner` after the blob is fetched but before any coordinator
  dial, producing a clear actionable error. This prevents the silent data
  plane breakage that previously occurred (TUN IP in wrong range, kernel
  drops cross-mesh packets).
- Test logs are written to `docs/TEST_LOG-YYYY-MM-DD.md`.
