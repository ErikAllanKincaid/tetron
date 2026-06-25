# `firewall` e2e scenario

Three hosts on one closed network `fw` (coordinator `srv-a`, members `srv-b` and
`srv-c`), exercising the firewall surface that the unit tests can't reach end to
end — the coordinator-suggestion pipeline and the per-packet rule matrix over a
real TUN.

## What it proves

| Step | Coverage |
|------|----------|
| 2 | `ray firewall suggest` rides the signed blob → a non-auto-accept member (`srv-b`) sees it in `firewall pending`, `firewall accept` installs it tagged `(suggested by fw)`, and a **blacklist** (denies-only) blocks just the named peer while others stay open. |
| 3 | `--auto-accept-firewall` installs without review; `firewall auto-accept off` makes the next suggestion queue instead; **whitelist** (allow-list) admits the listed peer:port while the node's own inbound default-deny blocks an unlisted port and an unlisted peer (suggestions are additive — no catch-all is synthesized). |
| 4 | Rule matrix: **UDP** allow vs default-deny, a **TCP port range** (8000-8010), and **same-selector replace** (deny→allow on one selector, latest wins — no dead rules). |
| 5 | **Per-network scoping** (`--network`): a `db`-scoped rule does not match `fw` traffic, while the same rule unscoped does; the scope is recorded in `firewall show`. |
| 6 | **File send bypasses the firewall**: with `srv-b` at `default deny` (all inbound TCP/UDP blocked), `ray send srv-a → srv-b` still round-trips — file transfer rides the identity-level `FILES_ALPN` control stream, not TUN/IP traffic, so the per-device firewall never gates it. |

Reachability is checked with real TCP/UDP probes (`fw_allows`/`fw_denies` in
`../../lib/common.sh`); suggestion visibility is polled (reconverge is the 60s
poller or a blob trigger).

## Run

```bash
tests/e2e.sh firewall            # provision (if needed) + deploy + drive + assert
tests/e2e.sh firewall teardown   # destroy the instances
```

See [`../README.md`](../README.md) for prerequisites and environment overrides.
