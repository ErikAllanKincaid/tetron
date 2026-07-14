# Privilege Tiers — Design Discussion

A tetron network today has two roles: **coordinator** (holds the per-network
`SecretKey`, can publish the signed blob, can admit, kick, and mint invites)
and **member** (uses the mesh, no network key). This document explores how
many tiers we need, what powers each tier carries, and how the design changes
for a network with no always-on node (the "laptop fleet" problem).

---

## Problem statements

### P1: No always-on member

In a network of laptop users who come and go, there is no stable coordinator.
When:
- The minting coordinator goes to sleep, its local invite store is unreachable
- Everyone goes offline, the DHT record expires
- The first person back needs to re-establish the network

The system must survive any single member being offline.

### P2: Additive vs destructive powers

Admitting, minting invites, and publishing the blob are **additive** — they
grow the shared state. Anyone doing them helps the network.

Kicking, revoking admin, and nuking are **destructive** — they shrink or
corrupt state. Not everyone should be trusted with these.

### P3: Read-only usage

Should a device exist on the mesh purely for connectivity — no ability to
administer the network at all? This matters for:
- IoT / server nodes that only need to accept connections
- Untrusted or guest devices
- Reducing the blast radius of a compromised member

---

## Powers catalog

| Power | Effect | Kind | Needs network key? |
|---|---|---|---|
| Use mesh | Send/receive traffic through TUN | Passive | No |
| Admit joiner | Add a peer to the roster | Additive | Yes |
| Mint invite | Create an invite key for future admission | Additive | Yes |
| Publish blob | Sign and push the roster to the DHT | Additive | Yes |
| Grant coordinator | Promote a member to coordinator | Additive | Yes |
| Kick member | Remove a peer from the roster | Destructive | Yes |
| Revoke coordinator | Demote a coordinator to member | Destructive | Yes |
| Grant admin | Promote to full admin | Destructive | Yes |
| Nuke | Publish empty roster, destroy network | Destructive | Yes |

---

## Tiers compared

### Two-tier (today)

| Tier | Powers | Has network key? |
|---|---|---|
| **Coordinator** | Everything | Yes |
| **Member** | Use mesh only | No |

**Problems:**
- No gradation — a coordinator can kick as easily as admit
- Invites are machine-local, so only the minting coordinator can validate them
- Every member without the key is dead weight for network availability

### Two-tier, power-split (proposed in earlier discussion)

| Tier | Powers | Has network key? |
|---|---|---|
| **Admin** | Kick, revoke, nuke, grant | Yes |
| **Coordinator** | Admit, mint, publish only | Yes |
| **Member** | Use mesh only | No |

**Numbers show three tiers even though the name says two-tier. See below.**

### Three-tier (final proposal)

| Tier | Powers | Has network key? | Examples |
|---|---|---|---|
| **Admin** | Everything including kick, revoke, nuke, grant any power | Yes | Network creator, trusted ops |
| **Coordinator** | Admit, mint invites, publish blob | Yes | Trusted laptop users |
| **Member** | Use mesh only | No | Servers, IoT, guests |

### Invite encodes the tier

```bash
# Joins as a member (default).
tetron invite mynetwork create --hostname bob

# Joins as a coordinator (gets network key).
tetron invite mynetwork create --hostname bob --coordinator

# Joins as a coordinator + kick power (higher trust).
tetron invite mynetwork create --hostname bob --admin
```

---

## Feature comparison

| Aspect | Two-tier (today) | Three-tier |
|---|---|---|
| Tiers | Coordinator, member | Admin, coordinator, member |
| Kick requires | Network key (anyone who has it) | Admin only |
| Invite validation | Minting coordinator's local store | Any online admin/coordinator (via blob) |
| Laptop fleet ready | No — coordinators are a SPOF | Partial — see below |
| Grant path | `admin add` → coordinator | `admin add` → coordinator; `--admin` → admin |
| Invite encoding | `bs58(pubkey∥coordinator∥secret)` | `bs58(pubkey∥secret∥role)` |
| CLI complexity | Simple | Moderately more |
| Stale blob protection | None | Fetch-before-publish merge |
| Security containment | Network key on every coordinator | Network key on every admin/coordinator, away from members |

