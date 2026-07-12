//! CLI firewall + declarative-apply handlers and their parsers/renderers.

use crate::*;

pub(crate) async fn ipc_firewall(action: FirewallAction) -> Result<()> {
    if let FirewallAction::Suggest {
        network,
        subject,
        allow,
        deny,
    } = action
    {
        return ipc_firewall_suggest(&network, &subject, allow, deny).await;
    }
    if let FirewallAction::Pending { network } = action {
        return ipc_firewall_pending(&network).await;
    }
    let mut stream = ipc::connect().await?;
    let req = match action {
        FirewallAction::Add {
            direction,
            action,
            proto,
            port,
            peer,
            network,
        } => ipc::IpcMessage::FirewallAdd {
            direction: direction.parse().map_err(anyhow::Error::msg)?,
            action: action.parse().map_err(anyhow::Error::msg)?,
            protocol: proto.parse().map_err(anyhow::Error::msg)?,
            port,
            peer,
            network,
        },
        FirewallAction::Remove { index } => ipc::IpcMessage::FirewallRemove { index },
        FirewallAction::Show => ipc::IpcMessage::FirewallShow,
        FirewallAction::Default { action } => ipc::IpcMessage::FirewallDefault {
            action: action.parse().map_err(anyhow::Error::msg)?,
        },
        FirewallAction::Reject { state } => {
            let enabled = match state.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                other => anyhow::bail!("expected `on` or `off`, got '{other}'"),
            };
            ipc::IpcMessage::FirewallReject { enabled }
        }
        FirewallAction::On => ipc::IpcMessage::FirewallSetEnabled { enabled: true },
        FirewallAction::Off => ipc::IpcMessage::FirewallSetEnabled { enabled: false },
        FirewallAction::Accept { network } => ipc::IpcMessage::FirewallAccept { network },
        FirewallAction::Deny { network } => ipc::IpcMessage::FirewallDeny { network },
        FirewallAction::AutoAccept { network, state } => {
            let enabled = match state.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                other => anyhow::bail!("expected `on` or `off`, got '{other}'"),
            };
            ipc::IpcMessage::FirewallAutoAccept { network, enabled }
        }
        // Handled above by early return (need extra round trips / interaction).
        FirewallAction::Suggest { .. } | FirewallAction::Pending { .. } => unreachable!(),
    };
    ipc::send(&mut stream, req).await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::FirewallState {
            default_inbound,
            default_outbound,
            reject,
            disabled,
            rules,
        } => {
            if json_enabled() {
                print_json(&serde_json::json!({
                    "default_inbound": default_inbound,
                    "default_outbound": default_outbound,
                    "reject": reject,
                    "disabled": disabled,
                    "rules": rules,
                }));
            } else {
                print!(
                    "{}",
                    render_firewall_rules(
                        Some((default_inbound, default_outbound)),
                        reject,
                        disabled,
                        &rules
                    )
                );
            }
        }
        ipc::IpcMessage::Error { message } => print_error("firewall", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

/// Print a JSON value as one compact line to stdout (jq-friendly).
pub(crate) fn print_json(value: &serde_json::Value) {
    println!("{value}");
}

/// Render a firewall rule table as aligned columns. `default` is the catch-all
/// action shown as a header (omitted for the pending-suggestions list).
pub(crate) fn render_firewall_rules(
    default: Option<(firewall::Action, firewall::Action)>,
    reject: bool,
    disabled: bool,
    rules: &[ipc::FirewallRuleView],
) -> String {
    let mut out = String::from("\n");
    if default.is_some() {
        // The torpedo firewall is separate from (and applies on top of) the host
        // OS / kernel firewall; both must allow a packet for it to pass.
        out.push_str(&format!(
            "  {}\n\n",
            style::faint("mesh firewall (separate from your host/kernel firewall)")
        ));
    }
    if disabled && default.is_some() {
        out.push_str(&format!(
            "  {}  {}\n\n",
            style::label("status     "),
            style::red("disabled (all packets allowed; torpedo firewall on to re-enable)")
        ));
    }
    if let Some((inbound, outbound)) = default {
        let styled = |a: firewall::Action| {
            let s = a.to_string();
            if a.is_deny() {
                style::red(&s)
            } else {
                style::green(&s)
            }
        };
        out.push_str(&format!(
            "  {}  {}\n",
            style::label("default in "),
            styled(inbound)
        ));
        out.push_str(&format!(
            "  {}  {}\n",
            style::label("default out"),
            styled(outbound)
        ));
        let reject_styled = if reject {
            style::green("on")
        } else {
            style::faint("off")
        };
        out.push_str(&format!(
            "  {}  {}\n\n",
            style::label("reject    "),
            reject_styled
        ));
    }
    if rules.is_empty() {
        out.push_str(&format!("  {}\n", style::faint("(no rules)")));
        return out;
    }
    let rows = rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let direction = r.direction.to_string();
            let protocol = r.protocol.to_string();
            let action_s = r.action.to_string();
            let action = if r.action.is_deny() {
                style::red(&action_s)
            } else {
                style::green(&action_s)
            };
            let sugg = r
                .suggested_by
                .as_ref()
                .map(|s| style::marker(&format!("suggested by {s}")))
                .unwrap_or_default();
            let sugg_plain = r
                .suggested_by
                .as_ref()
                .map(|s| format!("·suggested by {s}·"))
                .unwrap_or_default();
            vec![
                layout::Cell::new(i.to_string(), style::faint(&i.to_string())),
                layout::Cell::new(direction.clone(), style::value(&direction)),
                layout::Cell::new(action_s.clone(), action),
                layout::Cell::new(protocol.clone(), style::value(&protocol)),
                layout::Cell::right(r.port.clone(), style::value(&r.port)),
                layout::Cell::new(r.peer.clone(), style::value(&r.peer)),
                layout::Cell::new(r.network.clone(), style::faint(&r.network)),
                layout::Cell::new(sugg_plain, sugg),
            ]
        })
        .collect();
    out.push_str(&table(
        &["#", "dir", "action", "proto", "port", "peer", "network", ""],
        rows,
        4,
    ));
    out.push('\n');
    out
}

