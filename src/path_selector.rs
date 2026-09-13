//! PATHPREF-001: a custom `iroh::PathSelector` that lets a user force a
//! specific transport to carry real application data, once a candidate of
//! that type has proven itself with real received activity -- falling back
//! to iroh's own RTT-based selection (`iroh::BiasedRttPathSelector`,
//! re-exported for exactly this delegation, see `vendor/iroh-1.0.3/
//! PATCH.md`'s Patch 6) whenever no preference is set, or the preferred
//! transport has no validated candidate yet.
//!
//! Daemon-wide, not per-network: `MeshManager` holds exactly one shared
//! `iroh::Endpoint` for the whole process (`daemon/mod.rs`), and
//! `PathSelector` is registered once per endpoint -- see `PATHPREF-001`'s
//! own spec docstring for the full reasoning.

use std::sync::Arc;

use arc_swap::ArcSwapOption;
use iroh::{BiasedRttPathSelector, FourTuple, PathSelection, PathSelectionContext, PathSelector};

/// Live-reloadable slot for the current global path preference
/// (`tetron config set path-preference`). `None` means `auto` (the
/// default: current RTT-based behavior only). Registered once at
/// endpoint-build time (`transport::bind_endpoint`), read on every
/// `select()` call (cheap -- `ArcSwapOption::load` is lock-free), and
/// written by `daemon::MeshManager::set_path_preference` for live
/// reload over IPC, mirroring `log_reload`'s `LOG-004` pattern.
pub(crate) type PathPreferenceSlot = Arc<ArcSwapOption<String>>;

/// Whether `addr` is a candidate of the transport kind named by
/// `preference` (already validated against [`VALID_PREFERENCES`] by the
/// caller). Mirrors `daemon/mesh/select.rs::classify_candidate_addr`'s own
/// Veilid-vs-Tor classification (checked against the real transport id,
/// falling back to "Tor" for any other/future custom transport) so the
/// two never disagree about what a given address actually is.
fn matches_preference(addr: &FourTuple, preference: &str) -> bool {
    match preference {
        "direct" => matches!(addr, FourTuple::Ip { .. }),
        "relay" => matches!(addr, FourTuple::Relay { .. }),
        "veilid" => is_custom_transport(addr, true),
        "tor" => is_custom_transport(addr, false),
        _ => false,
    }
}

#[cfg_attr(not(feature = "veilid"), allow(unused_variables))]
fn is_custom_transport(addr: &FourTuple, want_veilid: bool) -> bool {
    let FourTuple::Custom { remote, .. } = addr else {
        return false;
    };
    #[cfg(feature = "veilid")]
    {
        (remote.id() == veilid_transport::VEILID_TRANSPORT_ID) == want_veilid
    }
    #[cfg(not(feature = "veilid"))]
    {
        // No Veilid support compiled in: every custom-transport address is
        // classified as Tor, matching `classify_candidate_addr`'s own
        // fallback for exactly the same reason.
        !want_veilid
    }
}

/// PATHPREF-001's own [`PathSelector`]. Wraps iroh's real default selector
/// for the `auto` case rather than reimplementing its tuning.
#[derive(Debug)]
pub(crate) struct TetronPathSelector {
    preference: PathPreferenceSlot,
    fallback: BiasedRttPathSelector,
}

impl TetronPathSelector {
    pub(crate) fn new(preference: PathPreferenceSlot) -> Self {
        Self {
            preference,
            fallback: BiasedRttPathSelector::default(),
        }
    }
}

impl PathSelector for TetronPathSelector {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        if let Some(preference) = self.preference.load().as_deref() {
            // Only override once a candidate of the preferred type has
            // real, confirmed *received* activity -- deliberately never
            // strands the connection on an unvalidated path. Same
            // received-bytes concept `PATHBLEED-STATUS-002`'s
            // `has_activity` already uses for status reporting
            // (`diagnostics.rs`): transmitted-only already counts a
            // path's own unvalidated `PATH_CHALLENGE` probe, so it
            // doesn't prove the path actually works.
            let validated = ctx.paths().find(|psd| {
                matches_preference(psd.network_path(), preference)
                    && psd.stats().is_some_and(|s| s.udp_rx.bytes > 0)
            });
            if let Some(psd) = validated {
                let mut selection = PathSelection::none();
                selection.set(&psd);
                return selection;
            }
        }
        self.fallback.select(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_addr(id: u64) -> iroh_base::CustomAddr {
        iroh_base::CustomAddr::from_parts(id, b"addr")
    }

    #[test]
    fn matches_preference_direct() {
        let ip = FourTuple::Ip {
            remote: "127.0.0.1:1234".parse().unwrap(),
            local: None,
        };
        assert!(matches_preference(&ip, "direct"));
        assert!(!matches_preference(&ip, "relay"));
        assert!(!matches_preference(&ip, "tor"));
        assert!(!matches_preference(&ip, "veilid"));
    }

    #[test]
    fn matches_preference_relay() {
        let relay = FourTuple::Relay {
            url: "https://relay.example.com".parse().unwrap(),
            endpoint_id: iroh::SecretKey::generate().public(),
        };
        assert!(matches_preference(&relay, "relay"));
        assert!(!matches_preference(&relay, "direct"));
    }

    #[cfg(feature = "veilid")]
    #[test]
    fn matches_preference_veilid_vs_tor() {
        let veilid_addr = FourTuple::Custom {
            remote: custom_addr(veilid_transport::VEILID_TRANSPORT_ID),
            local: None,
        };
        assert!(matches_preference(&veilid_addr, "veilid"));
        assert!(!matches_preference(&veilid_addr, "tor"));

        // A different custom transport id (e.g. Tor's own) falls back to
        // "tor", matching `classify_candidate_addr`'s own behavior.
        let other_addr = FourTuple::Custom {
            remote: custom_addr(u64::from_be_bytes(*b"\0\0\0tor\0\0")),
            local: None,
        };
        assert!(matches_preference(&other_addr, "tor"));
        assert!(!matches_preference(&other_addr, "veilid"));
    }

    #[cfg(not(feature = "veilid"))]
    #[test]
    fn matches_preference_custom_falls_back_to_tor_without_veilid_feature() {
        let addr = FourTuple::Custom {
            remote: custom_addr(42),
            local: None,
        };
        assert!(matches_preference(&addr, "tor"));
        assert!(!matches_preference(&addr, "veilid"));
    }

    #[test]
    fn matches_preference_unknown_value_matches_nothing() {
        let ip = FourTuple::Ip {
            remote: "127.0.0.1:1234".parse().unwrap(),
            local: None,
        };
        assert!(!matches_preference(&ip, "bogus"));
    }
}
