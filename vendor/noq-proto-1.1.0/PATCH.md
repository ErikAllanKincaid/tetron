# Local patches

**Upstream:** https://github.com/n0-computer/noq (crate `noq-proto`, pulled
in via `iroh`). Not reported upstream yet.

## Patch 1: `failed closing path` demoted from `warn!` to `debug!` (LOG-005)

**File:** `src/connection/mod.rs`, `Timer::PerPath` / `PathTimer::PathIdle`
handler.

**Found:** 2026-08-13/14, during tetron's own OOM-leak investigation
(`tetron/DO-NOT-COMMIT/oom-leak-investigation/`), tracing a sustained
journal warning storm on a real, in-use machine (xps-17-9720): up to
~28 occurrences/minute, effectively continuous.

**Root cause:** every `PathTimer::PathIdle` tick calls `close_path_inner`
with the multipath-specific close API, regardless of whether multipath
was ever actually negotiated on that connection. For an ordinary
single-path connection -- the common case, not an anomaly -- this always
fails with `ClosePathError::MultipathNotNegotiated`, and the caller logs
that failure at `warn!` every single time, with nothing rate-limiting or
debouncing it. Confirmed live that this is unrelated to connection
health: a currently healthy, `Direct`-connected peer generates the exact
same warning repeatedly, at the same rate as peers with no working
connection at all.

**Not fixed at the root** (that would mean either not arming
`PathTimer::PathIdle` at all for non-multipath connections, or having it
call a non-multipath close API instead -- a real behavior change in
noq-proto's own timer/path-management logic, out of scope for a
one-line local patch) -- this patch only demotes the resulting log line
from `warn!` to `debug!`, since at `warn!` it was drowning out real
signal in the journal for any tetron user with normal network conditions
(NAT'd, no multipath capable path, etc.), not just this investigation's
own real-hardware test machines. `debug!` still preserves the line for
anyone bumping tetron's file-log level up to actually diagnose path
behavior -- console/journal output is unconditionally `info` regardless
(`LOG-003`), so this is what actually removes it from journalctl.

Full analysis: `tetron/DO-NOT-COMMIT/oom-leak-investigation/
FINDINGS_MemoryLeak_xps-17-9720_HostEnvironment_and_ConnectionChurn_2026-08-13.md`.
