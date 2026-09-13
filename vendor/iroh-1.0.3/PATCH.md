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

## Patch 2: backfill + ping custom-transport backup paths (VEILID-007, found; not yet a full fix)

**File:** `src/socket/remote_map/remote_state.rs`,
`RemoteStateActor::handle_msg_add_connection`.

**Found:** 2026-09-11, live-verifying `spec/core.py`'s `VeilidCustomPathIrohRaceGap`
(`VEILID-007`) via `tetron-testsuite`'s `veilid-smoke` scenario, after every
gap in tetron's own code (`VEILID-001`..`006`) was already fixed and
confirmed correct.

**Root cause, part 1:** on establishing a connection whose winning path is
not relay, `handle_msg_add_connection` explicitly re-adds any known *relay*
candidate as a backup path ("We may have raced this with a relay address.
Try and add any relay addresses we have back.") -- but has no equivalent
handling for custom-transport (`--tor`/`--veilid`) candidates. On a
shared-LAN topology where Direct wins the connection race in ~1ms, a
Veilid candidate address correctly included in the original `EndpointAddr`
was silently dropped the moment Direct won, and could never appear in
`paths[]` at all -- confirmed live: zero `poll_send` calls on the custom
transport's sender across an entire settle window, on both a manual VM
pair and a clean automated test run.

**Fix, part 1:** widen the relay-only backfill filter to also match
`transports::Addr::Custom(_)`. Live-verified: `open_path_on_conn` now runs
for the Veilid candidate and registers a real `PathId` on the connection.

**Root cause, part 2, found immediately after fixing part 1 and still
seeing zero traffic:** registering a `PathId` only reserves a slot in the
connection; QUIC path validation (`PATH_CHALLENGE`/`PATH_RESPONSE`) only
begins once something actually pings the path. Relay's backup path shows
real activity regardless, because the relay connection carries its own
independent keepalive traffic entirely outside QUIC path validation -- a
custom transport has no equivalent side channel.

**Fix, part 2:** after the backfill loop, ping every relay/custom path
just opened, mirroring `handle_msg_network_change`'s own existing "ping
every path so loss-detection starts ASAP" pattern elsewhere in this same
file. Live-verified: `path.ping()` runs and returns `Ok` for the Veilid
path (no `"failed to ping"` warning) on a `PathId` confirmed registered on
a real, non-immediately-closed connection.

**Status: part 1 stands as originally written; part 2 (the ping call) was
buggy and has been superseded by Patch 3 below -- see that entry for why
and what changed.** Both parts were confirmed doing exactly what they were
meant to, live, at the time this was written (manual single-cycle VM pair,
clean automated `tetron-testsuite` run): `open_path_on_conn` opened a real
`PathId` for the Veilid candidate, and `path.ping()` returned `Ok` with no
`"failed to ping"` warning. What that observation didn't catch: the ping
call as originally placed almost never actually ran against the path that
had just opened (see Patch 3) -- the `Ok` results being logged were real,
they just weren't happening where or as often as this entry assumed at the
time. Investigating why `veilid-smoke` still failed despite both parts
apparently working also produced two further real, independently-necessary
fixes before Patch 3 was found: `VEILID-008`..`010` (roster-propagation and
control-reader gaps in `tetron`'s own code, not this vendored copy) and a
`footgun-nodeid-target` Cargo feature gate in `veilid-core` itself
(`VEILID-011`, `veilid-transport/Cargo.toml`). None of that work was
wasted -- each was independently confirmable and necessary regardless of
Patch 3 -- but none of it was sufficient either, because the actual
remaining defect was back here the whole time, in this file, underneath
all of it.

## Patch 3: ping a newly-opened backup path at its one real point of origin (VEILID-007, actual fix)

**File:** `src/socket/remote_map/remote_state.rs`, `State::open_path_on_conn`
(the ping call moves out of `RemoteStateActor::handle_msg_add_connection`,
which no longer needs any patch-local code of its own).

**Found:** 2026-09-12, after `VEILID-008`..`011` (see `spec/core.py`) were
all live-verified individually correct and `veilid-smoke` *still* failed.
Re-running with Patch 2's own `trace!` calls promoted to `info!` for one
diagnostic session (the same technique used to find Patch 2 itself)
showed the Veilid candidate opening successfully as a real `PathId` --
proving Patch 2's part 1 and `VEILID-011` both correct -- but **zero**
`"backup path ping issued"` or `"failed to ping"` log lines appeared
anywhere, across two separate live sessions (a same-LAN topology and a
Tailscale-adjacent one), despite the path opening every single time.

**Root cause:** Patch 2's ping loop lived inside
`handle_msg_add_connection`, right after that function's *own* call to
`open_path_on_conn`. But that first call essentially always returns
`RemoteCidsExhausted` (`open_path_on_conn`'s `None` branch) -- the
connection is only milliseconds old at that point; the peer has not yet
issued enough connection IDs for a new path. The failed address gets
queued into `pending_open_paths` and is opened successfully *later*, by
`open_path_on_all_conns`'s periodic retry sweep (`scheduled_open_path`,
fired every 333ms) -- a different call site, one call frame away from
`handle_msg_add_connection`, that Patch 2's ping loop never ran for. So
the real-world sequence was: open fails (no ping attempted, correctly --
nothing opened yet), open succeeds moments later on retry (path genuinely
opens -- but nothing ever pings it, because the only ping call site is
back in the function that failed). This matches every observation from
Patch 2's own write-up: `path_id` reliably assigned, zero ping activity,
ever.

**Fix:** move the ping call into `open_path_on_conn` itself, in the
`Some(path_id) =>` arm of `fut.path_id()`'s match -- the one place, shared
by every caller (`handle_msg_add_connection`'s first attempt *and*
`open_path_on_all_conns`'s retry sweep), where a `path_id` is ever newly
assigned. This also let the separate ping loop in
`handle_msg_add_connection` be deleted outright rather than kept
alongside it: every path that loop could have reached had already been
opened (and is now pinged) via this same function.

**Status: live-verified working exactly as designed** -- re-testing (with
`veilid-transport/Cargo.toml`'s `footgun-nodeid-target` feature and a
diagnostic `SafetySelection::Unsafe` switch, see that crate's own
`VeilidTransportBuilder::build`) showed genuine end-to-end delivery for
the first time in this entire investigation: `poll_send` firing with real
data, `AppMessage received` on the far side, the ping probe itself
arriving. This patch is confirmed correct. See Patch 4 below for what
happened next.

## Patch 4: idle timeout for custom-transport backup paths (VEILID-014)

**File:** `src/socket/remote_map/remote_state.rs`, `State::open_path_on_conn`
(same `Some(path_id) =>` arm Patch 3 added the ping to).

**Found:** 2026-09-12, immediately after Patch 3's own live re-verification
showed real Veilid delivery working, then disappearing: `tetron status`'s
final check, run roughly 90 seconds after the last confirmed
`AppMessage`, showed no Veilid path at all -- not present with zero
activity, simply gone, with no disconnect or error logged in between.

**Root cause:** `noq_proto`'s own `connection::paths::PathStatus::Backup`
doc comment: *"If the `max_idle_timeout` is specified the path will be
kept alive so that it does not expire."* The unstated alternative is that,
without one, it does. `register_and_configure_path` sets
`RELAY_PATH_MAX_IDLE_TIMEOUT` (30s) for a connection's *primary* path when
it is relay -- but nothing in this vendored copy ever set anything for a
*backup* path of any kind, relay included. Relay backup paths have simply
never needed it: they are kept alive incidentally by the relay
connection's own protocol-level keepalive, entirely outside QUIC path
validation. A Veilid backup path has no equivalent side channel.

**Fix:** at the same point Patch 3 pings a newly-opened path, also call
`path.set_max_idle_timeout` for `transports::FourTuple::Custom` addresses
specifically. Deliberately does **not** reuse `RELAY_PATH_MAX_IDLE_TIMEOUT`'s
exact value: a new `CUSTOM_TRANSPORT_PATH_MAX_IDLE_TIMEOUT` (300s) gives
comfortable margin over the 30-90s gaps observed live between real
traffic bursts on a Veilid backup path -- a first attempt reusing relay's
30s value was confirmed, by inches (within a handful of seconds), still
too short.

**Status: live-verified.** With this fix, a Veilid backup path survived a
full settle window and showed `has_activity: true` in
`tetron status --json`'s `paths[]` -- the first time this entire
investigation produced that result. (It reported as `conn_type: "Tor"`, a
separate, independent bug in `tetron`'s own status code, not this
vendored copy -- see `spec/core.py`'s `VeilidStatusMislabeledAsTor`,
VEILID-015.)

**Open, not a bug in this file:** getting this far required a diagnostic
`SafetySelection::Unsafe` switch in `veilid-transport` (tetron's own
crate, not vendored) -- the intended default, `SafetySelection::Safe`,
has never once delivered a message across this entire investigation,
despite `poll_send` reporting local success every time. Whether to ship
with `Unsafe` or keep pursuing `Safe` is a deliberate product decision,
tracked in `spec/core.py`'s `VeilidCustomPathIrohRaceGap` (VEILID-007)
UPDATE 6, not resolved by this patch.

## Patch 5: re-queue an abandoned custom-transport backup path for another open+ping attempt (VEILID-016)

**Files:** `src/socket/remote_map/remote_state.rs` --
`RemoteStateActor::handle_path_event`'s `Established`/`Abandoned` arms,
`State::open_path_on_conn`'s `Some(path_id)` arm, and a new
`State::unvalidated_paths` field.

**Found:** 2026-09-12, after VEILID-007 was declared closed --
re-running `tetron-testsuite`'s `veilid-smoke` repeatedly against the
same commit, no code changes, surfaced a real ~1-in-3 failure rate. A
dedicated diagnostic loop (`tetron/DO-NOT-COMMIT/veilid-flake-diag.sh`,
kept per that repo's own evidence-preservation policy, not deleted)
reproduced it five separate times across the diagnosis below, each
capture correcting the previous session's working theory.

**Root cause, layer 1:** `VeilidCustomSender::poll_send`
(`veilid-transport/src/lib.rs`, tetron's own crate, not this vendored
copy) is fire-and-forget: it spawns the async `app_message` call and
always returns `Poll::Ready(Ok(()))` to noq, regardless of whether the
send actually succeeds. Patch 3's `path.ping()` call fires exactly once
at path-open time. Live captures showed that specific `app_message`
send failing for two genuinely transient reasons -- `"No connection: no
routing domain"` (a brief offline/online blip in Veilid's own network
detection during early attach) and `"No connection: could not resolve
node id"` (DHT resolution of the peer's route, observed failing for
tens of seconds even after this node's own attach had already
completed). Because `poll_send` lies about success, noq never learns
the challenge was lost and eventually abandons the path.

**Root cause, layer 2 (first fix attempt, wrong target):** the initial
retry condition matched `PathAbandonReason::TimedOut`, which never
actually fires here. `open_path_on_conn` only runs client-side (`if
conn.side().is_server() { return; }`), so Patch 4's
`set_max_idle_timeout` override never reaches the *peer's* copy of the
path -- the peer (server side for that connection direction) times out
on its own shorter default first and sends a PATH_ABANDON frame, which
arrives here as `PathAbandonReason::RemoteAbandoned` carrying
`TransportErrorCode::PATH_UNSTABLE_OR_POOR` (confirmed live via the raw
wire code, `15990` = `0x3e76`).

**Root cause, layer 3 (second fix attempt, still not running):** even
after widening the match to `RemoteAbandoned{PATH_UNSTABLE_OR_POOR}`,
live captures showed the retry code still never executing.
`noq_proto::PathEvent::Abandoned`'s own doc comment states it directly:
*"this may be the first event for a path: if a path is abandoned before
having been established, no `Established` event is emitted."*
`ConnectionState::paths` -- what the retry code looked up the address
from -- is only ever populated by `register_and_configure_path`, called
*only* from the `Established` handler. A path abandoned before ever
validating (exactly the case here) was never in that map, so the lookup
silently failed every time.

**Fix:** a new `State::unvalidated_paths: FxHashMap<(ConnId, PathId),
FourTuple>` records a `Custom`-transport path's address at open+ping
time (in `open_path_on_conn`'s success arm), independent of whether it
ever reaches `Established`. In `handle_path_event`'s `Abandoned` arm,
the address is recovered from `ConnectionState::remove_path` (the
normal case) or, failing that, from `unvalidated_paths`. When the
reason is `TimedOut` or `RemoteAbandoned{PATH_UNSTABLE_OR_POOR}` and the
address is `Custom`, it is re-queued into the same
`pending_open_paths`/`scheduled_open_path` mechanism Patch 1/3 already
use -- a lost ping just becomes another open+ping attempt.
`unvalidated_paths` entries are cleared on `Established` too, so a path
that validates normally never lingers in both maps. Relay is exempt:
its own protocol keepalive already gives it real traffic outside QUIC
path validation, so it never hit this failure mode. No retry cap,
matching this file's existing indefinite-retry posture for
holepunching; each cycle costs at most one small datagram.

**Status: live-verified.** Once the retry logic was confirmed actually
running, two more captures surfaced one final real fact rather than a
code bug: worst-case Veilid path validation can legitimately take ~50+
seconds, and a retry needs a similar window of its own -- the
diagnostic loop's original 120s post-restart settle sometimes ended
mid-retry, with the path still pending rather than validated or
exhausted. `tetron-testsuite`'s own `tests/veilid-smoke.sh` had its
matching `TESTSUITE_VEILID_RESETTLE_SECS` raised from 120s to 240s for
the same reason.

**Honest final measurement, not just a declared fix:** a larger,
unattended batch at the 240s window (8 attempts, no early stop) passed
6 of 8 (75%) -- a real improvement over the pre-fix baseline, not full
elimination. The 2 failures were two different shapes. One matched this
patch's own target exactly: the retry fired 4 separate times, each
attempt failing the same way (`RemoteAbandoned{PATH_UNSTABLE_OR_POOR}`)
-- reads as genuinely sustained Veilid route-resolution unavailability
to that specific peer for the whole window, not a defect in the retry
logic itself. The other is a distinct, unexplained case: the path
reached noq's own `Established` event (meaning it validated) and then
still vanished with **no abandon event ever logged**, despite
confirming `open_path_on_conn`'s ping/idle-timeout branch did run for
it. Left open rather than blocking this patch -- possibly related to
`register_and_configure_path` (the code path for a path that validates
*asynchronously*) never applying the `CUSTOM_TRANSPORT_PATH_MAX_IDLE_TIMEOUT`
override the way `open_path_on_conn`'s own synchronous success arm
does, but unconfirmed against this specific capture.

## Patch 6: re-export `PathSelector` and widen `BiasedRttPathSelector` to `pub` (PATHPREF-001)

**Files:** `src/lib.rs` (new re-exports), `src/socket/biased_rtt_path_selector.rs`
(`pub(crate)` → `pub` on the struct's own declaration).

**Found:** 2026-09-13, scoping `PATHPREF-001` (a tetron feature letting a
user force a specific transport to actually carry application data).
`Endpoint::path_selector(self, selector: Arc<dyn PathSelector>) -> Self`
(`src/endpoint.rs`) is a public builder method, but `PathSelector` and its
supporting types (`PathSelection`, `PathSelectionContext`,
`PathSelectionData`, `AddrKind`, `FourTuple`) live in `mod socket;`
(private) / `pub(crate) mod remote_map;` (crate-only) — unreachable from
outside this crate as shipped, with no re-export anywhere. Confirmed live
by attempting to reference `iroh::socket::remote_map::PathSelector` from
tetron's own crate: `error[E0603]: module 'socket' is private`.

The trait's own `#[cfg_attr(not(feature = "unstable-custom-transports"),
allow(unreachable_pub))]` attribute suggests this was meant to become
reachable once that feature is enabled (which tetron already does for Tor)
— reads as an incomplete public surface in this vendored version, not a
deliberate "keep this private."

**Fix:** a narrow `pub use` block in `lib.rs` re-exporting exactly
`PathSelection`, `PathSelectionContext`, `PathSelectionData`,
`PathSelector`, `AddrKind`, `FourTuple` — not a broader `pub mod socket`,
so nothing else in that module tree becomes externally visible.
Separately, `BiasedRttPathSelector` (iroh's own default selector) is
declared `pub(crate)` on the struct itself, a stronger restriction a
`pub use` alone cannot cross — widened to plain `pub` (fields stay
module-private; only construction via `Default` and the `PathSelector`
impl become externally usable) so a custom selector can delegate to
iroh's *real* RTT-tuning logic for its "no preference set" case instead
of reimplementing tuning (switching thresholds, per-`AddrKind` biases)
that could silently drift out of sync with iroh's own.

**Tradeoff, accepted deliberately (not free):** this relies on iroh's
internal API shape rather than its documented public contract — none of
these types carry iroh's own semver guarantees, and a future iroh version
bump could change or remove any of them without warning. Weighed against
the alternatives (forking/reimplementing iroh's socket layer entirely, or
not building the feature) and accepted; see `spec/core.py`'s
`TransportPathPreference` (PATHPREF-001) for the full reasoning.

**Status: live-verified.** `tetron_path_selector`'s own unit tests pass
(classification logic, both with and without the `veilid` feature); a
full `tetron-testsuite` `core-smoke` run (create → join → status shows
peer → leave → gone) passed with the new selector wired into real
endpoint construction, confirming default (`auto`) behavior is unchanged
from before this patch.

## Patch 7: periodic unconditional `select_path()` re-evaluation (PATHPREF-001)

**File:** `src/socket/remote_map/remote_state.rs` -- a new
`RESELECT_PATH_INTERVAL` constant, a new `reselect_path` ticker in
`RemoteStateActor::run`'s event loop, and one new `select!` arm.

**Found:** 2026-09-13, live-verifying Patch 6's own `PATHPREF-001`
feature end to end (`DO-NOT-COMMIT/pathpref-veilid-live-check.sh`). With
Direct already selected and a validated Veilid backup path present,
`tetron config set path-preference veilid` reported "applied immediately"
(the IPC live-reload round-trip worked, confirmed via the shared
`ArcSwapOption` value), but `conn_type` stayed `Direct` -- the raw
`paths[]` array showed `is_selected: false` on the Veilid entry too,
proving this was not a status-display bug but the real `PathSelector`
never actually reconsidering its choice.

**Root cause:** `select_path()` -- the only function that calls
`PathSelector::select()` -- has exactly three call sites, all reactive:
a path becoming `Established`, a path being `Abandoned`, and one other
event-driven path. There was no periodic or on-demand trigger. On an
already-stable connection where nothing else changes, a `PathSelector`'s
own criteria changing (a live-reloaded preference, PATHPREF-001; in
principle also an RTT drift under `auto` mode with no new path event)
had no way to ever get re-evaluated.

**Fix:** a fourth call site -- a new `time::interval(RESELECT_PATH_INTERVAL)`
(3s) ticker in the actor's own `tokio::select!` loop, alongside the
existing `check_connections`/holepunch/path-open timers, calling
`self.select_path()` unconditionally on each tick. `select_path()` is a
pure read of already-cached path stats (no I/O), so this costs
negligibly more than the ticker itself, run per remote.

**Status: live-verified.** Re-ran the same live check after this fix:
`tetron config set path-preference veilid` correctly flipped `conn_type`
to `Veilid` within the check's ~10s wait (well inside the 3s tick
interval's margin), and `tetron config unset path-preference` correctly
reverted it to `Direct`. See `spec/core.py`'s `TransportPathPreference`
(PATHPREF-001) for the full account, including the first (failed) attempt.