/// `torpedo firewall pending`: fetch the queued suggestions, then either run the
/// interactive picker (TTY) or print a static table (piped / `--json`).
pub(crate) async fn ipc_firewall_pending(network: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::FirewallPending {
            network: network.to_string(),
        },
    )
    .await?;
    let rules = match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::FirewallPendingResponse { rules, .. } => rules,
        ipc::IpcMessage::Error { message } => {
            print_error("firewall pending", &message, None);
            return Ok(());
        }
        other => {
            eprintln!("Unexpected response: {other:?}");
            return Ok(());
        }
    };

    if json_enabled() {
        print_json(&serde_json::json!({ "network": network, "rules": rules }));
        return Ok(());
    }
    if rules.is_empty() {
        println!("\n  {}\n", style::faint("no pending suggested rules"));
        return Ok(());
    }
    // Non-interactive (piped / NO_COLOR): print the static table and stop.
    if !style::is_enabled() {
        print!("{}", render_firewall_rules(None, false, false, &rules));
        return Ok(());
    }

    // Interactive picker → resolve the user's per-rule decisions.
    let Some(resolution) = picker::run(network, &rules)? else {
        // Ctrl-C: leave the queue untouched.
        return Ok(());
    };
    if resolution.accept.is_empty() && resolution.deny.is_empty() {
        println!("  {}", style::faint("no changes"));
        return Ok(());
    }
    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::FirewallResolveSuggestions {
            network: network.to_string(),
            accept: resolution.accept,
            deny: resolution.deny,
        },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => {
            println!("  {} {}", style::check(), style::value(&message));
        }
        ipc::IpcMessage::Error { message } => print_error("firewall pending", &message, None),
        other => eprintln!("Unexpected response: {other:?}"),
    }
    Ok(())
}

/// Parse a `--allow`/`--deny` value into `(peer, proto:ports-list)`.
///
/// The grammar is `PEER:proto:ports`, but the leading `PEER:` is optional: when
/// the value begins with a protocol keyword (`tcp`/`udp`/`icmp`/`any`) the peer
/// defaults to `*` (any peer). So `tcp:22` is read as "tcp/22 from any peer" —
/// the intuitive form — instead of "any port from a peer named `tcp`", which
/// would silently drop on the joiner (unresolvable hostname) and materialize no
/// rule at all, inverting the intent.
pub(crate) fn parse_suggest_token(spec: &str, flag: &str) -> Result<(String, String)> {
    let spec = spec.trim();
    anyhow::ensure!(
        !spec.is_empty(),
        "{flag} expects PEER:proto:ports (e.g. '*:tcp:22'), got an empty value"
    );
    // A leading protocol keyword means the peer was omitted: treat the whole
    // value as the proto:ports list against any peer.
    let first = spec.split(':').next().unwrap_or("");
    if first.parse::<firewall::Protocol>().is_ok() {
        return Ok(("*".to_string(), spec.to_string()));
    }
    let (peer, ports) = spec
        .split_once(':')
        .with_context(|| format!("{flag} expects PEER:proto:ports, got '{spec}'"))?;
    anyhow::ensure!(
        !peer.is_empty() && !ports.is_empty(),
        "{flag} expects PEER:proto:ports, got '{spec}'"
    );
    Ok((peer.to_string(), ports.to_string()))
}

/// `torpedo firewall suggest`: read the network's current suggestions, merge the
/// requested subject edits, and publish the updated set (coordinator-only).
pub(crate) async fn ipc_firewall_suggest(
    network: &str,
    subject: &str,
    allow: Vec<String>,
    deny: Vec<String>,
) -> Result<()> {
    use ray_proto::HostSuggestions;

    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::FirewallSuggestions {
            network: network.to_string(),
        },
    )
    .await?;
    let mut suggestions = match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::FirewallSuggestionsResponse { suggestions } => suggestions,
        ipc::IpcMessage::Error { message } => {
            print_error("error", &message, None);
            std::process::exit(1);
        }
        other => {
            eprintln!("Unexpected response: {other:?}");
            std::process::exit(1);
        }
    };

    let entry = suggestions.entry(subject.to_string()).or_default();
    for a in &allow {
        let (peer, ports) = parse_suggest_token(a, "--allow")?;
        entry.allows.insert(peer, ports);
    }
    for d in &deny {
        let (peer, ports) = parse_suggest_token(d, "--deny")?;
        entry.denies.insert(peer, ports);
    }
    // Drop a now-empty subject so removing all of a host's rules clears it.
    if entry == &HostSuggestions::default() {
        suggestions.remove(subject);
    }

    let mut stream = ipc::connect().await?;
    ipc::send(
        &mut stream,
        ipc::IpcMessage::FirewallSuggest {
            network: network.to_string(),
            suggestions,
        },
    )
    .await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{message}"),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {other:?}"),
    }
    Ok(())
}
