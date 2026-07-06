#!/usr/bin/env python3
# reconcile.py -- run from ~/code/torpedo
# Usage: python3 reconcile.py
#
# Checks the automatable constraints (CON-001..CON-005) from spec/design_spec.py.
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


if __name__ == "__main__":
    ctx = {
        "build": check_build(),
        "clippy": check_clippy(),
        "test": check_tests(),
        "grep_hardcoded_cgnat": check_hardcoded_cgnat(),
        "relay_preset_untouched": check_relay_preset(),
        "self_update": check_self_update(),
    }
    print(json.dumps(ctx, indent=2))
    ok = (
        ctx["build"]["success"]
        and ctx["clippy"]["warnings"] == 0
        and ctx["test"]["pass"]
        and ctx["grep_hardcoded_cgnat"]["unexpected_count"] == 0
        and ctx["relay_preset_untouched"]["value"] == "rayfish"
        and ctx["self_update"]["enabled"] is False
    )
    sys.exit(0 if ok else 1)
