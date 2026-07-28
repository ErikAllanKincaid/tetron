# Local patch: musl `cmsghdr` alignment fix

**Upstream:** https://github.com/n0-computer/noq (crate `noq-udp`, pulled in
transitively via `iroh` -> `netwatch`/`noq`). Not reported upstream yet.

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

## Status

Local vendor + `[patch.crates-io]` in the workspace root `Cargo.toml`.
Not yet reported upstream. Remove this override (and this directory) once
either an upstream release includes an equivalent fix, or a decision is
made to maintain a real fork instead of an in-tree vendor copy.
