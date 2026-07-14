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

---

## The Ethernet analogy — a simpler lens

A tetron mesh is a virtual Ethernet cable. Joining is plugging in. What you
do with your connection is your business (and your firewall's). The network
admin can kick your MAC address at the router.

### The analogy

| Physical Ethernet | tetron |
|---|---|
| Plug patch cable into switch | Present invite key, get mesh IP |
| Switch forwards frames to all ports | Mesh forwards packets to all peers |
| Your machine listens on ports you choose | Host firewall on TUN interface |
| Admin blocks MAC on switch port | `tetron kick <peer>` |
| Switch keeps working when admin is out | Mesh keeps working when creator is offline |
| New person starts, needs admin to free a port | New join needs an invite from creator |
| If admin is gone forever, network is frozen | Same — roster freezes, no new admissions |

### What this implies

**Data plane is flat.** Every member on the mesh can reach every other
member's ports. No per-peer access control in the mesh itself — that is the
host firewall's job. tetron already matches this (MINIMAL-010 removed the
userspace firewall).

**Administration is separate from connectivity.** The admin role is only
about managing the membership list — who gets to be on the network. It has
no bearing on what members do once connected, any more than an Ethernet
switch admin controls what you do with your link.

**Joining should not carry privileges.** An invite is a "patch cable." It
admits you to the network. It should not encode an admin or coordinator
role, because being on the network and administering the network are
different concerns.

### What simplifies

The three-tier model (admin/coordinator/member) was over-engineered.
The Ethernet lens suggests two tiers:

| Tier | Powers |
|---|---|
| **Admin** | Kick, mint invites, publish blob |
| **Member** | Use the mesh |

No coordinator tier. No invites with `--admin` or `--coordinator` flags.
An invite is just an invite.

### The network key

Only admins hold the per-network `SecretKey`. Members do not need it —
they never publish, mint, or kick.

This creates the laptop fleet problem in its purest form: if the admin is
offline, no one can admit new members. But the Ethernet analogy accepts
this as normal. A physical switch port assignment waits for the admin to
come back. The mesh keeps working for existing members.

**If the admin is permanently gone** (lost laptop, left the group), the
network is administratively frozen but data-plane traffic between existing
members keeps working indefinitely. This is a design trade-off, not a bug,
and matches physical networking.

### Escape hatch: admin transfer

If permanent admin loss is unacceptable, add a single escape: a `--backup`
flag on `create` that pre-authorizes another identity as co-admin:

```bash
# Creator specifies a backup admin at network creation time.
tetron create --hostname alice --backup <bobs-endpoint-id>
```

The backup has the same powers as the creator. No tiers, no negotiation,
no consensus protocol. Just a second key holder from day one.

### What this means for earlier proposals

| Earlier proposal | Simplified by Ethernet lens |
|---|---|
| Three-tier (admin/coordinator/member) | Two-tier (admin/member). No coordinator role. |
| Invite encodes tier | Invite is just an invite. No role. |
| Auto-coordinator on join | Members never get the network key. |
| Invite in blob (needed for coordinator-less admission) | Invite in blob still useful — any admin can mint, any admin can validate. But invite does not carry a role. |
| Fetch-before-publish merge | Still needed if there are multiple admins. |
| Threshold kick | Unnecessary if admin transfer exists. |

### Remaining open questions through this lens

1. **Does invite-in-blob still make sense?** Yes — if there are multiple
   admins, any of them can mint an invite, and any of them can validate it
   without needing to coordinate. This is independent of the tier model.

2. **Is fetch-before-publish still needed?** Yes — multiple admins can
   still race on publishing. The problem is cross-admin, not
   cross-coordinator.

3. **How does the backup admin get created?** `--backup` flag on `create`,
   or `admin add` by the original admin later. The default is a single
   admin (the creator).

4. **Should there be a way to revoke admin from the backup?** In
   Ethernet terms, this is "fire the co-admin." The original admin kicks
   them (since kick works on any member) and sets a new backup. This is
   the same as kicking any other member — the backup is just a member with
   the key, and a kicked member with a key can still sign a stale blob.
   **This is a real problem.** See below.

### The kicked-admin problem

A kicked admin still holds the network key. They can still sign and publish
a blob even after being kicked. The blob will be stale (missing newer
members), but the DHT does not filter by membership — it filters by
signature, and the key still signs.

Solutions:

