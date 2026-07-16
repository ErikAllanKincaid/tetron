# Design Decisions

> Quick-reference tables for all settled design questions across the tetron
> codebase. Covers tier model, laptop fleet, subnet collision, invite model,
> and feature proposals.
>
> **Current direction:** Two-tier model (coordinator / member) with three
> supporting changes for laptop-fleet operation: multi-coordinator (routine),
> invite-in-blob, and peer address cache. The three-tier model
> (admin/coordinator/member) is set aside as unnecessary complexity for
> tetron's deployment profile.
> Last updated: 2026-07-15.

---

## 1. Tier model

| Decision | Value | Source |
|---|---|---|
| Number of tiers | Two (current and planned) | `docs/IDEAS_LaptopFleet.md` |
| Roles | Coordinator, member | code -- `NetworkRole` enum |
| Coordinator powers | Everything -- admit, mint, kick, publish, nuke, grant | `docs/IDEAS_LaptopFleet.md` |
| Member powers | Use mesh only (no network key) | code |
| Three-tier model | Set aside (still documented in `docs/PRIVILEGE_TIERS.md`) | `docs/IDEAS_LaptopFleet.md` |
| Why two-tier is sufficient | At tetron's scale (small-team meshes), all key holders are equally trusted. The coordinator/member split separates "trusted to administer" from "trusted to connect." Further splitting is over-engineering. | `docs/IDEAS_LaptopFleet.md` |

---

## 2. Laptop fleet (no always-on member)

| Decision | Value | Source |
|---|---|---|
| Strategy | Three complementary changes, no new roles | `docs/IDEAS_LaptopFleet.md` |
| Multi-coordinator | `tetron admin add <net> <identity>` as routine practice. Every fully trusted user gets the key. No SPOF for administration. | `docs/IDEAS_LaptopFleet.md` § 1 |
| Invite in blob | Move invite data from machine-local files into signed `GroupBlob`. Any online coordinator validates any invite. | `docs/IDEAS_LaptopFleet.md` § 2 |
| Peer address cache | Save known addresses to disk; seed iroh's peer table on startup. Reconnection works without DHT after all-offline gaps. | `docs/IDEAS_LaptopFleet.md` § 3 |
| Invite encoding | `bs58(pubkey(32) \|\| secret(16))` -- no coordinator-endpoint pinning | `docs/IDEAS_LaptopFleet.md` § 2 |
| Survivability -- freeze | Accept that a new member cannot join when ALL coordinators are offline. Matches physical networking. Multi-coordinator reduces probability of this happening. | `docs/PRIVILEGE_TIERS.md` § Survivability -- decision |
| Survivability -- vouchers | Rejected. Too complex; multi-coordinator + blob invites cover the use case without a gossip protocol. | `docs/IDEAS_LaptopFleet.md` |

---

## 3. Subnet collision

| Decision | Value | Source |
|---|---|---|
| Phase 1 | S1 + S2: reject overlapping subnets on create and join, with `--force` override | `docs/SUBNET_COLLISION.md` § Recommendation |
| Phase 2 (deferred) | S3: per-network policy routing (correct long-term fix) | `docs/SUBNET_COLLISION.md` § Recommendation |
| Rejected | S4: enforce one subnet per node (too restrictive) | `docs/SUBNET_COLLISION.md` § Recommendation |
| Default subnet | `10.88.0.0/24` | `spec/design_spec.py` SUBNET-011 |
| VLAN analogy | TUN = switch port, network = VLAN, collision = overlapping VLAN ranges | `docs/SUBNET_COLLISION.md` § Physical-network analogy (VLANs) |

---

## 4. Invite model

| Decision | Value | Source |
|---|---|---|
| Current storage | Machine-local `InviteStore` (`invites/<network>/<id>.toml`) | code |
| Planned storage | Signed `GroupBlob` (move to blob) | `docs/IDEAS_LaptopFleet.md` § 2 |
| Planned encoding | `bs58(pubkey(32) \|\| secret(16))` -- no coordinator-endpoint pinning | `docs/IDEAS_LaptopFleet.md` § 2 |
| Invite entry lifecycle | Added on mint, removed on admission (prevents replay, bounds blob size) | `docs/SUBNET_COLLISION.md` discussion |
| Replay protection (multi-coordinator race) | Local reject cache + `InviteUsed` gossip (short TTL, expires when blob propagates) | `docs/SUBNET_COLLISION.md` discussion |
| Reusable keys | Always member-only (if implemented) | `docs/PRIVILEGE_TIERS.md` § Open questions -- answered |

---

## 5. Features

| Proposal | Verdict | Reason | Source |
|---|---|---|---|
| Pre-signed admission vouchers | Rejected | Gossip + crypto dep; multi-coordinator + blob invites are simpler | `docs/PRIVILEGE_TIERS.md` § Survivability -- decision |
| Three-tier (admin/coordinator/member) | Set aside | Over-engineering for small-team meshes. Two-tier with multi-coordinator solves the same problems. | `docs/IDEAS_LaptopFleet.md` |
| Threshold kick | Rejected | `--backup` flag on create is simpler | `docs/PRIVILEGE_TIERS.md` § Open questions -- answered |
| Policy routing | Deferred | Phase 2 for subnet collision. Correct fix but moderate effort | `docs/SUBNET_COLLISION.md` § Recommendation |
| Key rotation on admin kick | Deferred | Accept stale-blob risk; merge mitigates it | `docs/PRIVILEGE_TIERS.md` § The kicked-admin problem |
| Reusable keys carrying a role | Rejected | Always member-only | `docs/PRIVILEGE_TIERS.md` § Open questions -- answered |
| Peer address cache | Planned | Low effort (~80 lines), solves all-offline reconnection | `docs/IDEAS_LaptopFleet.md` § 3 |
| Invite in blob | Planned | Medium effort, foundation for multi-coordinator invite validation | `docs/IDEAS_LaptopFleet.md` § 2 |
| Multi-coordinator (routine) | Already works | `tetron admin add` exists; needs to be the documented practice | `docs/IDEAS_LaptopFleet.md` § 1 |

