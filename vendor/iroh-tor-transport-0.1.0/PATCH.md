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

**Status: live-verified working, but insufficient on its own.** This
patch initially appeared to silently no-op: `set_events` was failing
every time with `ConnError::InvalidEventName`, a real bug in `torut`
itself (see `vendor/torut-0.2.1/PATCH.md`) -- unrelated to this crate,
but blocking this patch from ever actually subscribing. Once that was
fixed, both nodes in a live two-VM test reliably
confirmed `UPLOADED` within single-digit seconds of `ADD_ONION` -- far
faster than the 180s budget, and far faster than this project originally
assumed possible. However, `Host unreachable` **still occurred
consistently** even with confirmed publication -- this fix closes a real
gap (a caller could otherwise dial before publication with zero
guarantee), but descriptor publication was not, in the end, the actual
blocker in the environment this was tested in. See
`tetron/spec/core.py`'s `TorDialPathWiring` for the fuller investigation
and what was ultimately found.
