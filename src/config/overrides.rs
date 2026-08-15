//! Relay/discovery override resolution + `tetron config set`/`config get`
//! dispatch (MODULARIZE-003). Kept as one file, not split further: the
//! dispatch match arms in [`config_set`]/[`config_get`] call the resolver
//! functions directly, so separating "resolution" from "dispatch" would just
//! add a cross-file call for no gain. Re-exported from `crate::config` so
//! every existing `crate::config::…` path keeps compiling unchanged.

use anyhow::{Context, Result};
use std::net::Ipv4Addr;

use super::schema::{
    AppConfig, DropMonitorConfig, PathFlapConfig, RateLimitConfig, ReconnectColdConfig,
    ReconnectFrozenConfig, ReconnectLogConfig, ServerOverride,
};

/// Preset URL for the rayfish-operated iroh transport relay.
pub const RELAY_PRESET_RAYFISH: &str = "http://relay.iroh.rayfish.xyz:3340";
/// Preset URL for the rayfish-operated discovery-DNS / pkarr server.
pub const DISCOVERY_PRESET_RAYFISH: &str = "http://dns.iroh.rayfish.xyz:8080";

fn validate_http_url(s: &str) -> Result<()> {
    let u = url::Url::parse(s).with_context(|| format!("invalid URL: {s}"))?;
    anyhow::ensure!(
        matches!(u.scheme(), "http" | "https"),
        "URL must be http or https: {s}"
    );
    Ok(())
}

/// Resolve one relay/discovery entry: the `rayfish` keyword maps to `preset`,
/// anything else must be a valid http(s) URL (returned as-is).
fn resolve_url_entry(entry: &str, preset: &str) -> Result<String> {
    match entry {
        "rayfish" => Ok(preset.to_string()),
        other => {
            validate_http_url(other)?;
            Ok(other.to_string())
        }
    }
}

/// Resolve the relay override to concrete URL strings (presets expanded,
/// validated). Empty when unset.
pub fn relay_urls(o: &ServerOverride) -> Result<Vec<String>> {
    o.servers
        .iter()
        .map(|e| resolve_url_entry(e, RELAY_PRESET_RAYFISH))
        .collect()
}

/// Resolve the discovery-DNS override to concrete URL strings. Empty when unset.
pub fn discovery_urls(o: &ServerOverride) -> Result<Vec<String>> {
    o.servers
        .iter()
        .map(|e| resolve_url_entry(e, DISCOVERY_PRESET_RAYFISH))
        .collect()
}

/// Merge configured DNS upstreams with the system-captured ones. `replace`
/// drops the captured set; otherwise custom upstreams are tried first, then the
/// captured ones. Unset returns the captured set unchanged.
pub fn resolve_upstreams(o: &ServerOverride, captured: Vec<Ipv4Addr>) -> Vec<Ipv4Addr> {
    if o.servers.is_empty() {
        return captured;
    }
    let custom: Vec<Ipv4Addr> = o.servers.iter().filter_map(|s| s.parse().ok()).collect();
    if o.replace {
        custom
    } else {
        custom.into_iter().chain(captured).collect()
    }
}

