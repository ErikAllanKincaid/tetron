# Security Policy

## Supported versions

Tetron is a personal, actively-developed fork of rayfish. Tagged releases exist (`v0.1.0` through the current `v0.8.2`), but there is no formal backport policy, so report against the latest tag or current `main`.

## Reporting a vulnerability

Please report security vulnerabilities **privately**; do not open a public GitHub issue.

Use [GitHub private vulnerability reporting](https://github.com/ErikAllanKincaid/tetron/security/advisories/new) on this repository.

Include enough detail to reproduce: affected version/commit, configuration, and a description (ideally a proof of concept). Reports will be acknowledged, kept updated on remediation, and credited in the release notes unless you prefer to remain anonymous.

## Security posture

The properties tetron actually provides, for anyone evaluating whether it fits their threat model:

- **Encrypted communication, always.** Every peer connection, data plane and control plane alike, rides iroh's QUIC transport, which is end-to-end encrypted between the two peers' cryptographic identities. This isn't a toggle: there is no unencrypted mode. A relay, when one is needed for NAT traversal, forwards opaque encrypted QUIC datagrams and cannot read the traffic it relays.
- **Identity-based, not location-based.** Every peer is a persistent Ed25519 keypair (`EndpointId`), not an IP address. Mesh IPv4/IPv6 addresses are deterministically derived from that identity plus the network; the QUIC handshake authenticates the connection to the identity, so a peer cannot impersonate another peer's identity without its private key.
- **Key-based, invite-only entry.** There is no open-network mode and no password-style shared secret. A network's room id (its public key) is a *discovery* pointer only, published to the DHT; knowing it, even publicly, grants no access. The only way onto a network is a single-use invite key minted by a coordinator (`tetron invite create`, or the one auto-minted on `tetron create`), redeemed against the signed roster. Any coordinator can mint invites, so admission has no single point of failure.
- **Peer-to-peer, no data-plane server dependency.** Once a network exists, traffic between members flows directly (hole-punched) or via relay for NAT traversal; no tetron-operated server sits in the data path or brokers traffic. The only shared infrastructure is the DHT (pkarr) used purely for discovery: it stores a signed pointer to where to find a network's current roster, nothing about who is actually talking to whom or what is in the traffic.
- **Signed, tamper-evident group state.** The per-network pkarr record is signed by the network's own secret key, and its address *is* that key's derived public key, so the `GroupBlob` (membership roster, admin list, invites) cannot be forged or substituted by anyone without the key, including whoever operates the DHT/relay infrastructure. This is integrity, not confidentiality: the roster is verifiable, not secret, from anyone who has the room id.
- **Small attack surface by design.** tetron deliberately strips the feature classes most likely to carry their own vulnerabilities in this space: no userspace firewall to misconfigure, no OS DNS mutation, no embedded SSH server, no self-update mechanism, no file-transfer protocol. What the always-root daemon actually owns is one TUN device per joined network, a handful of routes, one shared QUIC endpoint, and a Unix socket. See `AGENTS.md` for the full list of what was removed and why.

## Security model (context for reviewers)

A few more nuanced, load-bearing properties and their actual limits, so reports can be scoped accurately:

- **No packet-level filtering; the network split is the boundary.** tetron has no built-in firewall: within a shared network, every member reaches every port any other member's host binds. Access control is entirely "do we share a network," not per-port/per-peer. Restrict what's actually reachable with the host firewall (nftables/ufw) on the TUN interface. The daemon's own only inbound check is anti-spoofing: a peer may only source packets claiming its own assigned mesh IP.
- **Local privilege.** The daemon authorizes each IPC request by the caller's UID (`SO_PEERCRED`), not by socket file permissions; the socket itself is mode `0666`. Read-only commands (`status`, `admin list`, `invite list`, `sync`) are open to any local user; commands that mutate state require root or the configured operator (`sudo tetron set-operator <user>`).
- **Control-plane abuse resistance.** Every inbound control message is gated by a per-connection rate limiter and a shared daemon-wide one; a sustained flood gets that connection closed, not just throttled.
- **Secrets at rest, and their actual limits.** Both the node's own identity key (`<config_dir>/secret_key`) and each joined network's secret key (`networks/<name>.toml`) are stored as **plaintext hex**, in `0600 root:root` files. There is no passphrase, KDF, or encryption at rest. That file permission is an in-OS control only: it does nothing against physical or offline access (a pulled drive, a live-USB boot, a disk image). A network's secret key *is* that network's full coordination authority (the signing key behind the roster, invites, kicks, and nuke), so whoever extracts it can act as any coordinator, indistinguishably. Nothing here is backed up automatically; see `README.md`'s Backup section for the manual procedure (and encrypt the archive yourself, e.g. with `age`, before it leaves the machine).
- **Kicking is not key revocation.** Granting co-coordinator status (`tetron admin add`) hands out a copy of the *same* shared network secret key. Removing a coordinator via `tetron kick` drops them from the roster and every honest node's enforcement, but does not invalidate their copy of the key, so a still-live, uncooperative former coordinator can self-mint an invite and rejoin. Treat `kick` as roster maintenance, not access revocation; there is currently no way to rotate a network's secret key short of destroying and recreating the network (`tetron nuke`).
