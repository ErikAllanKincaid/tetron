# Security Policy

## Supported versions

Tetron is a personal, pre-release fork of rayfish — no versioned releases
have been published yet, and there is no formal backport policy. Report
against the current `main`.

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
  key published to the DHT — on a closed network it is **not** sufficient to
  join. Admission runs through the coordinator via single-use invites or live
  approval.
- **Signed group state.** The per-network pkarr record is signed by the network
  secret key (the pkarr address *is* the network public key), so the `GroupBlob`
  and the firewall suggestions that ride in it are MITM-resistant. Suggested
  firewall rules are consumed only from the verified blob, never from a peer
  control message.
- **Local privilege.** The daemon authorizes each IPC request by the caller's
  UID (`SO_PEERCRED`), not by socket file permissions. Mutating commands require
  root or the configured operator.
- **Secrets at rest.** Invite ledgers are written `0600`; invite secrets are
  stored only as blake3 hashes; identity backups are encrypted (argon2 +
  chacha20poly1305). `tetron status --json` provides the same information.
  secret keys.
