//! Self-capture routing mitigation (SELFCAPTURE-ROUTE-001, closing the
//! original overlay self-capture bug, renamed `TUN-CAPTURE-001`).
//!
//! Every tetron node's TUN device installs an OS subnet route (e.g.
//! `10.88.0.0/24 -> tun0`) that iroh does not know is virtual, so it can
//! offer a peer's own overlay IP as a direct-dial candidate; any peer
//! sharing that same subnet route locally swallows the resulting packet
//! into its own kernel/tetron forwarder instead of ever reaching the real
//! remote host. The fix routes iroh's own outbound traffic -- identified by
//! its one fixed source port, daemon-wide since one shared `Endpoint` serves
//! every joined network -- around every overlay subnet route, regardless of
//! how many networks are joined.
//!
//! Applied once, daemon-wide, at daemon startup (`bootstrap::run_daemon`),
//! not at `tetron install` -- `ip rule`/`pf` state is runtime kernel state
//! that does not survive a reboot. Idempotent and fail-open: a missing tool
//! or failed command logs a warning and the daemon starts normally
//! regardless, since this is a best-effort mitigation, not a correctness
//! requirement. Torn down only at `tetron uninstall` (see [`teardown`]).

/// Apply the mitigation for the current listen port. Called once at daemon
/// startup, before any TUN device exists. `enabled` is the resolved
/// `tetron config` `selfcapture-mitigation` value (default `true`); when
/// `false`, any previously-applied rule/anchor from an earlier run is torn
/// down instead -- disabling the setting must fully undo it, not just stop
/// re-applying it, or a user who turns it off after running with it on would
/// be stuck with a stale rule they can no longer see reflected in config.
pub fn apply(listen_port: u16, enabled: bool) {
    if !enabled {
        tracing::debug!(
            "self-capture routing mitigation disabled by config; removing any existing state"
        );
        imp::teardown_inner();
        return;
    }
    if let Err(e) = imp::apply_inner(listen_port) {
        tracing::warn!(
            error = %e,
            "self-capture routing mitigation failed to apply; continuing without it"
        );
    } else {
        tracing::debug!(listen_port, "self-capture routing mitigation applied");
    }
}

/// Remove the mitigation entirely. Called only from `tetron uninstall`, not
/// on ordinary `tetron stop`/restart (mirrors TUN devices themselves, which
/// are likewise not torn down except on actual network leave/nuke).
pub fn teardown() {
    imp::teardown_inner();
}

#[cfg(target_os = "linux")]
mod imp {
    use std::process::Command;

    use anyhow::{Context, Result, bail};

    /// Fixed, arbitrary, tetron-owned routing-table id for the shadow
    /// default-route table. Distinct from the reserved local/main/default
    /// tables (0/253/254/255) and chosen high enough to be very unlikely to
    /// collide with another tool's own policy routing.
    const SHADOW_TABLE: u32 = 52369;
    /// FIB rule priority for the `ipproto udp sport` selector. Reused as the
    /// lookup key for idempotency: at most one rule ever points at
    /// `SHADOW_TABLE`, so finding "our" rule again after a restart or a
    /// `listen-port` change never depends on remembering the old port.
    const RULE_PRIORITY: u32 = 52369;

    pub(super) fn apply_inner(listen_port: u16) -> Result<()> {
        match current_rule_port()? {
            Some(p) if p == listen_port => {
                // Already correct. Still refresh the shadow default route
                // below in case the real gateway changed since last boot.
            }
            Some(old_port) => {
                remove_rule(old_port)?;
                add_rule(listen_port)?;
            }
            None => add_rule(listen_port)?,
        }
        refresh_shadow_default_route()
    }

    pub(super) fn teardown_inner() {
        if let Ok(Some(port)) = current_rule_port() {
            let _ = remove_rule(port);
        }
        let _ = Command::new("ip")
            .args(["route", "flush", "table", &SHADOW_TABLE.to_string()])
            .status();
    }