- **Rotate the network key on admin kick.** Generates a new keypair,
  republishes the blob, and distributes the new key to remaining admins
  via a secure channel (the existing mesh ALPN). This is complex but
  correct.

- **Accept it.** A kicked admin with the key can cause at most a stale
  blob revert, which is fixed by the next admin publish or by
  fetch-before-publish on all admins. They cannot corrupt the roster
  beyond what the merge resolves.

The second option is simpler and matches real Ethernet — a disgruntled ex-employee with physical access to the wiring closet can unplug cables. You change the locks (rotate the key) when you can.

---

## The router analogy — second pass

The Ethernet analogy is useful but I collapsed it too far the first time. A
physical network is not just cables; it has a **router/switch** that is the
shared infrastructure. tetron distributes that function across members who
hold the network key.

### What the router does in a physical network

| Router function | tetron equivalent |
|---|---|
| DHCP (hand out IPs) | `assign_ip` in the signed blob |
| ARP table / DHCP lease list (who is connected) | `MemberList` in the signed blob |
| Port security / MAC filtering (who gets on) | Invite validation + admission gate |
| VLAN config (which ports can talk to which) | (No equivalent — mesh is flat after MINIMAL-010) |
| Keeps forwarding when admin walks away | Mesh keeps working; roster is cached |
| Admin must be present to add a new port | Coordinator must be online to admit |

### The roles re-derived from this

| Physical network | tetron role | What you can do |
|---|---|---|
| Senior sysadmin (can MAC-ban, change VLANs) | Admin | Kick, promote, nuke |
| Junior sysadmin (can DHCP-reserve, config switch ports) | Coordinator | Admit, mint invites, publish blob |
| Staff with Ethernet cable | Member | Use the mesh |

**All three exist in physical networking.** They just are not named because
the router is a box with a single password. tetron distributes the router
across members, making the roles explicit.

### What this means for the design

**The coordinator tier is not over-engineering.** It is "can operate the
shared infrastructure." Multiple coordinators = more people who can
configure the switch when the senior admin is out. The earlier collapse to
two tiers (admin/member) was wrong.

**The invite encodes the tier.** A coordinator invite says "you are
authorized to touch the router." A member invite says "you are authorized
to plug in." The tier is a property of the invite, not a post-join grant.

**Admission is not "plugging in."** It is: the sysadmin pre-authorizes your
MAC on the switch port, then you plug in. The invite is the
pre-authorization. A coordinator admitting is the sysadmin configuring the
port.

**The blob is the router config.** It lists every device, its IP, and its
role. Publishing is saving the running config. Multiple people can save —
last-write-wins needs protection (fetch-before-publish merge, version
vectors).

### The network key is the router password

How many people have it is a trust decision per network, not a protocol
decision. The protocol should support any number:

- **Single admin:** one router password, one person operates it.
- **Two co-admins:** creator sets a `--backup` at create time.
- **Everyone is a sysadmin:** all join with `--coordinator` invites. The
  laptop fleet model where any online member can admit.
- **Mix:** some coordinators, mostly members. The normal case.

## Cross-network isolation

A machine is frequently a member of multiple tetron networks. Each network
is independent — membership on one has no bearing on authority on another.

### Concrete scenario

Person-1:
- Joins network-1 (via invite) as a **member** — no network key.
- Creates network-2 (with a different subnet) as **coordinator** — holds the key.

If person-2 tries to join network-1 and their connection lands on
person-1's node, person-1 does **not** authenticate them. The
`ProtocolRouter` dispatches by ALPN: network-1's ALPN maps to a
`MemberAcceptState` handler, which expects existing roster members and
does not validate invite secrets. The connection is silently dropped.

If person-3 tries to join network-2 and lands on person-1's node,
person-1 **does** authenticate them. Network-2's ALPN maps to a
`CoordinatorAcceptState` handler, which validates the invite secret
against the local `InviteStore` and admits on success.

### Why it works this way

Each network independently registers its `AcceptHandler` in the shared
`ProtocolRouter` at join/create time. The ALPN carries the network's
public key prefix, so the router can distinguish networks that belong to
the same daemon. The registration decision is based on whether this node
holds that *specific* network's secret key:

- **Has the key** → registers `CoordinatorAcceptState` (can admit, validate
  invites, publish blob).
- **Does not have the key** → registers `MemberAcceptState` (can only
  accept reconnecting known members, ignores strangers).

### Design implication