---

## 6. Hostname rename (`tetron hostname`)

Analysis of re-adding a rename command after MINIMAL-014 removed it.

### What MINIMAL-014 removed

| Piece | Deleted? | Lines |
|---|---|---|
| `Command::Hostname` CLI variant (`main.rs`) | Deleted | 5 |
| `IpcMessage::SetHostname` variant (`tetron-proto/src/ipc.rs`) | Deleted | 4 |
| `MeshManager::set_hostname()` daemon handler (`daemon/mod.rs`) | Deleted | ~75 |
| `MeshManager::announce_rename_to_peers()` (`daemon/mod.rs`) | Deleted | ~35 |
| `src/daemon/mesh/rename.rs` (pending_hostname logic, `rename_satisfied`, `has_pending_hostname`, old `outgoing_hostname`) | Deleted | 117 |
| `pending_hostname` field in `NetworkConfig` (`config.rs`) | Deleted | 1 |
| Reconverge 30s rename-backstop tick | Deleted | ~10 |
| Coordinator MeshHello hostname handling (still captures device_cert) | Deleted | ~15 |

### What survived

- `reconcile_local_hostname()` in `reconverge.rs` -- still adopts blob's authoritative name into config `my_hostname`
- `outgoing_hostname()` in `join.rs` -- now simply reads `my_hostname` from config (no more pending intent)
- `admission_hostname()` / `resolve_collision()` in `hostname.rs` -- collision helpers still there
- Hostname in `JoinRequest`/`MeshHello` still sent
- Coordinator still resolves collisions at admission

### What re-adding requires

| Layer | Change | Est. lines |
|---|---|---|
| **IPC** (`tetron-proto/src/ipc.rs`) | Add `SetHostname { network: String, hostname: String }` request variant | 4 |
| **Config** (`config.rs`) | Re-add `pending_hostname: Option<String>` to `NetworkConfig` (or skip it -- see simplified design below) | 2 |
| **Daemon handler** (`daemon/mod.rs` or `runtime.rs`) | Re-add `set_hostname()` handler: validate hostname, update local state + config, update roster entry, if coordinator republish blob, if member send MeshHello to coordinators | ~60 |
| **Coordinator control reader** (`coordinator.rs`) | Re-add hostname handling from MeshHello: resolve collision, update roster entry, trigger republish | ~30 |
| **CLI** (`main.rs` + `cli/network.rs`) | Add `Command::Hostname { network, name }`, add `ipc_set_hostname()` client | ~15 |
| **Reconverge** (`reconverge.rs`) | Optional -- `reconcile_local_hostname` already syncs the authoritative name. No backstop needed if we skip pending_hostname. | 0 |
| **Total** | | **~110 lines** |

### Simplified design (recommended if implemented)

MINIMAL-014's full `pending_hostname` machinery (durable intent, `rename_satisfied`, backstop) is not needed for a v1. A simpler flow:

1. `tetron hostname <net> <name>` validates the name, updates `my_hostname` in config + local `NetworkState` member entry, sends `MeshHello{hostname: <new>}` to every connected coordinator.
2. **Coordinator control reader**: when it receives a `MeshHello` with a hostname different from the roster, resolve collision (`resolve_collision`), update the member's entry in the roster, republish the blob.
3. **All members** adopt on next reconverge via existing `reconcile_local_hostname()`.

No `pending_hostname`, no `rename_satisfied`, no 30s backstop. The rename is best-effort until the blob confirms it. On reconnection, the member sends the new `my_hostname` from config, so the coordinator processes it again on reconnect -- exactly the same pattern as if it was the initial join name.

### Verdict

**Viable, moderate effort (~110 lines).** The simplified design eliminates most of the complexity that MINIMAL-014 removed. The coordinator control reader re-adding hostname handling is the most delicate part (must coexist with D1 device_cert capture). The pending_intent/backstop machinery would add another ~100 lines and is not worth it for tetron's use case -- a best-effort rename that converges within one reconverge cycle (30-60s) is acceptable.

Decision: **deferred** pending user demand. Note that the `kick` + rejoin workaround is not a good substitute -- it requires a coordinator to mint a fresh invite, the invite must be delivered out of band, and there is a connectivity interruption. The simplified rename is straightforward (~110 lines) when the need arises. Key insight: MINIMAL-014's heavy machinery (pending_hostname, rename_satisfied, backstop tick) was for the rename-intent survivability system, not for the rename itself. The actual rename propagation through MeshHello + coordinator republish + blob reconverge is ~30 lines in the coordinator control reader that MINIMAL-014 deleted, and the remaining ~80 lines are the CLI + IPC + daemon handler. All the supporting data (MeshCtx, dht_notify, peer table) is already live in the `CoordinatorAcceptState` call sites -- just not passed to the control reader. Revisit if users ask for rename. |
