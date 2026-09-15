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

**Status: fix applied, not yet re-verified live** (see this project's own
`DO-NOT-COMMIT/` for the pending re-run). If the connect genuinely was
hanging, this should now surface a `warn!` within 30s per attempt instead
of a silent 300s stall, and — if the underlying circuit build itself is
otherwise capable of succeeding — let a subsequent retry get further.
If a real onion-to-onion connect between two independent local Tor
daemons is *itself* the blocker (e.g., consistently exceeding 30s in this
test environment specifically), that will now show up as a repeating
`warn!` rather than silence, which is itself the next diagnostic step.
