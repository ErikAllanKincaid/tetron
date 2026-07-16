# Laptop fleet -- the way forward

A network of laptop users who come and go, with no always-on member. Three
changes make this work well without adding tiers or roles.

---

## 1. Multi-coordinator (routine)

Every fully trusted user should be made a coordinator via `tetron admin add
<net> <identity>`. This should be routine, not exceptional.

### What it means

Multiple machines hold the per-network `SecretKey`. Any of them can publish the
signed blob, admit new members, mint invites, and kick. No single machine is a
single point of failure for network administration.

### What it costs

The network key is shared among more people. In a fleet of trusted laptops
that is a feature, not a risk -- everyone on the mesh is trusted to be there.
The power to kick is the same as the power to admit; the social trust model
means no one abuses either.

### Multi-coordinator works well when paired with the other two changes

| Problem | Without fix | With multi-coordinator + blob + cache |
|---|---|---|
| Coordinator goes to sleep, someone needs to join | Join blocked -- invite is on the sleeping machine's disk | Any online coordinator validates the invite from the blob |
| Coordinator goes to sleep, someone needs to be kicked | Kick blocked | Any online coordinator kicks |
| DHT record expires after all-offline gap | First person back sees empty network | Peer address cache reconnects without DHT |
| Only one machine can republish after all-offline | Must wait for the single coordinator | Any coordinator republishes |

---

## 2. Invite in blob

Move invite data from per-coordinator disk files (`invites/<network>/<id>.toml`)
into the signed `GroupBlob`.

### Today

```
mint invite  -->  write to local file  -->  invite pinned to one machine
validate     -->  minter must be online and reachable
```

### With blob invites

```
mint invite  -->  add entry to GroupBlob  -->  sign + publish to DHT
validate     -->  any online coordinator checks the blob
```

The invite code no longer encodes the minting coordinator's endpoint ID:
`bs58(pubkey || secret)` instead of `bs58(pubkey || coordinator || secret)`.
The joiner dials any peer, not a pinned machine.

### Per-invite cost

~150 bytes in the blob (secret hash, creator, timestamps, used flag, role).
Negligible at human-scale minting rates. Removed from the blob on admission
to prevent replay races.

### Multi-coordinator dependency

Invite-in-blob requires fetch-before-publish merge so concurrent mints do not
clobber each other. Already documented.

---

## 3. Cache peer addresses to disk

On graceful shutdown (and periodically), save known peer addresses to a file.
On startup, seed iroh's peer table from the cache before querying the DHT.

### What it solves

After an all-offline gap, the first person back has no cached peer addresses
(fresh process = empty iroh table). DHT records may have expired. The relay
is empty because everyone else is offline. The network appears empty.

With a disk cache, the first person back tries each cached address directly.
If anyone else is also back, the QUIC handshake succeeds and the mesh is
live. No DHT or relay needed for bootstrap.

### Why it is safe

iroh verifies endpoint identity via the QUIC crypto handshake. A cached
address is just a hint -- wrong addresses produce connection failure, not
wrong peers. A poisoned cache cannot MITM the connection.

### Implementation sketch

- **Write:** On SIGTERM and every 5 minutes, serialize a flat map of
  `endpoint_id -> (addresses, relay_url, last_seen)` to a msgpack file in
  the config directory.
- **Read:** At startup, deserialize and call `Endpoint::add_peer_addr()` for
  each entry.
- **Prune:** Entries older than 30 days are discarded.
- **Atomic write:** Write to temp file, rename.

---

## How the three compose

| | Multi-coordinator | Invite in blob | Peer cache |
|---|---|---|---|
| Solves | No SPOF for admin actions | No SPOF for invite validation | No SPOF for reconnection |
| Requires | Nothing else | Fetch-before-publish merge | Nothing else |
| Code change | Already exists (`admin add`) | Medium (blob schema, validation path, invite encoding) | Small (~80 lines, no protocol change) |
| Order | 1 (already works) | 2 | 3 |

The three changes together mean a laptop fleet member can:

1. Come back from a week offline and immediately find other online members
   (peer cache).
2. Admit a new team member even if the person who minted the invite is asleep
   (blob invites).
3. Kick a departed member even if the network creator is offline
   (multi-coordinator).
4. Republish the DHT record so the network is discoverable again
   (multi-coordinator).

No new roles. No three-tier model. No vouchers. The two-tier model -- where
coordinators have all powers and members have none -- handles all of this
when the three supporting changes are in place.