---

## Mapping to the laptop fleet

For a network of trusted laptops with no always-on member:

- **Every joiner gets `--coordinator`.** Everyone has the network key. Anyone
  can admit new joiners, anyone can republish the blob, anyone can re-seed
  the DHT when they come back online after an all-offline gap.
- **One or two admins** (the creator plus a backup). Only they can kick.
- **Servers and static nodes join as members.** They connect and route
  traffic but never hold the key.

The risks of giving everyone the key are contained by the admin tier: a rogue
coordinator can add people (trivially revertible with fetch-before-publish
merge) but cannot kick anyone out, cannot nuke the network, and cannot
demote the admin.

---

## Changes needed from today's code

### Invite moves into the blob

The single-use invite store moves from local disk files (`InviteStore`,
`invites/<network>/<id>.toml`) into the signed `GroupBlob`:

```rust
// In GroupBlob (new):
invites: Vec<InviteEntry>,

struct InviteEntry {
    secret_hash: String,       // blake3 hex
    created_by: EndpointId,    // who minted it
    created_at: u64,
    expires_at: u64,           // 0 = permanent
    used: bool,
    role: Role,                // member, coordinator, admin
}
```

Every online admin/coordinator can validate any invite because all invite
data lives in the signed blob, not on one machine's disk.

### Invite encoding changes

Current: `bs58(pubkey(32) ∥ coordinator_endpoint(32) ∥ secret(16))`
New:     `bs58(pubkey(32) ∥ secret(16))`

The joiner does not dial a specific coordinator. They look up the network on
the DHT, dial any available peer, and present the secret. The peer checks
the blob's invite table.

### Stale blob prevention

Fetch-before-publish merge:

```rust
fn maybe_merge(current_roster: &[Member], local_roster: &[Member]) -> Vec<Member> {
    if local_roster is subset of current_roster {
        return union(local_roster, current_roster);
    }
    local_roster
}
```

### Auto-coordinator on join

Every new member who presents an invite with `role = coordinator` or `admin`
receives the network key during the join handshake. No separate `admin add`
step needed.

### Access gates

Existing kick/publish/admit paths gain a role check:

```rust
fn may_kick(actor: EndpointId, state: &NetworkState) -> bool {
    state.members.get(&actor).map(|m| m.role.is_admin()).unwrap_or(false)
}

fn may_publish(actor: EndpointId, state: &NetworkState) -> bool {
    state.members.get(&actor).map(|m| m.role.can_publish()).unwrap_or(false)
    // admin and coordinator both return true
}
```

### Deprecations

- `tetron admin add` → kept, but distinguishes `--admin` vs default
  (coordinator). The existing behavior ("grant full key") becomes
  `--admin`.
- `InviteStore` on disk → removed in favor of blob invites.
- Old invite encoding without role → rejected with a clear error
  ("legacy invite format; ask for a new one").

---

## Open questions for next session

1. **Threshold kick.** Should N-of-M coordinators together kick without an
   admin? This adds a consensus path but helps if the admin is permanently
   gone.

2. **Invite revocation.** With invites in the blob, revoking means
   publishing a new blob. The old invite is burned by marking `used = true`.
   Should there be a separate `invite revoke` flow or just `kick` + re-issue?

3. **Reusable keys (`--reusable`).** These already live in the blob
   (`GroupBlob.reusable_keys`). Phase 5 from the original plan. Do
   reusable keys also carry a role? Or are they always member-only?

4. **Role changes after join.** Can an admin downgrade a coordinator to
   member? This requires revoking the network key — but the network key is
   what signs the blob. If the key is baked into the member's config, there
   is no way to remotely un-bake it. Kick + re-join with a new invite is the
   only practical path.

5. **Read-only members.** What specifically should a member be unable to
   do? The list above says "no network key = cannot publish/admit/kick."
   Is there any other power a member should lack? Viewing `tetron status` is
   data-plane level and available to everyone.

6. **Transition.** Existing networks have coordinator-only members with the
   key. How does an existing network migrate to the new tier model? Does
   every existing member become admin, or coordinator, or need to re-join
   with a new invite?
