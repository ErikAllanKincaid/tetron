# Subnet collision across networks

## Scenario

1. Admin mints an invite for Person-1 on network A (`10.88.0.0/16`).
2. Person-1 joins network A, gets IP `10.88.0.2`.
3. Person-1 reads the HOWTO and creates their own network B.
4. Person-1 runs `tetron create --hostname alice2`.

## What happens today

Two outcomes depending on what Person-1 does:

### No subnet override (default)

Network B also uses `10.88.0.0/16`. Both networks share the same subnet
on the same TUN device. Person-1 has two mesh IPs — one per network —
both in the same range. The `PeerTable` tracks them by network name, but
the kernel routing table has two routes for `10.88.0.0/16` on `tetron`,
or one route covering both. Traffic meant for network A's `10.88.0.3`
could reach network B's `10.88.0.3` instead. **Namespace collision.**

### With subnet override

Person-1 runs `tetron config set subnet 10.77.0.0/16` first, then
creates network B. Now the node has two active subnets. The kernel
handles them independently since they are non-overlapping ranges —
`10.88.x.x` goes to network A's peers, `10.77.x.x` goes to network B's.
This works, but the node's identity derivation
(`derive_ip_with_index(identity, index, subnet)`) is subnet-relative.
Joining network A used `10.88.0.0/16`; creating network B uses
`10.77.0.0/16`. The node gets different IPs in each.

## The problem

- A user can accidentally create two networks on the same subnet.
- Routing conflicts are silent — traffic misdirects without error.
- Changing the node subnet after joining a network leaves the old network
  using the old subnet while the node is configured for the new one.

## Programmatic solutions

### Solution 1: Reject duplicate subnets on create

`tetron create` checks all active networks for subnet overlap before
proceeding:

```rust
fn check_subnet_collision(new_subnet: Subnet) -> Result<()> {
    for net in networks {
        let s = net.state.read().unwrap();
        if subnets_overlap(s.subnet, new_subnet) {
            bail!("subnet {new_subnet} overlaps with network '{}' ({})",
                  net.name, s.subnet);
        }
    }
    Ok(())
}
```

Two subnets overlap if either contains the other's base address, or if
their host ranges intersect. For simplicity, reject if the prefixes are
identical or one is a strict superset of the other. Since both are
probably `/16` or `/24`, identical prefix is the common case.

**Cost:** ~15 lines, no new state.
**Gap:** Only catches collisions at create time. Does not help if the
user joins a second network with the same subnet as their first.

### Solution 2: Reject duplicate subnets on join too

Same check in `join_network_inner`. If the joining network's subnet
(from the fetched `GroupBlob` or the node's own `config set subnet`)
overlaps an existing network, refuse to join:

```rust
// In join_network_inner:
let blob_subnet = resolve_subnet(data.subnet);
let node_subnet = config::node_subnet();
let effective_subnet = blob_subnet.or(node_subnet);
check_subnet_collision(effective_subnet)?;
```

**Cost:** ~5 lines.
**Gap:** What if the user intentionally wants overlapping subnets? The
check should have a `--force` override flag.

### Solution 3: Per-network routing, not per-subnet

The root cause is that the kernel route table is subnet-scoped, not
network-scoped. A packet to `10.88.0.3` matches the first `10.88.0.0/16`
route regardless of which network it belongs to.

A more complete fix: install network-specific policy routing. Each
network gets its own routing table (e.g. table `100 + net_index`). A
packet's source IP determines which routing table to use via `ip rule
add from <network-ip> table <net_table>`. This is how Tailscale handles
multi-network setups.

```bash
# Instead of one route for the subnet:
ip route add 10.88.0.0/16 dev tetron

# Do per-network policy routing:
ip rule add from 10.88.0.2 table 100
ip route add 10.88.0.0/16 dev tetron table 100
ip rule add from 10.77.0.2 table 101
ip route add 10.77.0.0/16 dev tetron table 101
```

**Cost:** moderate (route management in `tun.rs`, cleanup on leave/nuke).
**Benefit:** eliminates namespace collision entirely, even with
identical subnets.
**Gap:** More complex, more state to manage. Needs testing on Linux +
macOS.

### Solution 4: Enforce unique subnet per node

Refuse to join or create a network whose subnet does not match the
node's configured subnet (`config::node_subnet()`). The node has one
subnet, period. All networks share it.

This is the simplest rule. The HOWTO already says "set the subnet before
it is in use." Enforcing it programmatically just makes that rule
unavoidable:

```rust
fn enforce_node_subnet(network_subnet: Subnet) -> Result<()> {
    let node_subnet = config::node_subnet();
    if network_subnet != node_subnet {
        bail!("network uses {network_subnet} but node is on {node_subnet}; "
              + "run `tetron config set subnet {network_subnet}` first");
    }
}
```

**Cost:** ~10 lines.
**Downside:** Forces all networks on one node to share the same subnet.
If you join network A on `10.88.0.0/16`, you cannot create network B on
`10.77.0.0/16` without leaving A first. This matches the single-TUN
nature of tetron — one device, one subnet — but is restrictive.

## Recommendation

**Do Solution 1 + Solution 2 (reject overlapping subnets on create and
join) with a `--force` flag.** This catches the common mistake while
allowing power users who understand the routing implications to proceed.

Solution 3 (policy routing) is the correct long-term fix but is
higher effort. Add it as a deferred TODO.

Solution 4 is too restrictive — it prevents legitimate mixed-subnet
setups that work fine today with non-overlapping ranges.

### How the HOWTO should warn

Add to the "Custom subnet" section of `docs/HOWTO.md`:

> If you already belong to a network on `10.88.0.0/16` and create (or
> join) a second network on the same subnet, traffic can route to the
> wrong peer. tetron now refuses this by default. Use `--force` to
> override, or set a different subnet first:
> ```bash
> tetron config set subnet 10.77.0.0/16
> sudo tetron restart
> tetron create --hostname alice2
> ```
