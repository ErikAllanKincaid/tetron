# Security Policy

## Supported versions

Tetron is a personal, actively-developed fork of rayfish. Tagged releases exist
(`v0.1.0` through the current `v0.8.2`), but there is no formal backport policy —
report against the latest tag or current `main`.

## Reporting a vulnerability

Please report security vulnerabilities **privately** — do not open a public
GitHub issue.

Use [GitHub private vulnerability reporting](https://github.com/ErikAllanKincaid/tetron/security/advisories/new)
on this repository.

Include enough detail to reproduce: affected version/commit, configuration, and
a description (ideally a proof of concept). Reports will be acknowledged, kept
updated on remediation, and credited in the release notes unless you prefer to
remain anonymous.

## Security model (context for reviewers)

A few load-bearing properties, so reports can be scoped accurately:

- **Identity, not IP.** Peers are addressed by cryptographic identity
  (EndpointId); virtual addresses are derived from the identity and transport is
  end-to-end encrypted by iroh.
- **Discovery vs. admission.** A network's room id (public key) is a *discovery*
  key published to the DHT — it is **never** sufficient to join on its own.
  tetron is invite-only: a bare room-id join is always denied. Admission
  requires a single-use invite key minted by a coordinator (`tetron invite
  create`, or the one auto-minted on `tetron create`), validated against the
  signed `GroupBlob` by whichever coordinator the joiner dials. Any coordinator
  can mint invites — there is no single point of failure for admission.
- **Signed group state.** The per-network pkarr record is signed by the network
  secret key, and the pkarr address *is* the network's public key, so the
  `GroupBlob` (membership roster, admin list, invites) can't be spoofed —
  a node verifies the signature before trusting anything in it.
- **No packet-level filtering — the network split is the boundary.** tetron
  has no built-in firewall: within a shared network, every member reaches
  every port any other member's host binds. Access control is entirely
  "do we share a network," not per-port/per-peer. Restrict what's actually
  reachable with the host firewall (nftables/ufw) on the TUN interface. The
  daemon's own only inbound check is anti-spoofing: a peer may only source
  packets claiming its own assigned mesh IP.
- **Local privilege.** The daemon authorizes each IPC request by the caller's
  UID (`SO_PEERCRED`), not by socket file permissions — the socket itself is
  mode `0666`. Read-only commands (`status`, `admin list`, `invite list`,
  `sync`) are open to any local user; commands that mutate state require root
  or the configured operator (`sudo tetron set-operator <user>`).
- **Control-plane abuse resistance.** Every inbound control message is gated
  by a per-connection rate limiter and a shared daemon-wide one; a sustained
  flood gets that connection closed, not just throttled.
- **Secrets at rest, and their actual limits.** Both the node's own identity
  key (`<config_dir>/secret_key`) and each joined network's secret key
  (`networks/<name>.toml`) are stored as **plaintext hex**, in `0600
  root:root` files — there is no passphrase, KDF, or encryption at rest.
  That file permission is an in-OS control only: it does nothing against
  physical or offline access (a pulled drive, a live-USB boot, a disk image).
  A network's secret key *is* that network's full coordination authority
  (the signing key behind the roster, invites, kicks, and nuke) — whoever
  extracts it can act as any coordinator, indistinguishably. Nothing here is
  backed up automatically; see `README.md`'s Backup section for the manual
  procedure (and encrypt the archive yourself, e.g. with `age`, before it
  leaves the machine).
- **Kicking is not key revocation.** Granting co-coordinator status
  (`tetron admin add`) hands out a copy of the *same* shared network secret
  key. Removing a coordinator via `tetron kick` drops them from the roster
  and every honest node's enforcement, but does not invalidate their copy of
  the key — a still-live, uncooperative former coordinator can self-mint an
  invite and rejoin. Treat `kick` as roster maintenance, not access
  revocation; there is currently no way to rotate a network's secret key
  short of destroying and recreating the network (`tetron nuke`).