This is correct behavior — holding the key for one network must not imply
authority on another. It is a direct consequence of the tier model:
authority is a property of the per-network key, not of the node. A node
can be admin on network A, coordinator on network B, and member on
network C simultaneously, and the `ProtocolRouter` enforces the
boundary automatically.

---

### What stays from the earlier discussion

| Feature | Still needed? | Why |
|---|---|---|
| Invite in blob | Yes | Any coordinator can mint; any coordinator can validate. No machine-local bottleneck. |
| Fetch-before-publish merge | Yes | Multiple coordinators (or a kicked key holder) can publish concurrent rosters. |
| Invite encoding (pubkey + secret) | Yes | Removes coordinator-endpoint pinning so any online coordinator can validate. |
| Auto-coordinator on `--coordinator` invite | Yes | The invite encodes the tier; join handshake grants key. |
| `--backup` flag on create | Yes | Pre-authorize a second admin day one. |
| Kick requires admin | Yes | Matching physical MAC-ban. |
| Role encoded in invite | Yes | Coordinator vs member determined at admission, not after. |
| Threshold kick | Unnecessary | Admin transfer / backup is simpler. |
| Key rotation on admin kick | Deferred | Accept stale-blob risk for now; merge mitigates it. |

---

## Survivability assumption

The central unsolved tension: **can a new member join when all
coordinators are offline?**

### What works today

| Scenario | Works? | Why |
|---|---|---|
| Existing member reconnects after being offline | Yes | Cached blob; peer-to-peer blob exchange converges. |
| New member joins while a coordinator is online | Yes | Coordinator validates invite, admits, publishes. |
| New member joins while ALL coordinators are offline | **No** | No one holds the network key. Invite cannot be validated or burned. Blob cannot be published. |

### The question the document does not answer

Choose one:

**A. Accept the freeze.** The coordinator tier is a SPOF for growth but
not for connectivity. New members wait until a coordinator comes back.
Matches physical networking — the new hire sits in a bare cube until the
sysadmin activates their switch port. The invite is a pre-authorization
(like a port reservation), not a self-service credential.

**B. Pre-signed admission vouchers.** The invite is a signed statement
from a coordinator: "bearer of secret X joins as hostname bob, role
member." The voucher is signed by the minting coordinator's endpoint key
(not the network key). The coordinator's public key is listed in the
blob's `admins` field. Any online member verifies the signature and admits
the joiner locally, propagating the updated roster to peers via gossip. No
blob publish happens until a coordinator returns, fetches the converged
roster, and publishes the canonical signed blob.

```rust
// The voucher payload (signed by a coordinator's endpoint key):
struct AdmissionVoucher {
    network_pubkey: EndpointId,
    invite_secret: [u8; 16],
    hostname: Option<String>,
    role: Role,
    expires_at: u64,
}
```

Costs:
- Signature verification in the admit path (~new dep: ed25519-dalek or
  use iroh's existing signing)
- Voucher replay protection (nonce or expiry)
- Roster convergence by gossip (members exchange their local roster
  versions; newest winning)
- Blob publish by the returning coordinator (fetch from peers, merge,
  sign, publish)

**C. Give every member the network key** (the `--coordinator` default).
If everyone is a coordinator, everyone can admit. Blast radius of a
compromised key is limited by the admin tier (cannot kick). This is the
laptop-fleet model from earlier in this document. Simpler than vouchers
but trusts every member with the signing key.

### Impact on the rest of the design

| Feature | Needed under A? | Needed under B? | Needed under C? |
|---|---|---|---|
| Invite in blob | Yes (multi-coordinator validation) | Yes (members need invite table for voucher check) | Yes |
| Fetch-before-publish merge | Yes | Yes | Yes |
| Invite without coordinator endpoint | Yes | Yes | Yes |
| Auto-coordinator on join | No (only admins publish) | No | Yes (default) |
| Voucher signature verification | No | Yes (new) | No |
| Roster gossip between members | No | Yes (new) | No |
| Network key on all members | No | No | Yes |
| Coordinator SPOF for growth | Yes (accepted) | No | No |

### When the choice matters

- **Laptop fleet (no always-on node).** A is painful — new team members
  cannot join until a specific coordinator wakes up. B or C is better.
- **Corporate network.** A is fine. There is always a coordinator (the
  ops team runs one on a server). B and C add complexity for no benefit.
- **Mixed deployment.** Some networks want the freeze, some do not. The
  protocol could support both — the invite encoding includes a flag that
  selects the admission mode.

Not deciding this yet. Recorded for next session.
