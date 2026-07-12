# torpedo-min: project proposal

## Summary

torpedo-min is a minimal variant of [torpedo](https://github.com/ErikAllanKincaid/torpedo), itself a fork of [rayfish](https://github.com/rayfish/rayfish). It follows the Unix philosophy: do one thing, do it well. The one thing is connecting a set of machines into a private mesh network with stable addresses. Everything else that torpedo accumulated (userspace firewall, Magic DNS, file transfer, embedded SSH, self-update, declarative apply, diagnostics, direct-connect, multi-device pairing) is removed. Name resolution, packet filtering, file copying, and remote shells already have excellent dedicated tools; torpedo-min hands those jobs back to them.

Full-fat torpedo stays as it is and continues to evolve in its own repository. This repository was created as a git clone of torpedo at commit 4809edb, so the two share history and fixes can be cherry-picked in either direction.

## Motivation

- Torpedo's main crate is roughly 34,500 lines. The connect-a-network core is about 40 percent of that. The rest is feature surface that each carries its own bugs, security exposure, host mutation, and maintenance load.
- The removed features overlap with mature host tools: nftables/ufw filter packets, /etc/hosts or real DNS resolves names, scp/rsync copy files, sshd provides shells, package managers update binaries.
- A smaller daemon is easier to audit. The daemon runs as root and owns a TUN device; every removed subsystem (especially the OS DNS mutator and the userspace SSH server) is attack surface and failure surface that no longer exists.
- Dependency shrink is large: reqwest, rustls (direct), russh, pty-process, uzers, zbus, inotify, indicatif, crossterm, opentelemetry, self-replace, sha2, semver, mime_guess, humansize, and iroh-mdns-address-lookup all leave. iroh and iroh-blobs remain (iroh-blobs transports the signed GroupBlob and is core, not a file-sharing extra).

## What torpedo-min is

Identity (Ed25519 key) -> signed pkarr record -> signed GroupBlob roster -> iroh QUIC mesh -> TUN forwarding. Machines get stable IPv4 (configurable subnet, default 10.88.0.0/16) and IPv6 (200::/7) addresses derived from cryptographic identity.

The complete CLI surface:

```
torpedo create [--name n] [--hostname h] [--subnet CIDR]   # closed network, prints room id
torpedo join <room-id-or-invite> [--name alias] [--hostname h]
torpedo leave <net>  |  nuke <net>
torpedo requests <net>  |  accept <net> <id>  |  deny <net> <id>
torpedo admin <net> add <id> | list
torpedo kick <net> <peer>
torpedo status [--json]
torpedo up | down
torpedo config [get|set|unset]        # relay, discovery-dns, subnet only
torpedo completions <shell>  |  version
sudo torpedo install | restart | uninstall | start | stop | set-operator <user>
```

Kept internals: identity, transport (fixed port 43737, relays, pkarr discovery), dht, membership, control (rate-limited), peers, tun, forward (with the upstream anti-spoof ingress check), config (trimmed), ipc, daemon core (create/join/accept/bootstrap/publish/reconverge/coordinator/select/runtime), shutdown, logdir, hostname and network-name generation, the operator privilege model, and the panic-fail-fast convention.

## What is removed

| Feature | Replacement |
|---|---|
| Self-update (already disabled by CON-006) | package manager / redeploy |
| Embedded SSH server + port-22 NAT | host sshd over the mesh IPs |
| File transfer + auto-accept | scp/rsync over the mesh IPs |
| Multi-device pairing, unpair, cert revocation, 1Password backup | one identity per device; back up the key file yourself |
| Direct connect (contact ids, friend requests) | create a 2-member network |
| ping / netcheck diagnostics | ping/mtr against mesh IPs |
| mDNS local discovery | relays + pkarr discovery |
| Tor transport, OTLP export, deep links, audit log, metrics export, report bundles | none (out of scope) |
| Userspace firewall, REJECT mode, coordinator rule suggestions | nftables/ufw on the TUN interface |
| Declarative apply, aliases, groups, identityof | scripts over `status --json` |
| Magic DNS (.ray), OS DNS configuration | /etc/hosts, or scripts over `status --json` |
| Open networks, invite minting, reusable-key minting | closed networks with live approval (`requests`/`accept`) |
| Hostname rename propagation, ephemeral auto-kick | hostname is fixed at join; `kick` remains |
| Styled CLI (colors, spinners, tables, interactive picker) | plain text plus `--json` |
| ray-mobile Android crate | out of scope |

## Key design decisions

**D1: Wire compatibility with full torpedo is preserved.** `MESH_PROTOCOL_VERSION` stays 1, ALPNs are unchanged, and the GroupBlob schema keeps its `suggested_firewall` and `reusable_keys` fields. A torpedo-min node ignores firewall suggestions instead of enforcing them, preserves the fields verbatim when it republishes as coordinator, and still validates reusable keys and invite secrets presented by joiners where it can. This means min and full nodes interoperate on one network (for example: torpedo-min on servers, full torpedo on a laptop), and it keeps membership.rs textually close to torpedo for cherry-picking. Control messages a min node no longer initiates (Ping, file offers, pair) are either answered passively (Pong) or politely refused, never a decode error.

**D2: Admission is closed-plus-approval only.** `torpedo create` always makes a Restricted network; `--open` is gone. Joining a min-coordinated network is: dial, land in the pending queue, coordinator runs `torpedo accept`. A min node can still *join* a full-torpedo network using an invite code (redemption is a few client-side lines and is kept). Invite minting, the invite ledger, invite gossip, and reusable-key minting are removed. `admin add` (co-coordinator grant) is kept: it is small and is the availability story for admission.

**D3: No host mutation beyond the TUN device and routes.** Removing the DNS stack removes the resolv.conf takeover, NetworkManager drop-ins, and the panic-hook DNS restore. The daemon's host footprint becomes: TUN device, routes, config dir, log dir, unix socket.

**D4: Security posture change, stated loudly.** Without the userspace firewall, every mesh peer reaches every port on the TUN interface. The README must say so and show the two-line nftables equivalent. The mesh itself remains the coarse boundary (peers must share a network), and the anti-spoof ingress check stays.

**D5: Same binary name, no host coexistence with full torpedo.** The binary, service, paths, and ALPNs keep the torpedo identity, so torpedo-min and full torpedo can not be installed on the same host; a host runs one or the other. They *can* share a network (D1). All KEEP-ON-PURPOSE rules from torpedo (crate name `rayfish`, relay preset, upstream REPO_SLUG references that survive, author attribution) carry over unchanged.

**D6: Spec-first workflow carries over.** Same libspec + reconcile.py discipline: one requirement per commit, reconcile.py green, `libspec link` after each commit. New requirements are MINIMAL-*; new constraints are CON-M* (a separate constraint namespace so future torpedo CON-0xx numbers never collide when cherry-picking). Inherited SUBNET-*/RENAME-*/CON-* specs remain valid until a removal commit retires them explicitly.

## Costs and risks

- Two repositories to maintain. Mitigation: torpedo-min deletes whole files and avoids reshaping what it keeps, so torpedo fixes cherry-pick cleanly; the shared history makes `git cherry-pick` from the torpedo remote routine.
- Wire compatibility (D1) constrains how much membership/control code can be deleted. This is deliberate: the deleted code is feature surface, not protocol surface.
- reconcile.py and the tests/ e2e harness exercise removed features (firewall, dns, invite, files, ssh). Each removal commit must trim the corresponding checks and tests in the same commit to stay green.
- Some peripheral removals touch shared plumbing (DeviceUserMap, stats counters wired through forward.rs) and need care rather than bulk deletion.

## Success criteria

1. `cargo build` produces a torpedo binary whose CLI is exactly the surface above.
2. Main crate under ~15,000 lines; Cargo.toml direct dependencies roughly halved; CON-M01 (dependency absence gate) green.
3. A torpedo-min node and a full torpedo node join the same network and pass traffic (wire-compat proof, CON-M02).
4. Two torpedo-min nodes: create, approve, join, ping over mesh IPs, kick, leave, all green in the trimmed e2e harness.
5. reconcile.py green on every commit; libspec ledger continuous from torpedo's history.

## Naming

Directory and working name: torpedo-min. The binary stays `torpedo` (D5). If the project later wants a distinct install identity so both variants can coexist on one host, that is a RENAME-style follow-up project, not part of MINIMAL.
