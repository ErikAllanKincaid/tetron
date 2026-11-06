//! Thread-safe local group and user lookups (THREADSAFE-LOOKUP-001).
//!
//! `getgrnam` and `getpwnam` return pointers into static libc storage; musl
//! also reads the database into a static line buffer that it grows with
//! `realloc`. Called from several daemon threads at once they corrupt the
//! heap (the 0.15.2 crash). These helpers use the reentrant `_r` variants
//! with a buffer owned by the caller, so no libc state is shared.

use std::ffi::CString;

const START_BUF: usize = 1024;
const MAX_BUF: usize = 1 << 20;

/// The gid of the local group `name`, or None if it does not exist.
pub fn group_gid(name: &str) -> Option<u32> {
    let cname = CString::new(name).ok()?;
    let mut buf: Vec<libc::c_char> = vec![0; START_BUF];
    loop {
        // SAFETY: `libc::group` is plain old data; all-zero is a valid value.
        let mut grp: libc::group = unsafe { std::mem::zeroed() };
        let mut out: *mut libc::group = std::ptr::null_mut();
        // SAFETY: every pointer refers to memory owned by this frame. libc
        // writes the entry into `grp`, its strings into `buf`, and sets
        // `out` to `&mut grp` on success or null when there is no such group.
        let rc = unsafe {
            libc::getgrnam_r(
                cname.as_ptr(),
                &mut grp,
                buf.as_mut_ptr(),
                buf.len(),
                &mut out,
            )
        };
        if rc == libc::ERANGE && buf.len() < MAX_BUF {
            let n = buf.len() * 2;
            buf.resize(n, 0);
            continue;
        }
        return (rc == 0 && !out.is_null()).then_some(grp.gr_gid);
    }
}

/// The uid of the local user `name`, or None if it does not exist.
pub fn user_uid(name: &str) -> Option<u32> {
    let cname = CString::new(name).ok()?;
    let mut buf: Vec<libc::c_char> = vec![0; START_BUF];
    loop {
        // SAFETY: `libc::passwd` is plain old data; all-zero is a valid value.
        let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: as in `group_gid`: all pointers are owned by this frame.
        let rc = unsafe {
            libc::getpwnam_r(
                cname.as_ptr(),
                &mut pw,
                buf.as_mut_ptr(),
                buf.len(),
                &mut out,
            )
        };
        if rc == libc::ERANGE && buf.len() < MAX_BUF {
            let n = buf.len() * 2;
            buf.resize(n, 0);
            continue;
        }
        return (rc == 0 && !out.is_null()).then_some(pw.pw_uid);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn root_group_and_user_are_zero() {
        assert_eq!(group_gid("root"), Some(0));
        assert_eq!(user_uid("root"), Some(0));
    }

    #[test]
    fn missing_group_and_user_are_none() {
        assert_eq!(group_gid("tetron-no-such-group-x9"), None);
        assert_eq!(user_uid("tetron-no-such-user-x9"), None);
        assert_eq!(group_gid("bad\0name"), None);
    }

    /// The daemon looks the group up from many threads at once. With the
    /// old `getgrnam` this corrupted the heap in the musl build.
    #[test]
    fn concurrent_lookups_agree() {
        let want_g = group_gid("root");
        let want_u = user_uid("root");
        let threads: Vec<_> = (0..16)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..500 {
                        assert_eq!(group_gid("root"), want_g);
                        assert_eq!(user_uid("root"), want_u);
                        assert_eq!(group_gid("tetron-no-such-group-x9"), None);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("lookup thread panicked");
        }
    }
}
