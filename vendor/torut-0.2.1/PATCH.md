# Local patches

**Upstream:** https://github.com/teawithsand/torut (crate `torut`). Not
reported upstream yet.

## Patch 1: allow `_` in `is_valid_event` (TOR-DIAL-001 follow-up)

**File:** `src/utils/mod.rs`, `is_valid_event`.

**Found:** 2026-09-15, live-verifying `iroh-tor-transport`'s new
`HS_DESC`-confirmation wait (`vendor/iroh-tor-transport-0.1.0/PATCH.md`,
Patch 2). `AuthenticatedConn::set_events(false, &mut ["HS_DESC"].into_iter())`
failed every time with `ConnError::InvalidEventName`, even though `HS_DESC`
is a real, documented Tor control-protocol event
(control-spec.txt §4.1.25).

**Root cause:** `is_valid_event` requires every character in the event
name to satisfy `char::is_ascii_uppercase()`. `HS_DESC` (and
`HS_DESC_CONTENT`) contain an underscore, which is not an uppercase ASCII
letter, so the check rejected them outright before the request ever
reached Tor. A pre-existing bug in torut itself, unrelated to Tor or to
this crate's own usage.

**Fix:** widen the check to also accept `_`. No other punctuation appears
in any real control-spec event name, so this isn't a switch to a full
allow-list, just closing the one gap that blocked a real, spec-compliant
name.

**Status: live-verified.** With this patch, `set_events(false, &mut
["HS_DESC"].into_iter())` succeeds (`250 OK`), and `iroh-tor-transport`'s
own `HS_DESC UPLOADED` handler (see its own PATCH.md) starts actually
receiving events instead of silently never being able to subscribe at
all -- confirmed live via `tetron-testsuite`'s `tor-smoke.sh`.