    /// Returns the source port of the existing rule pointing at
    /// `SHADOW_TABLE`, if any. There is never more than one -- this table id
    /// is tetron's own.
    fn current_rule_port() -> Result<Option<u16>> {
        let out = Command::new("ip")
            .args(["rule", "list"])
            .output()
            .context("run `ip rule list`")?;
        anyhow::ensure!(out.status.success(), "`ip rule list` failed");
        let text = String::from_utf8_lossy(&out.stdout);
        let needle = format!("lookup {SHADOW_TABLE}");
        for line in text.lines() {
            if !line.contains(&needle) {
                continue;
            }
            let port = line
                .split("sport ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<u16>().ok());
            if port.is_some() {
                return Ok(port);
            }
        }
        Ok(None)
    }

    fn add_rule(port: u16) -> Result<()> {
        run_ip(&[
            "rule",
            "add",
            "ipproto",
            "udp",
            "sport",
            &port.to_string(),
            "table",
            &SHADOW_TABLE.to_string(),
            "priority",
            &RULE_PRIORITY.to_string(),
        ])
    }

    fn remove_rule(port: u16) -> Result<()> {
        run_ip(&[
            "rule",
            "del",
            "ipproto",
            "udp",
            "sport",
            &port.to_string(),
            "table",
            &SHADOW_TABLE.to_string(),
            "priority",
            &RULE_PRIORITY.to_string(),
        ])
    }

    /// Mirrors the real default route into `SHADOW_TABLE` via `route
    /// replace` (an idempotent upsert, unlike `route add`), so iroh's own
    /// marked traffic still reaches the real internet-facing gateway instead
    /// of falling through to nothing once the overlay routes are bypassed.
    fn refresh_shadow_default_route() -> Result<()> {
        let out = Command::new("ip")
            .args(["route", "show", "default"])
            .output()
            .context("run `ip route show default`")?;
        anyhow::ensure!(out.status.success(), "`ip route show default` failed");
        let text = String::from_utf8_lossy(&out.stdout);
        let first_line = text
            .lines()
            .next()
            .context("no default route on this host")?;
        let mut toks = first_line.split_whitespace();
        let (mut gw, mut dev) = (None, None);
        while let Some(t) = toks.next() {
            match t {
                "via" => gw = toks.next(),
                "dev" => dev = toks.next(),
                _ => {}
            }
        }
        let (gw, dev) = (
            gw.context("default route has no gateway")?,
            dev.context("default route has no device")?,
        );
        run_ip(&[
            "route",
            "replace",
            "default",
            "via",
            gw,
            "dev",
            dev,
            "table",
            &SHADOW_TABLE.to_string(),
        ])
    }

    fn run_ip(args: &[&str]) -> Result<()> {
        let status = Command::new("ip")
            .args(args)
            .status()
            .with_context(|| format!("run `ip {}`", args.join(" ")))?;
        if !status.success() {
            bail!("`ip {}` exited with {status}", args.join(" "));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::{Context, Result, bail};

    /// Sub-anchor under the stock `nat-anchor "com.apple/*"` every macOS
    /// install already ships -- matching upstream's own already-proven
    /// `pfctl` integration in its exit-node feature. Loading a ruleset into
    /// a named anchor replaces its prior contents, so re-applying on every
    /// daemon start is naturally idempotent without a separate check-first
    /// step. Assumes `pf` itself is active, true by default on a stock,
    /// untinkered-with macOS (Apple's own built-in facilities depend on it)
    /// -- if a user has deliberately disabled `pf` globally, this load
    /// still succeeds but filters nothing until `pf` is re-enabled; tetron
    /// does not force-enable `pf` itself, since that is a much broader,
    /// system-wide action this mitigation has no business taking
    /// unilaterally.
    const ANCHOR_NAME: &str = "com.apple/tetron_selfcapture";

    pub(super) fn apply_inner(listen_port: u16) -> Result<()> {
        let (gw, dev) = default_route()?;
        let rule =
            format!("pass out proto udp from any port {listen_port} route-to ({dev} {gw})\n");
        let mut child = Command::new("pfctl")
            .args(["-a", ANCHOR_NAME, "-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn pfctl")?;
        child
            .stdin
            .take()
            .context("pfctl stdin unavailable")?
            .write_all(rule.as_bytes())
            .context("write pfctl ruleset")?;
        let output = child.wait_with_output().context("wait for pfctl")?;
        if !output.status.success() {
            bail!(
                "pfctl failed to load self-capture anchor: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    pub(super) fn teardown_inner() {
        let _ = Command::new("pfctl")
            .args(["-a", ANCHOR_NAME, "-F", "all"])
            .status();
    }

    /// Parses `route -n get default`'s `gateway:`/`interface:` lines.
    fn default_route() -> Result<(String, String)> {
        let out = Command::new("route")
            .args(["-n", "get", "default"])
            .output()
            .context("run `route -n get default`")?;
        anyhow::ensure!(out.status.success(), "`route -n get default` failed");
        let text = String::from_utf8_lossy(&out.stdout);
        let (mut gw, mut dev) = (None, None);
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("gateway: ") {
                gw = Some(v.to_string());
            }
            if let Some(v) = line.strip_prefix("interface: ") {
                dev = Some(v.to_string());
            }
        }
        Ok((
            gw.context("no default gateway found")?,
            dev.context("no default interface found")?,
        ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    use anyhow::Result;

    // No mitigation on other platforms (e.g. Android, where the packet
    // interface is a VpnService fd with no equivalent self-capture path).
    pub(super) fn apply_inner(_listen_port: u16) -> Result<()> {
        Ok(())
    }

    pub(super) fn teardown_inner() {}
}
