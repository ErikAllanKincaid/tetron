# Local patches

**Upstream:** https://github.com/sticnarf/tokio-socks (crate `tokio-socks`).
Not reported upstream yet.

## Patch 1: surface SOCKS5 reply codes outside the standard RFC 1928 range instead of collapsing them

**Files:** `src/error.rs` (new `Error::ExtendedError(u8)` variant),
`src/tcp/socks5.rs` (the CONNECT-reply byte-to-error mapping extracted
into a new pure function, `connect_reply_error`, replacing the inline
`match` whose catch-all arm was `_ => Err(Error::UnknownAuthMethod)?`).

**Found:** 2026-09-15, deep in `tetron`'s own `TOR-DIAL-001` investigation
(`tetron/spec/core.py`). After several real, independently-confirmed fixes
to `iroh-tor-transport`/`torut` (HSDir descriptor quorum, a port-collision
double-build bug, a concurrent-connect stampede, a too-tight connect
timeout), a real two-machine cross-network test still failed to establish
Tor connectivity, with every failure surfacing through this crate as one
of a small set of generic named errors (`Error::HostUnreachable` most
commonly). Research into how other real-world Tor onion-service
applications (OnionShare, Ricochet-Refresh, Cwtch) diagnose exactly this
class of problem turned up Tor's own SOCKS5 protocol extension
(`ExtendedErrors` on a client's `SocksPort`,
<https://spec.torproject.org/socks-extensions.html>): when enabled, Tor
replies to a failed onion-service CONNECT with one of several specific
reply codes in the `0xF0`-`0xF7` range (descriptor not found, introduction
failed, rendezvous failed, introduction timed out, etc.) instead of a
single generic code -- genuinely different, actionable information this
investigation had no way to see.

**Root cause:** this crate's own CONNECT-reply parser only recognized the
standard RFC 1928 reply codes `0x00`-`0x08`; anything else -- including
every one of Tor's own extended codes -- fell through a catch-all arm that
returned `Error::UnknownAuthMethod`, a variant whose name refers to SOCKS5
*authentication* method negotiation, not a CONNECT reply at all (a
pre-existing minor naming bug in the catch-all's choice of variant,
unrelated to but compounding this one) -- and, critically, discarded the
actual reply byte entirely. Root-caused by reading Tor's own control-spec
directly and reasoning through what a client with `ExtendedErrors` enabled
would actually receive on the wire versus what this crate was capable of
representing.

**Fix:** extracted the byte-to-error mapping into a standalone pure
function (`connect_reply_error`), directly unit-testable without needing
to drive the full async handshake state machine (three tests: the success
byte maps to `None`, standard codes map to their existing named variants
unchanged, and Tor's own extended codes -- `0xF0`, `0xF2`, `0xF3`, `0xF7`
-- map to a new `Error::ExtendedError(u8)` variant that preserves the raw
byte). No caller-side code changes were needed elsewhere in this
investigation: `iroh-tor-transport` already logs the full `Display` output
of any connect failure (`tracing::warn!(..., %err, "Tor packet send
failed")`), and `Error::ExtendedError`'s own `#[error(...)]` message
(`"Extended (non-standard) SOCKS5 reply code: {0:#04x}"`) surfaces the raw
byte through that existing log line automatically once `ExtendedErrors` is
enabled on the Tor daemon's own `SocksPort` (a torrc/deployment change,
not a code change this crate or `iroh-tor-transport` can make on Tor's
behalf).

**Status: implemented, unit-tested; live cross-machine re-verification
pending.** See `tetron/spec/core.py`'s `TorDialPathWiring` for the full
investigation and the re-test result once run.
