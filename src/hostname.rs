//! Hostname generation, validation, and collision handling for the roster.

use rand::RngExt;

use crate::network_name::NOUNS_B;

/// Default hostname when nothing else is available: the machine's own OS
/// hostname, sanitized into a valid tetron hostname. Falls back to a random
/// noun (the old default) only if the OS hostname is unavailable or
/// sanitizes down to nothing usable (e.g. empty, or made entirely of
/// characters tetron doesn't allow in a hostname).
///
/// Random names gave zero information about which machine a roster entry
/// actually was; the real hostname is immediately meaningful in `tetron
/// status`/`kick`/`admin add` at the cost of exposing it to every peer on
/// every network you join -- an explicit `--hostname` still overrides this
/// for anyone who'd rather not.
pub fn generate_hostname() -> String {
    if let Some(h) = machine_hostname() {
        return h;
    }
    let mut rng = rand::rng();
    NOUNS_B[rng.random_range(0..NOUNS_B.len())].to_string()
}

/// The machine's OS hostname (via `libc::gethostname`), sanitized into a
/// valid tetron hostname, or `None` if unavailable/unusable.
fn machine_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let raw = std::str::from_utf8(&buf[..end]).ok()?;
    sanitize_hostname(raw)
}

/// Turn an arbitrary string (an OS hostname, or a user-typed `--hostname`)
/// into a valid tetron hostname: keep only the first label (OS hostnames
/// are sometimes FQDN-ish, e.g. macOS's `MyLaptop.local`), lowercase ASCII
/// letters/digits, collapse anything else (spaces, underscores, other
/// punctuation) to a hyphen, trim leading/trailing hyphens, and truncate to
/// 63 characters. Returns `None` if nothing usable survives.
pub fn sanitize_hostname(raw: &str) -> Option<String> {
    let first_label = raw.split('.').next().unwrap_or(raw);
    let cleaned: String = first_label
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else {
                Some('-')
            }
        })
        .collect();
    let truncated: String = cleaned.trim_matches('-').chars().take(63).collect();
    let truncated = truncated.trim_end_matches('-');
    if truncated.is_empty() {
        None
    } else {
        Some(truncated.to_string())
    }
}

pub fn is_valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Decide the hostname to assign an admitted peer.
///
/// `authoritative` names come from an invite binding (`tetron invite --hostname`):
/// they are assigned verbatim, and a clash with a *different* identity is
/// rejected — no silent rename — so no peer can claim another's name (and the
/// Magic-DNS entry that resolves to it). A joiner-chosen (non-authoritative)
/// name keeps collision-resolution (`alice` → `alice-1` → …).
///
/// `taken` must already exclude the joining identity's own current name.
/// Returns `Ok(assigned)` or `Err(conflicting_name)` when an authoritative name
/// is already in use.
pub fn admission_hostname(
    desired: &str,
    taken: &[&str],
    authoritative: bool,
) -> Result<String, String> {
    if authoritative {
        if taken.contains(&desired) {
            return Err(desired.to_string());
        }
        return Ok(desired.to_string());
    }
    Ok(resolve_collision(desired, taken))
}

pub fn resolve_collision(desired: &str, taken: &[&str]) -> String {
    if !taken.contains(&desired) {
        return desired.to_string();
    }
    for i in 1u32.. {
        let candidate = format!("{desired}-{i}");
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_hostname_is_valid() {
        for _ in 0..100 {
            let h = generate_hostname();
            assert!(is_valid_hostname(&h), "invalid: {h}");
        }
    }

    #[test]
    fn sanitize_lowercases() {
        assert_eq!(sanitize_hostname("MyLaptop"), Some("mylaptop".to_string()));
    }

    #[test]
    fn sanitize_keeps_only_first_label() {
        // macOS-style FQDN hostnames (mDNS `.local` suffix).
        assert_eq!(
            sanitize_hostname("MyLaptop.local"),
            Some("mylaptop".to_string())
        );
    }

    #[test]
    fn sanitize_collapses_invalid_chars_to_hyphen() {
        assert_eq!(
            sanitize_hostname("my laptop_2"),
            Some("my-laptop-2".to_string())
        );
    }

    #[test]
    fn sanitize_trims_leading_trailing_hyphens() {
        assert_eq!(sanitize_hostname("-alice-"), Some("alice".to_string()));
        assert_eq!(sanitize_hostname("!!!"), None);
    }

    #[test]
    fn sanitize_rejects_empty_result() {
        assert_eq!(sanitize_hostname(""), None);
        assert_eq!(sanitize_hostname("...."), None);
    }

    #[test]
    fn sanitize_truncates_to_63_chars() {
        let long = "a".repeat(100);
        let sanitized = sanitize_hostname(&long).unwrap();
        assert_eq!(sanitized.len(), 63);
        assert!(is_valid_hostname(&sanitized));
    }

    #[test]
    fn sanitize_truncation_does_not_leave_trailing_hyphen() {
        // 62 'a's then a hyphen then more content -- truncating at 63 chars
        // would land exactly on the hyphen without the trim-after-truncate step.
        let raw = format!("{}-{}", "a".repeat(62), "b".repeat(10));
        let sanitized = sanitize_hostname(&raw).unwrap();
        assert!(is_valid_hostname(&sanitized));
        assert!(!sanitized.ends_with('-'));
    }

    #[test]
    fn valid_hostnames() {
        assert!(is_valid_hostname("alice"));
        assert!(is_valid_hostname("my-host"));
        assert!(is_valid_hostname("host2"));
        assert!(is_valid_hostname("a"));
    }

    #[test]
    fn invalid_hostnames() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("-start"));
        assert!(!is_valid_hostname("end-"));
        assert!(!is_valid_hostname("UPPER"));
        assert!(!is_valid_hostname("has space"));
        assert!(!is_valid_hostname("has.dot"));
        let long = "a".repeat(64);
        assert!(!is_valid_hostname(&long));
    }

    #[test]
    fn collision_no_conflict() {
        assert_eq!(resolve_collision("alice", &["bob"]), "alice");
    }

    #[test]
    fn collision_appends_number() {
        assert_eq!(resolve_collision("alice", &["alice"]), "alice-1");
        assert_eq!(resolve_collision("alice", &["alice", "alice-1"]), "alice-2");
    }

    #[test]
    fn admission_authoritative_rejects_collision() {
        // An invite-bound (authoritative) name already taken by someone else is
        // rejected — no silent rename — so a peer can't steal another's name.
        assert_eq!(
            admission_hostname("alice", &["alice"], true),
            Err("alice".to_string())
        );
    }

    #[test]
    fn admission_authoritative_free_name_assigned_as_is() {
        // An authoritative name nobody holds is assigned verbatim (no rename).
        assert_eq!(
            admission_hostname("alice", &["bob"], true),
            Ok("alice".to_string())
        );
    }

    #[test]
    fn admission_free_name_collision_is_renamed() {
        // A joiner-chosen (non-authoritative) name keeps collision-rename.
        assert_eq!(
            admission_hostname("alice", &["alice"], false),
            Ok("alice-1".to_string())
        );
        assert_eq!(
            admission_hostname("alice", &["bob"], false),
            Ok("alice".to_string())
        );
    }
}
