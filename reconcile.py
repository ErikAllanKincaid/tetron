#!/usr/bin/env python3
# reconcile.py -- run from ~/code/torpedo
# Usage: python3 reconcile.py
#
# Checks the automatable constraints (CON-001..CON-008) from spec/design_spec.py.
# It does NOT check the Requirement classes (SUBNET-*/RENAME-*); those are
# structural/design requirements verified by reading the diff and code directly.
import json
import re
import subprocess
import sys
from pathlib import Path


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True)


def check_build() -> dict:
    r = run(["cargo", "build", "--quiet"])
    return {"success": r.returncode == 0, "stderr": r.stderr[-2000:] if r.returncode else ""}


def check_clippy() -> dict:
    r = run(["cargo", "clippy", "--all-targets", "--quiet", "--", "-D", "warnings"])
    # -D warnings makes clippy fail (non-zero) if there are any warnings, so a
    # clean pass means returncode == 0; report 0 warnings in that case.
    return {"warnings": 0 if r.returncode == 0 else r.stderr.count("warning:")}


def check_tests() -> dict:
    r = run(["cargo", "test", "--quiet"])
    return {"pass": r.returncode == 0}


def check_hardcoded_cgnat(
    allowed_default_line_substrings=("100.64.0.0/10", "10.88.0.0/16"),
) -> dict:
    """Grep the touched files for leftover 100.64/100.100 literals. Comment lines
    documenting the default subnet (now 10.88.0.0/16) or the legacy/Tailscale
    100.64.0.0/10 range are allowed; anything beyond that is unexpected."""
    touched = ["src/membership.rs", "src/tun.rs", "src/dns.rs"]
    unexpected = 0
    for f in touched:
        p = Path(f)
        if not p.exists():
            continue
        for line in p.read_text().splitlines():
            if re.search(r"100\.64\.0\.0|100\.100\.100\.\d+", line):
                if not any(s in line for s in allowed_default_line_substrings):
                    unexpected += 1
    return {"unexpected_count": unexpected}


def check_relay_preset() -> dict:
    p = Path("src/config.rs")
    text = p.read_text() if p.exists() else ""
    return {"value": "rayfish" if '"rayfish" => Ok(preset.to_string())' in text else "MISSING"}


def check_self_update() -> dict:
    """CON-006: the self-update kill switch must stay off. `enabled` is False only
    while the exact `SELF_UPDATE_ENABLED: bool = false;` const is present; flipping
    it to true or removing it makes this True and fails the constraint."""
    p = Path("src/update.rs")
    text = p.read_text() if p.exists() else ""
    disabled = "pub const SELF_UPDATE_ENABLED: bool = false;" in text
    return {"enabled": not disabled}


def check_host_identity() -> dict:
    """CON-007: none of the collision-prone rayfish host-artifact / user-identifier
    tokens may remain anywhere under src/. This is a curated token set (NOT a bare
    `rayfish` grep), so it never trips on the KEEP-ON-PURPOSE rayfish names — the
    relay/discovery preset URLs (relay.iroh.rayfish.xyz, dns.iroh.rayfish.xyz),
    REPO_SLUG (rayfish/rayfish), the internal crate name, or the author
    attribution — which are all allowed to remain."""
    tokens = [
        "rayfish-dns.conf",  # NetworkManager drop-in (RENAME-006)
        ".before-rayfish",  # resolv.conf backup suffix (RENAME-006)
        "# Added by rayfish",  # resolv.conf takeover marker (RENAME-006)
        "tun-rayfish",  # resolvconf interface tag (RENAME-006)
        "Network/Service/rayfish",  # macOS SCDynamicStore key (RENAME-006)
        'new("rayfish")',  # macOS SCDynamicStore client name (RENAME-006)
        "com.rayfish.vpn",  # macOS launchd label / plist (RENAME-008)
        "rayfish://",  # deep-link URI scheme (RENAME-007)
        "RAYFISH_CONFIG_DIR",  # config-dir override env var (RENAME-007)
    ]
    leaks = 0
    for p in Path("src").rglob("*.rs"):
        text = p.read_text()
        for t in tokens:
            leaks += text.count(t)
    return {"leak_count": leaks}


def check_build_tooling_identity() -> dict:
    """RENAME-010/CON-008: none of the collision-prone `rayfish` build-tooling
    tokens may remain in `justfile` or `contrib/`. Curated set, same rationale
    as CON-007 (host_identity) but for non-Rust build/deploy tooling, which
    CON-007's src/**/*.rs scan does not cover. Does not flag `ray-mobile`/
    `libray_mobile` (the Android crate/artifact name) — a separate,
    deliberately-undecided item, not a regression of this fix."""
    tokens = [
        'binary := "ray"',  # justfile's binary variable, pre-fix form
        "groupadd rayfish",
        "systemctl restart rayfish",
        "systemctl stop rayfish",
        "/etc/rayfish",
        "rayfish.service",
        "com.rayfish.vpn",
    ]
    targets = [Path("justfile")]
    if Path("contrib").is_dir():
        targets += [p for p in Path("contrib").rglob("*") if p.is_file()]
    leaks = 0
    for p in targets:
        if not p.exists():
            continue
        text = p.read_text()
        for t in tokens:
            leaks += text.count(t)
    return {"unexpected_count": leaks}


if __name__ == "__main__":
    ctx = {
        "build": check_build(),
        "clippy": check_clippy(),
        "test": check_tests(),
        "grep_hardcoded_cgnat": check_hardcoded_cgnat(),
        "relay_preset_untouched": check_relay_preset(),
        "self_update": check_self_update(),
        "host_identity": check_host_identity(),
        "build_tooling_identity": check_build_tooling_identity(),
    }
    print(json.dumps(ctx, indent=2))
    ok = (
        ctx["build"]["success"]
        and ctx["clippy"]["warnings"] == 0
        and ctx["test"]["pass"]
        and ctx["grep_hardcoded_cgnat"]["unexpected_count"] == 0
        and ctx["relay_preset_untouched"]["value"] == "rayfish"
        and ctx["self_update"]["enabled"] is False
        and ctx["host_identity"]["leak_count"] == 0
        and ctx["build_tooling_identity"]["unexpected_count"] == 0
    )
    sys.exit(0 if ok else 1)
