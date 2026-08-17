# Local patches

**Upstream:** https://github.com/n0-computer/iroh (crate `iroh`). Not
reported upstream yet.

## Patch 1: dedupe `pending_open_paths` before `push_back` (PATH-DIAG-008)

**File:** `src/socket/remote_map/remote_state.rs`,
`RemoteStateActor::open_path_on_conn`'s `RemoteCidsExhausted`/
`MaxPathIdReached` failure branch.

**Found:** 2026-08-16, during tetron's own OOM-leak investigation
(`tetron/DO-NOT-COMMIT/oom-leak-investigation/`), by a size-filtered
`realloc` uprobe on a live-bursting daemon: six reallocations above 32 MB
in 17 seconds, every one the same stack, bottoming out in
`VecDeque<FourTuple>::grow` inside this function.

**Root cause:** on `RemoteCidsExhausted`/`MaxPathIdReached`,
`open_path_on_conn` unconditionally pushes the failing address onto
`State::pending_open_paths` (a plain `VecDeque<FourTuple>`, no dedup, no
bound) -- once per connection that fails on that address. A 333ms timer
then drains the whole queue and retries every popped address against
*every* live connection to the remote peer
(`RemoteStateActor::open_path_on_all_conns`), regardless of which
connection originally queued it. So an address failing on C connections
queues C identical copies; next tick, each of those C copies fans back out
to C connections again. The queue multiplies by C every 333ms for as long
as the failure condition holds -- geometric, not linear. C=1 is a fixed
point (pop one, push one back: stable); C=8 (an 8-peer coordinator in the
reproduction harness) multiplies the queue eightfold per tick, producing
the observed 40 -> 80 -> 160 -> 320 MB doublings within single-digit
seconds -- the memory bursts that drove the whole investigation.

**Fix:** a dedup guard immediately before the `push_back` --
`if !self.pending_open_paths.contains(open_addr) { push_back(...) }`.
`FourTuple` already derives `PartialEq, Eq, Hash`
(`src/socket/transports.rs:975`), so the check is a cheap structural
comparison. This loses no retry coverage: `open_path_on_all_conns` already
retries every distinct candidate against every live connection
unconditionally on every tick, so the duplicate queue entries this patch
removes never carried distinct per-connection state to begin with -- they
were pure amplification. The dedup converts the growth from geometric
(multiply by C per tick) back to bounded (at most one entry per distinct
candidate address ever in flight at once), matching the C=1 fixed-point
behavior for any C.

**Not a fix for CID exhaustion itself** -- the underlying condition
(a connection running out of remote-issued connection IDs under path
churn) is unchanged and expected QUIC behavior under load; this patch only
removes the consequence of retrying it via an unbounded queue. Two other
fix shapes were considered and rejected in favor of this one -- pushing
once per address via a restructured call site (same resulting bound, but
a larger diff touching `open_path_on_conn`'s signature and the caller),
and bounding the queue with a fixed cap (does not fix the mechanism: under
geometric growth any practical cap saturates almost immediately, so the
burst still happens up to the cap, and whichever candidates get dropped
past it are silently never retried). Full reasoning:
`tetron/DO-NOT-COMMIT/oom-leak-investigation/
PLAN_VendoredIrohDedupePatch_ChoicesSequenceReasons_2026-08-17.md`.

Full root-cause analysis and evidence:
`tetron/DO-NOT-COMMIT/oom-leak-investigation/aorus-tracking/taskcensus/
ROOTCAUSE_IrohPendingOpenPaths_2026-08-16.md`.

**Status: live-verified 2026-08-17.** Two independent coordinators (8
members each, 45s-down/45s-up synchronized churn, no `MemoryMax` anywhere),
uncapped, run 4 hours -- well past the 25-62 minute onset window observed
pre-patch across three coordinators in the original investigation. Both
arms: zero burst-watchdog triggers, RSS deltas throughout in the tens to
low-thousands of kB (three to four orders of magnitude below the
40,000-320,000 kB doublings that defined the original bug), while the CID-
exhaustion trigger itself fired thousands of times (4,342 / 6,016
`scheduling open_path` events) -- so this is a clean pass against a
genuinely, repeatedly exercised failure condition, not a quiet run.
Protocol: `tetron/DO-NOT-COMMIT/oom-leak-investigation/
PLAN_PatchVerification_PATH-DIAG-008_UncappedReproHarness_2026-08-17.md`.

Separately, and NOT a regression in this patch: both arms' RSS showed a
much smaller (tens of kB/min) climb underneath the (absent) burst signature
-- one arm decelerating over the run (consistent with settling), the other
holding a roughly constant ~25 kB/min rate for three of the four hours with
no further decay. Four to five orders of magnitude smaller/slower than
this patch's target mechanism, and not conflated with it here -- tracked
as its own open item,
`tetron/DO-NOT-COMMIT/TODO_DETAILS.md#slow-climb-leak-post-burst-patch`.
