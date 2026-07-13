# Rayfish end-to-end tests

Each scenario provisions real Scaleway instances, deploys `ray` over SSH, drives a
flow end to end, and prints a `PASS`/`FAIL` line per check (exit non-zero on any
failure). The shared SSH/deploy/reset/assert plumbing lives in
[`../lib/`](../lib) and is sourced by every scenario.

## Scenarios

| Dir | Hosts | What it proves |
|-----|-------|----------------|
| [`closed-net/`](closed-net) | 3 | Closed-net admission + lifecycle commands: live approval (`requests`/`accept`/`deny`), co-coordinator (`admin add`) gatekeeper resilience (the co-coordinator admits by approval while the coordinator is offline), `torpedo hostname` propagation, and `torpedo leave`/`nuke`. |
| [`reliability/`](reliability) | 4 | Full-mesh packet-loss test: every pair probed both ways with `ping -c 1000 -i 0.01`, ICMP flood, and iperf3 UDP, over the torpedo tunnel vs the direct public-IP baseline. Fails when torpedo adds loss over the raw link. |
| [`restore-offline/`](restore-offline) | 3 | A member restores and reconnects while the coordinator is offline, proving member reconnect survives a single coordinator being down. |

Everything runs through one dispatcher, [`../e2e.sh`](../e2e.sh):

```bash
tests/e2e.sh <scenario>             # provision (if needed) + deploy + drive + assert
tests/e2e.sh <scenario> provision   # just spin up instances -> <dir>/.servers
tests/e2e.sh <scenario> teardown    # destroy the instances (manual)
```

where `<scenario>` is `closed-net`, `reliability`,
`restore-offline`, or `bench` (run `tests/e2e.sh` with no scenario for usage). The per-scenario run steps live in `<dir>/run.sh`
(still runnable directly once `.servers` exists); the fleet definitions and the
provision/teardown/assert bodies are shared in [`../lib/`](../lib).

The throughput/latency benchmark (`tests/e2e.sh bench`) is a sibling suite
under [`../bench/`](../bench) (same shared `tests/lib/`).

## Prerequisites (all scenarios)

- `scw` authenticated (`scw account project list` should work) and `jq` installed.
- Docker running (used by `cross` for the x86_64-linux build behind `just deploy`),
  plus `just`.
- Your `~/.ssh/id_ed25519` public key registered in the Scaleway account so the
  instances accept `root@<ip>`. Override the key with `SSH_KEY=…`.

## Common environment overrides

| Var | Default | Meaning |
|-----|---------|---------|
| `ZONE` | `fr-par-1` | Scaleway zone (provision) |
| `TYPE` | `DEV1-S` | instance type (provision) |
| `IMAGE` | `ubuntu_jammy` | instance image label (provision) |
| `SSH_KEY` | `~/.ssh/id_ed25519` | private key for `root@<ip>` |
| `KEEP_STATE` | `0` | `1` skips the per-run torpedo state wipe |
