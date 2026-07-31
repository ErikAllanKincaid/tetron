# Local patches

**Upstream:** https://github.com/n0-computer/noq (crate `noq-udp`, pulled in
transitively via `iroh` -> `netwatch`/`noq`). Neither patch reported upstream yet.

Two independent patches are carried here:

1. [musl `cmsghdr` alignment fix](#patch-1-musl-cmsghdr-alignment-fix)
2. [Android `EPERM` on `IP_PMTUDISC_PROBE`](#patch-2-android-eperm-on-ip_pmtudisc_probe)

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

---

# Status

Local vendor + `[patch.crates-io]` in the workspace root `Cargo.toml`.
Neither patch reported upstream yet. Remove this override (and this
directory) once either an upstream release includes equivalent fixes, or a
decision is made to maintain a real fork instead of an in-tree vendor copy.
