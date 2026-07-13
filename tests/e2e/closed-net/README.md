# `closed-net` e2e scenario

Three hosts on a closed network `priv` (coordinator `srv-a`, members `srv-b` and
`srv-c`), exercising the admission + lifecycle command surface the other
scenarios don't cover.

## What it proves

| Step | Coverage |
|------|----------|
| 2 | **Live approval** with no invite: `srv-b` dials the closed net → `torpedo requests` shows it → `torpedo accept` admits it. |
| 3 | **Live denial**: `srv-c` dials → `torpedo deny` rejects it → it never becomes a member. |
| 4 | **Co-coordinator grant**: `torpedo admin add` promotes `srv-b`; `torpedo admin list` shows two key-holders. |
| 5 | **Gatekeeper resilience**: with `srv-a` taken offline (`torpedo down`), the co-coordinator `srv-b` admits `srv-c` by live approval (`requests`/`accept`) — proving any network-key holder can gatekeep. |
| 6 | **Hostname change**: `torpedo hostname` propagates to the coordinator's roster; srv-bb is then reachable from srv-c by its mesh IP (Magic DNS removed in tetron). |
| 7 | **Graceful leave + nuke**: `torpedo leave` prunes the member promptly; `torpedo nuke` drops the network. |

## Run

```bash
tests/e2e.sh closed-net            # provision (if needed) + deploy + drive + assert
tests/e2e.sh closed-net teardown   # destroy the instances
```

See [`../README.md`](../README.md) for prerequisites and environment overrides.
