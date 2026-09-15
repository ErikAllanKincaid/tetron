# Local patches

**Upstream:** https://github.com/n0-computer/iroh-tor-transport (crate
`iroh-tor-transport`). Not reported upstream yet.

## Patch 1: bound the SOCKS5 connect + surface send failures (TOR-DIAL-001 follow-up)

**Files:** `src/lib.rs` -- `TorPacketSender::get_or_connect` (new
`CONNECT_TIMEOUT` wrap) and `TorCustomSender::poll_send`'s fire-and-forget
spawn (new `warn!` on failure).

**Found:** 2026-09-14/15, live-verifying `tetron`'s own `TOR-DIAL-001`
fix (`transport.rs`'s `connect_to_peer_with_alpn` now correctly offers a
Tor candidate address to iroh at dial time, and a separate local patch to
vendored `iroh` -- see `vendor/iroh-1.0.3/PATCH.md` -- correctly retries a
backup path abandoned with `PathAbandonReason::UnusableAfterNetworkChange`).
Even with both of those fixes, `tetron-testsuite`'s `tor-smoke.sh` still
never showed a Tor path entry in `tetron status --json`'s `paths[]` at
all. A 480-second soak with `trace!` calls in vendored iroh promoted to
`info!` for one diagnostic session (the same technique used throughout
this project's VEILID-007 investigation) showed the Tor backup path
opening, `path.ping()` reporting success, and then **nothing** for the
entire 300-second idle-timeout window (`CUSTOM_TRANSPORT_PATH_MAX_IDLE_TIMEOUT`,
`vendor/iroh-1.0.3/PATCH.md`) before being abandoned as `TimedOut` and
retried -- repeating indefinitely, never once reaching `Established`.

**Root cause:** `TorPacketSender::send` (called from `poll_send`'s spawned,
fire-and-forget task) calls `get_or_connect`, which on a cache miss awaits
`Socks5Stream::connect(socks_addr, (onion_addr, onion_port))` with **no
timeout at all**. If the underlying Tor circuit build (rendezvous +
introduction to the peer's hidden service) stalls -- plausible under load,
and especially plausible in a resource-constrained VM (`tetron-testsuite`'s
own topology is a 512MB/1vCPU box) -- this future can hang indefinitely.
Because `poll_send` reports `Ready(Ok(()))` immediately regardless (the
correct, required behavior for a non-blocking `CustomSender`) and the
spawned task's own result was discarded (`let _ = sender.send(...).await`),
a stuck connect was **completely invisible**: no error, no log line,
nothing -- the QUIC PATH_CHALLENGE this send was meant to carry simply
never left the process, and the path just sat waiting for a PATH_RESPONSE
that could never arrive, until QUIC's own idle timeout eventually gave up.

**Fix:**
- Wrap the connect in `tokio::time::timeout(CONNECT_TIMEOUT, ...)`
  (30s -- generous for a real Tor circuit build, per Tor's own typical
  circuit-build-timeout order of magnitude, while still leaving room for
  several retries inside the existing 300s idle-timeout budget rather
  than one attempt consuming all of it).
- Log a `warn!` on any send failure (including the new timeout) instead of
  silently discarding it -- purely observability, no behavior change to
  the fire-and-forget contract itself.

**Status: live-verified working as designed.** Once actually exercised,
the connect consistently failed fast (a few seconds, well under the 30s
bound) with `Host unreachable` rather than hanging -- the timeout itself
was never actually needed to unstick anything in this environment, but
the `warn!` visibility it came with directly enabled diagnosing Patch 2
below. Kept as a real, independently-justified robustness fix regardless
(an unbounded external-process connect with no visibility on failure is
a latent bug on its own terms).

## Patch 2: wait for `HS_DESC UPLOADED` before returning from `build()`

**Files:** `src/lib.rs` -- `TorCustomTransportBuilder::build` (new
`HS_DESC_PUBLISH_TIMEOUT`/`HS_DESC_POLL_INTERVAL` constants, an
`AsyncEvent` handler registered via `set_async_event_handler`, a
`set_events(false, &mut ["HS_DESC"].into_iter())` subscription, and a
poll loop after `add_onion_v3` waiting for at least one matching
`UPLOADED` line).

**Found:** 2026-09-15, continuing Patch 1's investigation. With Patch 1's
`warn!` in place, `Host unreachable` fired consistently within seconds of
every single dial attempt, immediately after `Hidden service created` --
too fast and too consistent for a slow circuit build. Researching Tor's
own control-spec confirmed the mechanism: `ADD_ONION`'s `250 OK` only
confirms the service was created *locally* on this Tor process; the
descriptor upload to the HSDir network (what makes the service actually
*findable* by another client) is a genuinely separate, asynchronous step,
signaled by the `HS_DESC` control-protocol event's `UPLOADED` action.
`build()` had no subscription to this event at all -- callers (this
crate's own tests, `tetron`, and almost certainly `rayfish`'s original
integration, which shares the identical gap) start dialing immediately
after `build()` returns, with no guarantee the descriptor has reached the
network yet.

**Implementation notes:** `torut::control::AuthenticatedConn` has no
dedicated "wait for the next async event" call -- events are only
drained and dispatched to the registered handler as a side effect of
reading a response to some other command. Since nothing else uses this
connection after setup (it exists purely to keep the ephemeral service
alive for as long as it's held), a `noop()` call every
`HS_DESC_POLL_INTERVAL` (500ms) is used purely to pump that read loop
promptly, up to `HS_DESC_PUBLISH_TIMEOUT` (180s, generous headroom over
the single-digit-seconds this project has actually observed in practice).
A failure to subscribe at all (e.g. an older Tor without this event) is
non-fatal -- logs a `warn!` and falls back to the pre-patch behavior
(return immediately, no confirmation) rather than blocking hidden-service
creation on it.

**Status: live-verified working, but insufficient on its own -- see Patch 3
below, which found and fixed why.** This patch initially appeared to
silently no-op: `set_events` was failing every time with
`ConnError::InvalidEventName`, a real bug in `torut` itself (see
`vendor/torut-0.2.1/PATCH.md`) -- unrelated to this crate, but blocking
this patch from ever actually subscribing. Once that was fixed, both nodes
in a live two-VM test reliably confirmed `UPLOADED` within single-digit
seconds of `ADD_ONION` -- far faster than the 180s budget, and far faster
than this project originally assumed possible. However, `Host unreachable`
**still occurred consistently** even with confirmed publication -- this
fix closes a real gap (a caller could otherwise dial before publication
with zero guarantee), but "at least one confirmation" turned out to be the
wrong threshold, not merely an early return. See `tetron/spec/core.py`'s
`TorDialPathWiring` for the fuller investigation.

## Patch 3: wait for a quorum of `HS_DESC UPLOADED` confirmations, not just one

**Files:** `src/lib.rs` -- new `HS_DESC_UPLOAD_QUORUM` constant and
`hs_desc_wait_should_stop` pure function; `TorCustomTransportBuilder::build`'s
publish-wait loop now checks confirmation count against the quorum instead
of `>= 1`.

**Found:** 2026-09-15, a real cross-machine test (`tetron/spec/core.py`'s
`TorDialPathWiring`, Fix 6) ruled out every environment-level explanation
tried so far (shared NAT, single external IP, outdated Tor client) for why
`Host unreachable` persisted even with Patch 2's single-confirmation wait
in place. Checking Tor's own daemon log directly (`journalctl -u
tor@default`, not this crate's or tetron's own log) during that test
showed the real mechanism: `Closed N streams for service [scrubbed].onion
for reason resolve failed. Fetch status: No more HSDir available to
query.` -- a descriptor *lookup* failure, not a circuit-extension one.

**Root cause:** Tor's v3 onion service spec (`rend-spec-v3`) uploads each
descriptor to a deterministic set of `hsdir_n_replicas` (2) x
`hsdir_spread_store` (4) = 8 distinct HSDirs, computed from the service's
blinded key and the current time period. A dialing client computes and
queries that exact same set independently, with **no fallback beyond it**.
Patch 2's wait returned as soon as *any one* of those 8 confirmed the
upload -- so a caller could (and, empirically, reliably did) start dialing
while only 1 of 8 responsible directories actually had the descriptor. Any
client whose own query hit one of the other 7 got a clean, fast "not
found" -- exactly matching the observed symptom's speed and consistency,
and its absence for DuckDuckGo's real service (long-lived enough for all 8
to have long since converged).

**Fix:** raise the threshold from 1 to `HS_DESC_UPLOAD_QUORUM = 8`. The
existing 180s timeout is unchanged as a fallback, so a network that never
reaches quorum still proceeds anyway after 180s exactly as Patch 2 already
did for zero confirmations -- this is strictly an improvement to the
common case, never a regression to the worst case. The stop/continue
decision itself was extracted into `hs_desc_wait_should_stop`, a pure
function of `(confirmations, quorum, deadline_passed)`, directly unit
tested (`src/tests/mod.rs`) without a live Tor connection.

**Status: implemented, unit-tested, live-verified real and correct -- but
not sufficient alone.** A cross-machine re-test confirmed the specific
`Host unreachable`/`No more HSDir available to query` symptom this patch
targets no longer occurs. A second, distinct failure remains after quorum
is reached (a 30s connect timeout, traced to Tor's own client log showing
every one of the target's introduction points marked unusable) -- not
something this patch can address, since it happens after descriptor lookup
succeeds. Root-caused separately, see Patch 4 below. See
`tetron/spec/core.py`'s `TorDialPathWiring`, Fix 6, for the full
investigation.

## Patch 4: de-duplicate concurrent connects to the same peer

**Files:** `src/lib.rs` -- `TorPacketSender`'s `streams` map now holds a
per-peer connect *slot* (`Mutex<HashMap<EndpointId, Arc<Mutex<Option<Arc<Mutex<TcpStream>>>>>>>`,
was `Mutex<HashMap<EndpointId, Arc<Mutex<TcpStream>>>>`); `get_or_connect`
locks that slot across the whole connect attempt instead of only inserting
into the map after success; `close`/`close_all` updated for the new shape.

**Found:** 2026-09-15, investigating why the introduction-point-usability
failure (Patch 3's own status note) persisted even after ruling out every
other candidate tried, including a real, independent bug found and worked
around along the way (a fixed-listen-port collision causing tetron's own
`bind_endpoint` retry to call this crate's `build()` -- and thus
`ADD_ONION` -- twice, tearing down the first ephemeral service's intro
circuits mid-flight; see `tetron/spec/core.py`'s `TorDialPathWiring`, Fix 7
-- ruled out as *this* symptom's cause once worked around and re-tested).

**Root cause:** `poll_send` (this crate's `CustomSender` hook) spawns a
brand-new, independent, fire-and-forget `tokio::spawn` task per outbound
packet/chunk. `get_or_connect` only recorded a peer's connection in its
cache *after* a connect succeeded, so every packet that needed to go out
while a previous connect for that same peer was still in flight
independently started its *own* fresh `Socks5Stream::connect` -- and QUIC's
own PATH_CHALLENGE retransmission (plus this project's own backup-path
probing) fires well inside the many seconds a real Tor hidden-service
rendezvous can take, so this wasn't a rare race but the common case under
any real traffic. Confirmed live via Tor's own log: 146 distinct "New SOCKS
connection opened" events in one ~5-minute test window (about one every 2
seconds), interleaved with repeated `Found unusable descriptor in cache
for [scrubbed]. Refetching..` -- Tor's own client invalidating and
re-fetching a descriptor that a competing, uncoordinated attempt had just
interrupted. A self-perpetuating stampede: no single attempt ever got to
run uninterrupted long enough to complete Tor's own real-world rendezvous
latency (independently confirmed elsewhere in this investigation at ~18s
for a real external onion service). This explains every symptom variant
seen across different test runs -- fast `intro_point_is_usable(): ...had
an error` bursts, an occasional 30s timeout, an occasional `unexpected end
of file` -- as different snapshots of the same underlying race, not
independent flakiness.

**Fix:** map each peer to a connect slot up front; the first caller to lock
an empty slot performs the real connect and fills it, every other
concurrent caller -- once it acquires the same slot's lock -- finds it
already filled and reuses it immediately, with zero additional connects.

**Status: implemented, unit-tested, live cross-machine re-verified.**
(`test_sender_dedupes_concurrent_connects_to_same_peer`: 5 concurrent
`send()` calls to the same peer against a connector gated to force real
overlap, asserting exactly one underlying connect regardless; the existing
sequential-reuse test continues to pass unchanged, confirming no regression
there.) Confirmed live: SOCKS connection count dropped from ~146/5min to
single digits across repeated re-tests -- the stampede is fixed -- but this
alone did not achieve end-to-end delivery; see Patch 5 below for the
mechanism found immediately after. See `tetron/spec/core.py`'s
`TorDialPathWiring`, Fix 8, for the full investigation.

## Patch 5: raise CONNECT_TIMEOUT from 30s to 90s

**Files:** `src/lib.rs` -- `CONNECT_TIMEOUT` constant only.

**Found:** 2026-09-15, immediately after Patch 4. With the connect
stampede fixed, live logs showed genuine Tor-protocol-level progress for
the first time in this investigation: real `INTRODUCE_ACK ack! Informing
rendezvous` successes (previously only NACKs, see `tetron/spec/core.py`'s
`TorDialPathWiring`, Fix 8's account of a second AI model's research pass
identifying stale client-side descriptor cache via `INTRODUCE_ACK` NACK
reason 1). But even a successful introduction didn't yet surface as an
active Tor path. Tracing one stream's full lifecycle in Tor's own log
(`connection_ap_handshake_attach_circuit`'s "stream N sec old"
progression) showed Tor cycling through four distinct rendezvous-circuit
build attempts in ~25 seconds, each completing a real intro-ack success
and `RENDEZVOUS_ESTABLISHED`, normal real-world v3 rendezvous behavior
under ordinary relay/circuit jitter -- not a hang, not a Tor defect. At
the 28-30s mark, `CONNECT_TIMEOUT` (Patch 1, 30s) fired, abandoning the
SOCKS5 socket -- and Tor's own log shows, in the same second, a mass burst
of every introduction point suddenly marked "had an error. Not usable".
This codebase's own timeout was cutting Tor off mid-cycle and then
(correctly, from Tor's point of view) having that abandonment interpreted
as every intro point having failed -- self-inflicted, not independent
Tor-network flakiness.

**Fix:** raise `CONNECT_TIMEOUT` to 90s, generous enough for several
real-world rendezvous-circuit-build cycles (~5-8s each, observed) while
still leaving room for at least two full attempts inside the existing 300s
`CUSTOM_TRANSPORT_PATH_MAX_IDLE_TIMEOUT` ceiling (`vendor/iroh-1.0.3/PATCH.md`)
before the QUIC path itself gives up and retries -- matching the same
"give Tor's own timing real headroom" philosophy already applied to
`HS_DESC_PUBLISH_TIMEOUT` (180s, Patch 2).

**Status: implemented; live cross-machine re-verification pending** -- the
hotspot machine needed for the separate-network test became unavailable
for approximately 2 hours partway through this investigation. See
`tetron/spec/core.py`'s `TorDialPathWiring`, Fix 9, for the full
investigation and the re-test result once run.
