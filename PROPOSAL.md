# tetron: project proposal

## Summary

tetron is a minimal, standalone P2P mesh VPN: it connects a set of machines into a private overlay network with stable addresses and nothing else.

It began as a fork of [torpedo](https://github.com/ErikAllanKincaid/torpedo) (itself a fork of [rayfish](https://github.com/rayfish/rayfish)), but the product identity is now independent. The full-fat torpedo feature set (userspace firewall, Magic DNS, file transfer, embedded SSH, self-update, declarative apply, diagnostics, direct-connect, multi-device pairing) has been stripped away. Name resolution, packet filtering, file copying, and remote shells already have excellent dedicated tools; tetron hands those jobs back to them. This repository shares git history with torpedo (cloned at commit 4809edb) so fixes can be cherry-picked from either side, but the two projects are not wire-compatible.

## Motivation

- Torpedo's main crate is roughly 34,500 lines. The connect-a-network core is about 40 percent of that. The rest is feature surface that each carries its own bugs, security exposure, host mutation, and maintenance load.
- The removed features overlap with mature host tools: nftables/ufw filter packets, /etc/hosts or real DNS resolves names, scp/rsync copy files, sshd provides shells, package managers update binaries.
- A smaller daemon is easier to audit. The daemon runs as root and owns a TUN device; every removed subsystem (especially the OS DNS mutator and the userspace SSH server) is attack surface and failure surface that no longer exists.
- Dependency shrink is large: reqwest, rustls (direct), russh, pty-process, uzers, zbus, inotify, indicatif, crossterm, opentelemetry, self-replace, sha2, semver, mime_guess, humansize, and iroh-mdns-address-lookup all leave. iroh and iroh-blobs remain (iroh-blobs transports the signed GroupBlob and is core, not a file-sharing extra).

## What tetron is

Identity (Ed25519 key) -> signed pkarr record -> signed GroupBlob roster -> iroh QUIC mesh -> TUN forwarding. Machines get stable IPv4 (configurable subnet, default 10.88.0.0/16) and IPv6 (200::/7) addresses derived from cryptographic identity.

The complete CLI surface:

```
tetron create [--name n] [--hostname h] [--subnet CIDR] [--tor]   # closed network, prints room id
tetron join <room-id-or-invite> [--name alias] [--hostname h] [--tor]
tetron leave <net>  |  nuke <net>
tetron requests <net>  |  accept <net> <id>  |  deny <net> <id>
tetron admin <net> add <id> | list
tetron kick <net> <peer>
tetron status [--json]
tetron up | down
tetron config [get|set|unset]        # relay, discovery-dns, subnet only
tetron completions <shell>  |  version
sudo tetron install | restart | uninstall | start | stop | set-operator <user>
```

Kept internals: identity, transport (fixed port 43737, relays, pkarr discovery), dht, membership, control (rate-limited), peers, tun, forward (with the upstream anti-spoof ingress check), config (trimmed), ipc, daemon core (create/join/accept/bootstrap/publish/reconverge/coordinator/select/runtime), shutdown, logdir, hostname and network-name generation, the operator privilege model, the panic-fail-fast convention, and the compile-time `tor` feature (off by default, see D7).

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
| OTLP export, deep links, audit log, metrics export, report bundles | none (out of scope) |
| Userspace firewall, REJECT mode, coordinator rule suggestions | nftables/ufw on the TUN interface |
| Declarative apply, aliases, groups, identityof | scripts over `status --json` |
| Magic DNS (.ray), OS DNS configuration | /etc/hosts, or scripts over `status --json` |
| Open networks, invite minting, reusable-key minting | closed networks with live approval (`requests`/`accept`) |
| Hostname rename propagation, ephemeral auto-kick | hostname is fixed at join; `kick` remains |
| Styled CLI (colors, spinners, tables, interactive picker) | plain text plus `--json` |
| ray-mobile Android crate | out of scope |

## Key design decisions

**D1 (RETIRED by RENAME-M02): full product rename.** tetron is no longer wire-compatible with full torpedo. The ALPN prefix changed from `torpedo/net/...` to `tetron/net/...`, so the two meshes cannot interoperate — they negotiate different ALPNs at the QUIC handshake and never connect. The binary, service unit, config/log/socket paths, and all user-facing identity were renamed from `torpedo` to `tetron`. A brief attribution note in the README and the upstream author field in Cargo.toml are the only remaining references to the project's lineage. The `GroupBlob` schema still retains its `suggested_firewall` and `reusable_keys` fields for schema stability, but they are inert in tetron.

**D2: Admission is closed-plus-approval only.** `tetron create` always makes a Restricted network; `--open` is gone. Joining is: dial the room id, land in the pending queue, coordinator runs `tetron accept`. Invite minting, the invite ledger, invite gossip, and reusable-key minting are removed. `admin add` (co-coordinator grant) is kept: it is small and is the availability story for admission.

**D3: No host mutation beyond the TUN device and routes.** Removing the DNS stack removes the resolv.conf takeover, NetworkManager drop-ins, and the panic-hook DNS restore. The daemon's host footprint becomes: TUN device, routes, config dir, log dir, unix socket.

**D4: Security posture change, stated loudly.** Without the userspace firewall, every mesh peer reaches every port on the TUN interface. The README must say so and show the two-line nftables equivalent. The mesh itself remains the coarse boundary (peers must share a network), and the anti-spoof ingress check stays.

**D5 (RENAMED): Product identity is tetron, fully independent.** The binary, service unit, config/log/socket paths, ALPN prefixes, and all user-facing identity are renamed from `torpedo` to `tetron`. tetron and full torpedo can coexist on the same host (different ports, paths, and service names) and are unaware of each other. KEEP-ON-PURPOSE rules from the torpedo/rayfish lineage (the `"rayfish"` relay preset keyword and URLs, upstream author attribution in Cargo.toml) remain unchanged.

**D6: Spec-first workflow carries over.** Same libspec + reconcile.py discipline: one requirement per commit, reconcile.py green, `libspec link` after each commit. New requirements are MINIMAL-*; new constraints are CON-M* (a separate constraint namespace so future torpedo CON-0xx numbers never collide when cherry-picking). Inherited SUBNET-*/RENAME-*/CON-* specs remain valid until a removal commit retires them explicitly.

**D7: Tor stays, as compile-time-gated glue, with a flexible per-network policy as the post-MINIMAL roadmap.** Tor carries only TCP streams, so an iroh QUIC/UDP mesh can not be torified externally (torsocks, TransPort redirection, and gateway setups all drop UDP); the in-endpoint iroh-tor-transport integration is the only way, and it already delegates the actual onion routing to the system Tor daemon. The `tor` cargo feature and the per-network `--tor` flag therefore survive MINIMAL-008 unchanged; default builds carry zero Tor code. The flexible target is TOR-M01 (deferred): a per-network transport policy `any` / `tor` (dial-preference over the shared endpoint; censorship resistance, not anonymity) / `tor-isolated` (a second, Tor-only endpoint with its own key, no relays, onion-only discovery; the only leak-free per-network tier). Policy is node-local routing and never touches the blob or protocol.

## Costs and risks

- Two repositories to maintain. Mitigation: tetron deletes whole files and avoids reshaping what it keeps, so torpedo fixes cherry-pick cleanly; the shared history makes `git cherry-pick` from the torpedo remote routine.
- The product rename breaks compatibility — existing tetron nodes and full-torpedo nodes cannot mesh. Existing networks must be recreated after the rename. The ALPN change is a deliberate protocol boundary.
- reconcile.py and the tests/ e2e harness exercise removed features (firewall, dns, invite, files, ssh). Each removal commit must trim the corresponding checks and tests in the same commit to stay green.
- Some peripheral removals touch shared plumbing (DeviceUserMap, stats counters wired through forward.rs) and need care rather than bulk deletion.

## Success criteria

1. `cargo build` produces a tetron binary whose CLI is exactly the surface above.
2. Main crate under ~15,000 lines; Cargo.toml direct dependencies roughly halved; CON-M01 (dependency absence gate) green.
3. Two tetron nodes: create, approve, join, ping over mesh IPs, kick, leave, all green in the trimmed e2e harness.
4. reconcile.py green on every commit; libspec ledger continuous from torpedo's history.
5. No residual `torpedo` strings in user-facing output, CLI help, error messages, or host artifacts (paths, service units, socket names).

## Naming and crate identity

Everything is `tetron`. Cargo package/library `tetron`, helper crate `tetron-proto`, binary `tetron`, service `tetron`, paths under `/etc/tetron` and `/var/run/tetron`, ALPN prefix `tetron/net/...`.

### Rename history

**RENAME-M01 — COMPLETED 2026-07-13.** Crate identity renamed: `rayfish` -> `tetron`, `ray-proto` -> `tetron-proto`. Internal only, D1 preserved.

**RENAME-M02 — COMPLETED 2026-07-13.** Full product identity rename: binary, service, paths, ALPNs, and all user-facing strings from `torpedo` to `tetron`. Severs D1 wire compat. CON-M02 retired.

### Never renamed

The relay preset keyword `"rayfish"` and its URLs (CON-001) — that is the name of upstream's hosted relay/DNS service, not our identity. The MPL-2.0 lineage (LICENSE, upstream author attribution in Cargo.toml). This project is and remains a derivative of rayfish; separation is identity separation, never attribution removal.