/// Parse a comma list of entries (trimmed, empties dropped).
fn parse_entries(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Apply a `tetron config set`/`unset` to the in-memory config. An empty value or
/// the lone keyword `n0` resets the key to its default (iroh n0). Validates
/// every entry, so a bad URL/IP or unknown preset is rejected before persist.
pub fn config_set(cfg: &mut AppConfig, key: &str, value: &str, replace: bool) -> Result<()> {
    let entries = parse_entries(value);
    let reset = entries.is_empty() || entries == ["n0"];
    match key {
        "relay" => {
            if reset {
                cfg.relay = ServerOverride::default();
            } else {
                for e in &entries {
                    resolve_url_entry(e, RELAY_PRESET_RAYFISH)?;
                }
                cfg.relay = ServerOverride {
                    servers: entries,
                    replace,
                };
            }
        }
        "discovery-dns" => {
            if reset {
                cfg.discovery_dns = ServerOverride::default();
            } else {
                for e in &entries {
                    resolve_url_entry(e, DISCOVERY_PRESET_RAYFISH)?;
                }
                cfg.discovery_dns = ServerOverride {
                    servers: entries,
                    replace,
                };
            }
        }
        "subnet" => {
            // A single CIDR overlay subnet, not a URL list. Empty (or `n0`)
            // resets to the built-in default; `replace` is ignored here.
            if reset {
                cfg.subnet = None;
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "subnet takes a single CIDR, e.g. 10.88.0.0/16"
                );
                cfg.subnet = Some(crate::membership::parse_cidr(&entries[0])?);
            }
        }
        ratelimit_key if ratelimit_key.starts_with("ratelimit.") => {
            set_ratelimit_key(&mut cfg.ratelimit, ratelimit_key, &entries, reset)?;
        }
        drop_key if drop_key.starts_with("drop-monitor.") => {
            set_drop_monitor_key(&mut cfg.drop_monitor, drop_key, &entries, reset)?;
        }
        flap_key if flap_key.starts_with("path-flap.") => {
            set_path_flap_key(&mut cfg.path_flap, flap_key, &entries, reset)?;
        }
        reconnect_key if reconnect_key.starts_with("reconnect-log.") => {
            set_reconnect_log_key(&mut cfg.reconnect_log, reconnect_key, &entries, reset)?;
        }
        cold_key if cold_key.starts_with("reconnect-cold.") => {
            set_reconnect_cold_key(&mut cfg.reconnect_cold, cold_key, &entries, reset)?;
        }
        frozen_key if frozen_key.starts_with("reconnect-frozen.") => {
            set_reconnect_frozen_key(&mut cfg.reconnect_frozen, frozen_key, &entries, reset)?;
        }
        "nuke-proposal-ttl" => {
            cfg.nuke_proposal_ttl = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "nuke-proposal-ttl takes a single duration, e.g. 24h"
                );
                Some(parse_duration(&entries[0]).map_err(anyhow::Error::msg)?)
            };
        }
        "listen-port" => {
            cfg.listen_port = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "listen-port takes a single port number"
                );
                Some(parse_ratelimit_value(&entries[0])?)
            };
        }
        "poller-interval" => {
            cfg.poller_interval = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "poller-interval takes a single value in seconds"
                );
                Some(parse_ratelimit_value(&entries[0])?)
            };
        }
        "log-retention" => {
            cfg.log_retention = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "log-retention takes a single value in days"
                );
                Some(parse_ratelimit_value(&entries[0])?)
            };
        }
        "invite-default-expiry" => {
            cfg.invite_default_expiry = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "invite-default-expiry takes a single duration, e.g. 24h"
                );
                Some(parse_duration(&entries[0]).map_err(anyhow::Error::msg)?)
            };
        }
        "selfcapture-mitigation" => {
            cfg.selfcapture_mitigation = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "selfcapture-mitigation takes a single value: on/off"
                );
                Some(parse_bool_value(&entries[0])?)
            };
        }
        "log-level" => {
            cfg.log_level = if reset {
                None
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "log-level takes a single value: trace/debug/info/warn/error"
                );
                Some(parse_log_level_value(&entries[0])?)
            };
        }
        other => anyhow::bail!(
            "unknown config key: {other} (expected relay, discovery-dns, subnet, \
             nuke-proposal-ttl, listen-port, poller-interval, log-retention, \
             invite-default-expiry, selfcapture-mitigation, log-level, \
             drop-monitor.<window|threshold|cooldown>, \
             path-flap.<threshold|window>, \
             reconnect-log.<threshold|window>, \
             reconnect-cold.<threshold|backoff>, \
             reconnect-frozen.<threshold|backoff>, or \
             ratelimit.<capacity|refill-per-sec|strike-limit|global-capacity|\
             global-refill-per-sec|global-strike-limit>)"
        ),
    }
    Ok(())
}

