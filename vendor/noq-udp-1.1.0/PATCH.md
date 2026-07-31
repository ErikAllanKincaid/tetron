# Local patches

**Upstream:** https://github.com/n0-computer/noq (crate `noq-udp`, pulled in
transitively via `iroh` -> `netwatch`/`noq`). Neither patch reported upstream yet.

Two independent patches are carried here:

1. [musl `cmsghdr` alignment fix](#patch-1-musl-cmsghdr-alignment-fix)
2. [Android `EPERM` on `IP_PMTUDISC_PROBE`](#patch-2-android-eperm-on-ip_pmtudisc_probe) (defensive only; it did not fix the crash it was written for, and its original rationale turned out to be wrong -- see the 2026-07-30 addendum in that section, which also records the real root cause and resolution)

## Rebase onto real upstream 1.1.1, 2026-07-30

This vendor copy's `Cargo.toml` declared `version = "1.1.0"` while genuine
upstream had since released `1.1.1`. Cargo's resolver prefers the newest
semver-compatible version it can find and only consults `[patch]` for
whichever version it actually wants -- since our declared version (1.1.0)
no longer matched, the patch was **silently never applied** the moment
anything resolved fresh against it (confirmed live: a separate consuming
crate, `tetron-mobile`, resolved plain unpatched `noq-udp 1.1.1` from
crates.io with no warning that either patch was missing). Note this is a
real, general Cargo footgun for any vendored patch, independent of
anything specific to this crate: **a vendored patch's declared version
must track real upstream's latest release, or it silently stops being
selected the next time anything re-resolves the dependency graph fresh.**

Before just relabeling the version, diffed this vendor copy against the
real `noq-udp 1.1.1` tarball from crates.io to check for any actual
content drift beyond our own two patches. Found one: real 1.1.1
re-disables `SO_TIMESTAMPNS` (`src/unix.rs`, `tests/tests.rs`), reverting
it behind `https://github.com/n0-computer/noq/issues/774` -- a genuine
upstream bug fix this vendor copy (based on a pre-1.1.1 state where it was
enabled) did not have. Relabeling the version without adopting this would
have shipped a copy claiming to be 1.1.1 while still carrying a known
upstream-tracked bug. Adopted the real 1.1.1 change (re-disabled
`SO_TIMESTAMPNS`, matching test updated to match) and bumped
`Cargo.toml`'s `version` to `1.1.1` to match. Re-diffed after: the only
remaining differences from real upstream `1.1.1` are this file's own two
documented patches, confirmed via `diff -rq` against the genuine crates.io
`noq-udp-1.1.1` source.

---

# Patch 1: musl `cmsghdr` alignment fix

## The bug

Running a musl-linked `tetron` daemon (`x86_64-unknown-linux-musl`) crashed
immediately on startup:

```
thread 'tokio-rt-worker' panicked at .../noq-udp-1.1.0/src/cmsg/mod.rs:81:5:
assertion failed: align_of::<T>() <= align_of::<C>()
```

`src/cmsg/mod.rs`'s `decode`/`push` asserted `align_of::<T>() <=
align_of::<C>()` (`C` = `libc::cmsghdr`) before using `ptr::read`/
`ptr::write`, which require proper alignment. This assumes `cmsghdr`'s
alignment is consistent across unix libcs -- it isn't: glibc's `cmsghdr`
uses `size_t cmsg_len` (8 bytes on x86_64, giving 8-byte alignment); musl's
uses `socklen_t cmsg_len` (4 bytes, giving 4-byte alignment). A `T` like
`libc::timespec` (8-byte aligned) legitimately fails that assertion under
musl even though the actual cmsg payload is fine.

## The fix

`decode`/`push` (`src/cmsg/mod.rs`) now use `ptr::read_unaligned`/
`ptr::write_unaligned` instead, and the alignment assertions are removed
-- the unaligned pointer operations don't need the guarantee the assertion
was checking for, so the check becomes unnecessary rather than something
to work around.

## Verified

- `cross build --release --target x86_64-unknown-linux-musl` compiles
  clean with this patch in place.
- Installed + ran the resulting binary on a real Rocky Linux 9 VM
  (`generic/rocky9`, vagrant-libvirt): `tetron install`, `tetron create`,
  `tetron status --json` all worked -- no crash, no coredump. Previously
  coredumped (`code=dumped, status=6/ABRT`) within seconds of daemon
  startup with the unpatched crate.

---

# Patch 2: Android `EPERM` on `IP_PMTUDISC_PROBE`

## The bug

The `tetron-mobile` Android embedder crashed at daemon startup on real
devices and emulators, before any network activity:

```
failed to bind iroh endpoint
Caused by:
    0: failed to bind iroh endpoint
    1: Failed to bind sockets
    2: Operation not permitted (os error 1)
```

`UdpSocketState::new` (`src/unix.rs`) sets `IP_MTU_DISCOVER` to
`IP_PMTUDISC_PROBE` (and the `IPV6_` equivalent) inside its
`#[cfg(any(target_os = "linux", target_os = "android"))]` block, to forbid
IPv4 fragmentation. Per `man 7 ip`, `IP_PMTUDISC_PROBE` requires
`CAP_NET_ADMIN` -- a sandboxed Android app process never holds that
capability, so the kernel rejects the `setsockopt` with `EPERM` (errno 1).
Notably this is `EPERM`, not `EACCES`, which is what ruled out a missing
Android manifest permission as the cause: `android.permission.INTERNET`
and `ACCESS_NETWORK_STATE` were already declared and confirmed granted.

The helper these calls go through, `set_socket_option_supported`, only
tolerated `ENOPROTOOPT` and `EOPNOTSUPP` as "option unavailable, degrade
gracefully" (returning `Ok(false)`). Every other errno, `EPERM` included,
propagated as a hard `Err` through the caller's trailing `?`, aborting
socket setup and therefore the whole iroh endpoint bind.

## The fix

`set_socket_option_supported` (`src/unix.rs`) gained a third tolerated
errno, `EPERM`, which now logs a warning and returns `Ok(false)` like the
other two rather than propagating.

Broadening the shared helper rather than special-casing the two Android
call sites is deliberate. The helper is single-purpose: all four of its
call sites use the identical `may_fragment |= !set_socket_option_supported(..)`
shape (`IP_MTU_DISCOVER`, `IPV6_MTU_DISCOVER`, `IP_DONTFRAG`,
`IPV6_DONTFRAG`), so `Ok(false)` uniformly means "could not disable
fragmentation, assume datagrams may fragment" -- there is no call site
where the option being unset carries some other consequence that an
`EPERM` could mask. The degradation is bounded and already designed for:
`may_fragment` reaches quinn only as `let allow_mtud = !socket.may_fragment()`
(`quinn/src/endpoint.rs`), i.e. it disables path-MTU discovery, leaving the
connection on quinn's conservative datagram-size floor. tetron already
handles a small `max_datagram_size` correctly by fragmenting oversized TUN
packets itself (FRAG-001/FRAG-002 in `src/forward.rs`).

The `EPERM` arm warns rather than staying silent, matching how the
Windows backend (`src/windows.rs`) already handles its own equivalent
`IP_DONTFRAGMENT`/`IPV6_DONTFRAG` fallbacks.

## Verified

- `cargo -q check` clean at the workspace root with the patch in place.
- Diagnosis established by reading the failing code path directly: the
  errno (1/`EPERM`), the `CAP_NET_ADMIN` requirement documented in
  `man 7 ip`, and the helper's errno allowlist all agree.

**Still to verify:** an actual `tetron-mobile` run on a device/emulator,
confirming the endpoint now binds. That lives in the separate proprietary
`tetron-mobile` repo, not here.

## Addendum 2026-07-30: this patch did not fix the crash, and its stated rationale is wrong

A rebuild and redeploy with the patch confirmed present in the shipped `.so` produced the identical error, and the `EPERM` arm's own warning never appeared in logcat. Re-checking the premise from scratch shows why: `IP_PMTUDISC_PROBE` does **not** require `CAP_NET_ADMIN`. `man 7 ip` names `CAP_NET_ADMIN` only for `IP_TRANSPARENT` and for high `IP_TOS` priority levels, never for `IP_MTU_DISCOVER`, and the kernel's `ip_setsockopt`/`do_ipv6_setsockopt` handlers for `IP_MTU_DISCOVER`/`IPV6_MTU_DISCOVER` do a range check on the value and nothing else. Setting it as an unprivileged user succeeds; that was confirmed empirically by running the same `setsockopt` call as a normal user on an ordinary Linux host. The paragraph above claiming otherwise was wrong, so this call was almost certainly never the source of the `EPERM`.

Nothing else in `UdpSocketState::new` is a better suspect either. For an IPv4 socket the only remaining hard-`?` `setsockopt` is `IP_PKTINFO`, which likewise has no capability check anywhere in the kernel. More decisively, Android's own network eBPF (`packages/modules/Connectivity/bpf/progs/netd.c`) attaches a `setsockopt/prog` cgroup program that **permits every socket-option write**, so the Android sandbox is not what would be denying a `setsockopt` in the first place. The same file does attach programs that return `EPERM` for two other steps of socket setup: `cgroupsock/inet_create`, which denies `socket()` outright for an app UID lacking the `INTERNET` permission in the kernel's UID permission map, and `bind4/inet4_bind` / `bind6/inet6_bind`, which deny `bind()` to any port present in a blocked-ports bitmap. Both of those calls live in `netwatch::udp::SocketState::bind`, one layer above this crate, and both propagate straight into iroh's `BindError::Sockets`.

The port-specific one is already ruled out by the reported error text: it contains `failed to bind iroh endpoint` twice, which in `tetron`'s `transport::create_endpoint_with_alpns` only happens on the ephemeral-port retry path, so the `0.0.0.0:0` bind failed too. Android's `block_port` returns allow immediately when the requested port is 0. That leaves `socket()` as the leading candidate and, in any case, moves the investigation out of this crate.

**`IP_PKTINFO` was deliberately not given the same treatment.** It is not a candidate for the `set_socket_option_supported` bucket even if it were the failing call. That helper's whole contract is "could not disable fragmentation, so assume datagrams may fragment", and its `Ok(false)` feeds only `may_fragment`. `IP_PKTINFO` is what makes the `pktinfo` cmsg arrive on receive, which is where `RecvMeta.dst_ip` comes from, which is in turn what lets the sender pin the source address of a reply to the local address the peer actually reached. Silently dropping it on a wildcard-bound socket is a correctness change on any multi-homed host, not a bounded degradation, and it would be invisible: a `Ok(false)` there has nowhere to be recorded. If a future diagnosis really does land on `IP_PKTINFO`, the answer is a deliberate, separately-designed fallback, not widening the fragmentation helper.

**Disposition of the `EPERM` arm:** kept. It is harmless and defensive (an option this crate only uses to suppress fragmentation should never be fatal, whatever errno the platform picks), but it is no longer believed to fix anything, and it must not be read as evidence that the Android `EPERM` was ever understood.

**Root cause found, resolved, outside this crate entirely.** A temporary
diagnostic (`transport::probe_socket_setup` in the `tetron` crate itself,
since removed -- it folded a report of the exact `socket()`/`bind()`/
`setsockopt()` sequence into the bind error itself rather than relying on
logging, since an embedder this early in startup may have no tracing
sink wired up) pinpointed the failure precisely: `socket()` itself, for
both IPv4 and IPv6, failed with `EPERM` -- before any `bind()` or
`setsockopt()` was even reached. This matches Android's `cgroupsock/
inet_create` eBPF hook (`packages/modules/Connectivity/bpf/progs/
netd.c`), which denies `socket()` outright for an app UID lacking the
`INTERNET` permission in the kernel's own UID permission map -- a
separate, later-synced thing from the app simply *declaring* the
permission in its manifest.

The actual cause: the test app's UID had been reinstalled (`adb install
-r`) many times across this same debugging session, starting from an
early build that declared no `INTERNET` permission at all. Android's
netd permission bitmap is keyed by UID, and a plain `-r` reinstall
keeps the same UID -- the kernel-level map was never resynced after
`INTERNET` was added in a later build. A full `adb uninstall` (assigning
a fresh UID) followed by a clean install resolved it immediately, with
zero code changes: the endpoint bound and the daemon started
successfully. This was never a bug in `tetron`, `iroh`, `netwatch`, or
this crate -- it was stale local-device state produced by the test
methodology itself, and would not recur on a real first-time install of
a properly-signed release build.

---

# Status

Local vendor + `[patch.crates-io]` in the workspace root `Cargo.toml`.
Neither patch reported upstream yet. Remove this override (and this
directory) once either an upstream release includes equivalent fixes, or a
decision is made to maintain a real fork instead of an in-tree vendor copy.