/// Parse and apply one `drop-monitor.<key>` entry (LOG-002). `reset` (empty
/// value or "n0") clears the field back to `None` (compiled default = disabled).
fn set_drop_monitor_key(
    dm: &mut DropMonitorConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("drop-monitor.").expect("checked by caller");
    if reset {
        match sub {
            "window" => dm.window_secs = None,
            "threshold" => dm.threshold = None,
            "cooldown" => dm.cooldown_secs = None,
            other => anyhow::bail!("unknown drop-monitor config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "drop-monitor.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "window" => dm.window_secs = Some(parse_ratelimit_value(raw)?),
        "threshold" => dm.threshold = Some(parse_ratelimit_value(raw)?),
        "cooldown" => dm.cooldown_secs = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown drop-monitor config key: {other}"),
    }
    Ok(())
}

/// Parse and apply one `path-flap.<key>` entry (PATH-DIAG-006). `reset`
/// (empty value or "n0") clears the field back to `None` (compiled default).
fn set_path_flap_key(
    pf: &mut PathFlapConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("path-flap.").expect("checked by caller");
    if reset {
        match sub {
            "threshold" => pf.threshold = None,
            "window" => pf.window_secs = None,
            other => anyhow::bail!("unknown path-flap config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "path-flap.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "threshold" => pf.threshold = Some(parse_ratelimit_value(raw)?),
        "window" => pf.window_secs = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown path-flap config key: {other}"),
    }
    Ok(())
}

/// Parse and apply one `reconnect-log.<key>` entry (LOG-005). `reset` (empty
/// value or "n0") clears the field back to `None` (compiled default).
fn set_reconnect_log_key(
    rl: &mut ReconnectLogConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("reconnect-log.").expect("checked by caller");
    if reset {
        match sub {
            "threshold" => rl.threshold = None,
            "window" => rl.window_secs = None,
            other => anyhow::bail!("unknown reconnect-log config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "reconnect-log.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "threshold" => rl.threshold = Some(parse_ratelimit_value(raw)?),
        "window" => rl.window_secs = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown reconnect-log config key: {other}"),
    }
    Ok(())
}

/// Parse and apply one `reconnect-cold.<key>` entry (CONVERGE-011). `reset`
/// (empty value or "0") clears the field back to `None` (compiled default).
fn set_reconnect_cold_key(
    rc: &mut ReconnectColdConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("reconnect-cold.").expect("checked by caller");
    if reset {
        match sub {
            "threshold" => rc.threshold = None,
            "backoff" => rc.backoff_secs = None,
            other => anyhow::bail!("unknown reconnect-cold config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "reconnect-cold.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "threshold" => rc.threshold = Some(parse_ratelimit_value(raw)?),
        "backoff" => rc.backoff_secs = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown reconnect-cold config key: {other}"),
    }
    Ok(())
}

/// Parse and apply one `reconnect-frozen.<key>` entry (CONVERGE-013).
/// `reset` (empty value or "0") clears the field back to `None` (compiled
/// default). Same shape as [`set_reconnect_cold_key`], one tier up.
fn set_reconnect_frozen_key(
    rf: &mut ReconnectFrozenConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("reconnect-frozen.").expect("checked by caller");
    if reset {
        match sub {
            "threshold" => rf.threshold = None,
            "backoff" => rf.backoff_secs = None,
            other => anyhow::bail!("unknown reconnect-frozen config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "reconnect-frozen.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "threshold" => rf.threshold = Some(parse_ratelimit_value(raw)?),
        "backoff" => rf.backoff_secs = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown reconnect-frozen config key: {other}"),
    }
    Ok(())
}

/// Parse and apply one `ratelimit.<key>` entry (HARDEN-005). `reset` (empty
/// value or "n0") clears the field back to `None` (compiled default).
fn set_ratelimit_key(
    rl: &mut RateLimitConfig,
    key: &str,
    entries: &[String],
    reset: bool,
) -> Result<()> {
    let sub = key.strip_prefix("ratelimit.").expect("checked by caller");
    if reset {
        match sub {
            "capacity" => rl.capacity = None,
            "refill-per-sec" => rl.refill_per_sec = None,
            "strike-limit" => rl.strike_limit = None,
            "global-capacity" => rl.global_capacity = None,
            "global-refill-per-sec" => rl.global_refill_per_sec = None,
            "global-strike-limit" => rl.global_strike_limit = None,
            other => anyhow::bail!("unknown ratelimit config key: {other}"),
        }
        return Ok(());
    }
    anyhow::ensure!(
        entries.len() == 1,
        "ratelimit.{sub} takes a single numeric value"
    );
    let raw = &entries[0];
    match sub {
        "capacity" => rl.capacity = Some(parse_ratelimit_value(raw)?),
        "refill-per-sec" => rl.refill_per_sec = Some(parse_ratelimit_value(raw)?),
        "strike-limit" => rl.strike_limit = Some(parse_ratelimit_value(raw)?),
        "global-capacity" => rl.global_capacity = Some(parse_ratelimit_value(raw)?),
        "global-refill-per-sec" => rl.global_refill_per_sec = Some(parse_ratelimit_value(raw)?),
        "global-strike-limit" => rl.global_strike_limit = Some(parse_ratelimit_value(raw)?),
        other => anyhow::bail!("unknown ratelimit config key: {other}"),
    }
    Ok(())
}

fn parse_ratelimit_value<T: std::str::FromStr>(raw: &str) -> Result<T> {
    raw.parse::<T>()
        .map_err(|_| anyhow::anyhow!("invalid ratelimit value: {raw} (expected a whole number)"))
}

fn parse_bool_value(raw: &str) -> Result<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => anyhow::bail!("invalid value: {raw} (expected on/off)"),
    }
}

/// LOG-003: validates and canonicalizes a `log-level` value. Returns the
/// lowercase level name, matching a `tracing`/`EnvFilter` directive.
fn parse_log_level_value(raw: &str) -> Result<String> {
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => Ok(lower),
        _ => anyhow::bail!("invalid log-level: {raw} (expected trace/debug/info/warn/error)"),
    }
}

/// Parse a human-readable duration string into seconds.
///
/// Supports suffixes: `s` (seconds), `m` (minutes), `h` (hours), `d` (days),
/// `w` (weeks). A bare number is treated as seconds. Returns an error if the
/// string is malformed or the value overflows `u64`. Shared by
/// `invite-default-expiry`/`nuke-proposal-ttl` config parsing and
/// `tetron invite create --expires`.
pub(crate) fn parse_duration(s: &str) -> std::result::Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, suffix) = if s.ends_with(|c: char| c.is_ascii_alphabetic()) {
        let split = s.len() - 1;
        (&s[..split], &s[split..])
    } else {
        (s, "s") // bare number = seconds
    };
    let value: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number '{num_str}'"))?;
    let multiplier = match suffix {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err(format!("unknown suffix '{suffix}', use s/m/h/d/w")),
    };
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "duration overflows u64".to_string())
}

fn render_override(o: &ServerOverride) -> String {
    if o.is_unset() {
        "<default>".to_string()
    } else {
        let mode = if o.replace { "replace" } else { "augment" };
        format!("{} ({mode})", o.servers.join(","))
    }
}

/// Render config settings as `(key, value)` rows for `tetron config get`. With a
/// key, returns just that one (error on unknown key); without, all three.
pub fn config_get(cfg: &AppConfig, key: Option<&str>) -> Result<Vec<(String, String)>> {
    fn render_opt<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map(|n| n.to_string())
            .unwrap_or_else(|| "<default>".to_string())
    }
    let row = |k: &str| -> Result<(String, String)> {
        if k == "subnet" {
            let val = cfg
                .subnet
                .map(|(b, p)| format!("{b}/{p}"))
                .unwrap_or_else(|| "<default>".to_string());
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("ratelimit.") {
            let val = match sub {
                "capacity" => render_opt(cfg.ratelimit.capacity),
                "refill-per-sec" => render_opt(cfg.ratelimit.refill_per_sec),
                "strike-limit" => render_opt(cfg.ratelimit.strike_limit),
                "global-capacity" => render_opt(cfg.ratelimit.global_capacity),
                "global-refill-per-sec" => render_opt(cfg.ratelimit.global_refill_per_sec),
                "global-strike-limit" => render_opt(cfg.ratelimit.global_strike_limit),
                other => anyhow::bail!("unknown ratelimit config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("drop-monitor.") {
            let val = match sub {
                "window" => render_opt(cfg.drop_monitor.window_secs),
                "threshold" => render_opt(cfg.drop_monitor.threshold),
                "cooldown" => render_opt(cfg.drop_monitor.cooldown_secs),
                other => anyhow::bail!("unknown drop-monitor config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("path-flap.") {
            let val = match sub {
                "threshold" => render_opt(cfg.path_flap.threshold),
                "window" => render_opt(cfg.path_flap.window_secs),
                other => anyhow::bail!("unknown path-flap config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("reconnect-log.") {
            let val = match sub {
                "threshold" => render_opt(cfg.reconnect_log.threshold),
                "window" => render_opt(cfg.reconnect_log.window_secs),
                other => anyhow::bail!("unknown reconnect-log config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("reconnect-cold.") {
            let val = match sub {
                "threshold" => render_opt(cfg.reconnect_cold.threshold),
                "backoff" => render_opt(cfg.reconnect_cold.backoff_secs),
                other => anyhow::bail!("unknown reconnect-cold config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if let Some(sub) = k.strip_prefix("reconnect-frozen.") {
            let val = match sub {
                "threshold" => render_opt(cfg.reconnect_frozen.threshold),
                "backoff" => render_opt(cfg.reconnect_frozen.backoff_secs),
                other => anyhow::bail!("unknown reconnect-frozen config key: {other}"),
            };
            return Ok((k.to_string(), val));
        }
        if k == "nuke-proposal-ttl" {
            return Ok((k.to_string(), render_opt(cfg.nuke_proposal_ttl)));
        }
        if k == "listen-port" {
            return Ok((k.to_string(), render_opt(cfg.listen_port)));
        }
        if k == "poller-interval" {
            return Ok((k.to_string(), render_opt(cfg.poller_interval)));
        }
        if k == "log-retention" {
            return Ok((k.to_string(), render_opt(cfg.log_retention)));
        }
        if k == "invite-default-expiry" {
            return Ok((k.to_string(), render_opt(cfg.invite_default_expiry)));
        }
        if k == "selfcapture-mitigation" {
            let val = match cfg.selfcapture_mitigation {
                Some(true) => "on".to_string(),
                Some(false) => "off".to_string(),
                None => "<default: on>".to_string(),
            };
            return Ok((k.to_string(), val));
        }
        if k == "log-level" {
            let val = match &cfg.log_level {
                Some(level) => level.clone(),
                None => "<default: info>".to_string(),
            };
            return Ok((k.to_string(), val));
        }
        let o = match k {
            "relay" => &cfg.relay,
            "discovery-dns" => &cfg.discovery_dns,
            other => anyhow::bail!(
                "unknown config key: {other} (expected relay, discovery-dns, subnet, \
                 nuke-proposal-ttl, listen-port, poller-interval, log-retention, \
                 invite-default-expiry, selfcapture-mitigation, log-level, \
                 drop-monitor.<window|threshold|cooldown>, \
                 path-flap.<threshold|window>, \
                 reconnect-log.<threshold|window>, \
                 reconnect-cold.<threshold|backoff>, \
                 reconnect-frozen.<threshold|backoff>, or \
                 ratelimit.<capacity|refill-per-sec|strike-limit|global-capacity|\
                 global-refill-per-sec|global-strike-limit>)"
            ),
        };
        Ok((k.to_string(), render_override(o)))
    };
    match key {
        Some(k) => Ok(vec![row(k)?]),
        None => Ok(vec![
            row("relay")?,
            row("discovery-dns")?,
            row("subnet")?,
            row("ratelimit.capacity")?,
            row("ratelimit.refill-per-sec")?,
            row("ratelimit.strike-limit")?,
            row("ratelimit.global-capacity")?,
            row("ratelimit.global-refill-per-sec")?,
            row("ratelimit.global-strike-limit")?,
            row("drop-monitor.window")?,
            row("drop-monitor.threshold")?,
            row("drop-monitor.cooldown")?,
            row("path-flap.threshold")?,
            row("path-flap.window")?,
            row("reconnect-log.threshold")?,
            row("reconnect-log.window")?,
            row("reconnect-cold.threshold")?,
            row("reconnect-cold.backoff")?,
            row("reconnect-frozen.threshold")?,
            row("reconnect-frozen.backoff")?,
            row("nuke-proposal-ttl")?,
            row("listen-port")?,
            row("poller-interval")?,
            row("log-retention")?,
            row("invite-default-expiry")?,
            row("selfcapture-mitigation")?,
            row("log-level")?,
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_urls_expands_rayfish_preset() {
        let o = ServerOverride {
            servers: vec!["rayfish".into()],
            replace: false,
        };
        assert_eq!(
            relay_urls(&o).unwrap(),
            vec![RELAY_PRESET_RAYFISH.to_string()]
        );
        let d = ServerOverride {
            servers: vec!["rayfish".into()],
            replace: false,
        };
        assert_eq!(
            discovery_urls(&d).unwrap(),
            vec![DISCOVERY_PRESET_RAYFISH.to_string()]
        );
    }

    #[test]
    fn url_entry_rejects_bad() {
        assert!(
            relay_urls(&ServerOverride {
                servers: vec!["ftp://x".into()],
                replace: false
            })
            .is_err()
        );
        assert!(
            relay_urls(&ServerOverride {
                servers: vec!["not a url".into()],
                replace: false
            })
            .is_err()
        );
        // A real http URL passes through unchanged.
        let ok = ServerOverride {
            servers: vec!["http://r:1".into()],
            replace: false,
        };
        assert_eq!(relay_urls(&ok).unwrap(), vec!["http://r:1".to_string()]);
    }

    #[test]
    fn resolve_upstreams_augment_and_replace() {
        let captured = vec![Ipv4Addr::new(192, 168, 1, 1)];
        let one = Ipv4Addr::new(1, 1, 1, 1);

        // Unset: captured unchanged.
        assert_eq!(
            resolve_upstreams(&ServerOverride::default(), captured.clone()),
            captured
        );

        // Augment: custom first, then captured.
        let aug = ServerOverride {
            servers: vec!["1.1.1.1".into()],
            replace: false,
        };
        assert_eq!(
            resolve_upstreams(&aug, captured.clone()),
            vec![one, captured[0]]
        );

        // Replace: custom only.
        let rep = ServerOverride {
            servers: vec!["1.1.1.1".into()],
            replace: true,
        };
        assert_eq!(resolve_upstreams(&rep, captured.clone()), vec![one]);
    }

    #[test]
    fn config_set_get_path_flap() {
        let mut cfg = AppConfig::default();
        assert_eq!(config_get(&cfg, Some("path-flap.threshold")).unwrap()[0].1, "<default>");
        assert_eq!(config_get(&cfg, Some("path-flap.window")).unwrap()[0].1, "<default>");

        config_set(&mut cfg, "path-flap.threshold", "5", false).unwrap();
        assert_eq!(cfg.path_flap.threshold, Some(5));
        assert_eq!(config_get(&cfg, Some("path-flap.threshold")).unwrap()[0].1, "5");

        config_set(&mut cfg, "path-flap.window", "90", false).unwrap();
        assert_eq!(cfg.path_flap.window_secs, Some(90));
        assert_eq!(config_get(&cfg, Some("path-flap.window")).unwrap()[0].1, "90");

        // Empty resets to default (None).
        config_set(&mut cfg, "path-flap.threshold", "", false).unwrap();
        assert_eq!(cfg.path_flap.threshold, None);

        // Unknown sub-key and non-numeric value are both rejected.
        assert!(config_set(&mut cfg, "path-flap.bogus", "1", false).is_err());
        assert!(config_set(&mut cfg, "path-flap.threshold", "not-a-number", false).is_err());
    }

    #[test]
    fn config_set_get_reconnect_log() {
        let mut cfg = AppConfig::default();
        assert_eq!(config_get(&cfg, Some("reconnect-log.threshold")).unwrap()[0].1, "<default>");
        assert_eq!(config_get(&cfg, Some("reconnect-log.window")).unwrap()[0].1, "<default>");

        config_set(&mut cfg, "reconnect-log.threshold", "5", false).unwrap();
        assert_eq!(cfg.reconnect_log.threshold, Some(5));
        assert_eq!(config_get(&cfg, Some("reconnect-log.threshold")).unwrap()[0].1, "5");

        config_set(&mut cfg, "reconnect-log.window", "120", false).unwrap();
        assert_eq!(cfg.reconnect_log.window_secs, Some(120));
        assert_eq!(config_get(&cfg, Some("reconnect-log.window")).unwrap()[0].1, "120");

        // Empty resets to default (None).
        config_set(&mut cfg, "reconnect-log.threshold", "", false).unwrap();
        assert_eq!(cfg.reconnect_log.threshold, None);

        // Unknown sub-key and non-numeric value are both rejected.
        assert!(config_set(&mut cfg, "reconnect-log.bogus", "1", false).is_err());
        assert!(config_set(&mut cfg, "reconnect-log.threshold", "not-a-number", false).is_err());
    }

    // CONVERGE-011: reconnect-cold.<threshold|backoff> knobs.
    #[test]
    fn config_set_get_reconnect_cold() {
        let mut cfg = AppConfig::default();
        assert_eq!(config_get(&cfg, Some("reconnect-cold.threshold")).unwrap()[0].1, "<default>");
        assert_eq!(config_get(&cfg, Some("reconnect-cold.backoff")).unwrap()[0].1, "<default>");

        config_set(&mut cfg, "reconnect-cold.threshold", "20", false).unwrap();
        assert_eq!(cfg.reconnect_cold.threshold, Some(20));
        assert_eq!(config_get(&cfg, Some("reconnect-cold.threshold")).unwrap()[0].1, "20");

        config_set(&mut cfg, "reconnect-cold.backoff", "1800", false).unwrap();
        assert_eq!(cfg.reconnect_cold.backoff_secs, Some(1800));
        assert_eq!(config_get(&cfg, Some("reconnect-cold.backoff")).unwrap()[0].1, "1800");

        // Empty resets to default (None).
        config_set(&mut cfg, "reconnect-cold.threshold", "", false).unwrap();
        assert_eq!(cfg.reconnect_cold.threshold, None);

        // Unknown sub-key and non-numeric value are both rejected.
        assert!(config_set(&mut cfg, "reconnect-cold.bogus", "1", false).is_err());
        assert!(config_set(&mut cfg, "reconnect-cold.backoff", "not-a-number", false).is_err());
    }

    // CONVERGE-013: reconnect-frozen.<threshold|backoff> knobs, same shape
    // as reconnect-cold one tier up.
    #[test]
    fn config_set_get_reconnect_frozen() {
        let mut cfg = AppConfig::default();
        assert_eq!(config_get(&cfg, Some("reconnect-frozen.threshold")).unwrap()[0].1, "<default>");
        assert_eq!(config_get(&cfg, Some("reconnect-frozen.backoff")).unwrap()[0].1, "<default>");

        config_set(&mut cfg, "reconnect-frozen.threshold", "200", false).unwrap();
        assert_eq!(cfg.reconnect_frozen.threshold, Some(200));
        assert_eq!(config_get(&cfg, Some("reconnect-frozen.threshold")).unwrap()[0].1, "200");

        config_set(&mut cfg, "reconnect-frozen.backoff", "43200", false).unwrap();
        assert_eq!(cfg.reconnect_frozen.backoff_secs, Some(43200));
        assert_eq!(config_get(&cfg, Some("reconnect-frozen.backoff")).unwrap()[0].1, "43200");

        // Empty resets to default (None).
        config_set(&mut cfg, "reconnect-frozen.threshold", "", false).unwrap();
        assert_eq!(cfg.reconnect_frozen.threshold, None);

        // Unknown sub-key and non-numeric value are both rejected.
        assert!(config_set(&mut cfg, "reconnect-frozen.bogus", "1", false).is_err());
        assert!(config_set(&mut cfg, "reconnect-frozen.backoff", "not-a-number", false).is_err());
    }

    #[test]
    fn config_set_unknown_key_errors() {
        let mut cfg = AppConfig::default();
        assert!(config_set(&mut cfg, "bogus", "rayfish", false).is_err());
        assert!(config_get(&cfg, Some("bogus")).is_err());
    }

    #[test]
    fn config_set_n0_resets() {
        let mut cfg = AppConfig::default();
        config_set(&mut cfg, "relay", "rayfish", true).unwrap();
        assert!(!cfg.relay.is_unset());
        config_set(&mut cfg, "relay", "n0", false).unwrap();
        assert!(cfg.relay.is_unset());
    }

    #[test]
    fn config_set_get_subnet() {
        use std::net::Ipv4Addr;
        let mut cfg = AppConfig::default();
        // Unset renders as <default>.
        assert_eq!(config_get(&cfg, Some("subnet")).unwrap()[0].1, "<default>");
        // Set a CIDR (stored raw, even distinct from default).
        config_set(&mut cfg, "subnet", "10.99.0.0/16", false).unwrap();
        assert_eq!(cfg.subnet, Some((Ipv4Addr::new(10, 99, 0, 0), 16)));
        assert_eq!(config_get(&cfg, Some("subnet")).unwrap()[0].1, "10.99.0.0/16");
        // Empty resets to default (None).
        config_set(&mut cfg, "subnet", "", false).unwrap();
        assert_eq!(cfg.subnet, None);
        // Garbage / bad prefix is rejected.
        assert!(config_set(&mut cfg, "subnet", "not-a-cidr", false).is_err());
        assert!(config_set(&mut cfg, "subnet", "10.0.0.0/33", false).is_err());
    }

    #[test]
    fn config_set_get_configurability_audit_keys() {
        let mut cfg = AppConfig::default();
        // All five default to <default> when unset.
        for k in [
            "nuke-proposal-ttl",
            "listen-port",
            "poller-interval",
            "log-retention",
            "invite-default-expiry",
        ] {
            assert_eq!(config_get(&cfg, Some(k)).unwrap()[0].1, "<default>");
        }

        config_set(&mut cfg, "nuke-proposal-ttl", "12h", false).unwrap();
        assert_eq!(cfg.nuke_proposal_ttl, Some(12 * 3600));
        assert_eq!(config_get(&cfg, Some("nuke-proposal-ttl")).unwrap()[0].1, "43200");

        config_set(&mut cfg, "listen-port", "51820", false).unwrap();
        assert_eq!(cfg.listen_port, Some(51820));

        config_set(&mut cfg, "poller-interval", "30", false).unwrap();
        assert_eq!(cfg.poller_interval, Some(30));

        config_set(&mut cfg, "log-retention", "14", false).unwrap();
        assert_eq!(cfg.log_retention, Some(14));

        config_set(&mut cfg, "invite-default-expiry", "1d", false).unwrap();
        assert_eq!(cfg.invite_default_expiry, Some(86400));

        // Empty resets each back to None/<default>.
        config_set(&mut cfg, "nuke-proposal-ttl", "", false).unwrap();
        assert_eq!(cfg.nuke_proposal_ttl, None);
        config_set(&mut cfg, "listen-port", "", false).unwrap();
        assert_eq!(cfg.listen_port, None);

        // Garbage is rejected.
        assert!(config_set(&mut cfg, "listen-port", "not-a-port", false).is_err());
        assert!(config_set(&mut cfg, "nuke-proposal-ttl", "abc", false).is_err());
    }

    #[test]
    fn config_set_get_log_level() {
        let mut cfg = AppConfig::default();
        assert_eq!(
            config_get(&cfg, Some("log-level")).unwrap()[0].1,
            "<default: info>"
        );

        config_set(&mut cfg, "log-level", "DEBUG", false).unwrap();
        assert_eq!(cfg.log_level, Some("debug".to_string()));
        assert_eq!(config_get(&cfg, Some("log-level")).unwrap()[0].1, "debug");

        // Reset back to <default: info>.
        config_set(&mut cfg, "log-level", "", false).unwrap();
        assert_eq!(cfg.log_level, None);

        // Garbage is rejected.
        assert!(config_set(&mut cfg, "log-level", "verbose", false).is_err());
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("30").unwrap(), 30);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), 300);
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), 7200);
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("7d").unwrap(), 604800);
    }

    #[test]
    fn test_parse_duration_weeks() {
        assert_eq!(parse_duration("2w").unwrap(), 1209600);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_duration_overflow() {
        let big = format!("{}w", u64::MAX);
        assert!(parse_duration(&big).is_err());
    }
}
