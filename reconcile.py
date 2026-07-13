#!/usr/bin/env python3
# reconcile.py -- run from ~/code/torpedo
# Usage: python3 reconcile.py
#
# Checks the automatable constraints (CON-001..CON-012, CON-M01, CON-M02, CON-M03)
# from spec/design_spec.py. It does NOT check the Requirement classes
# (SUBNET-*/RENAME-*/MINIMAL-*); those are structural/design requirements
# verified by reading the diff and code directly.
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
        'name = "rayfish"',  # Prometheus ForwardMetrics family (RENAME-015)
        'name = "rayfish_peer"',  # Prometheus PeerMetrics family (RENAME-015)
        'service_name("rayfish")',  # OTEL OTLP service name (RENAME-015)
        'tracer("rayfish")',  # OTEL tracer name (RENAME-015)
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


def check_report_identity() -> dict:
    """CON-009/RENAME-014: none of the collision-prone `rayfish` product-name
    tokens on the diagnostic/report + repo surface may remain. Spans a file set
    CON-007 (src only) and CON-008 (justfile/contrib) do not cover: `src/**/*.rs`
    plus the release/repo tooling `.github/**` and `cliff.toml`. Curated set, so
    it never trips on the KEEP-ON-PURPOSE `rayfish` names (the kept REPO_SLUG
    `rayfish/rayfish` has no `/compare` suffix; presets/crate/author differ) nor
    on RENAME-011's deferred `ray <verb>` comments (a different token)."""
    tokens = [
        "rayfish-report",  # torpedo report bundle filename (diagnostics.rs)
        "root:rayfish",  # firewall.toml perms comment (firewall.rs)
        "rayfish {version}",  # report sysinfo/issue banner (diagnostics.rs)
        "/var/log/rayfish",  # bug-report template log path
        "/Library/Logs/rayfish",  # bug-report template log path (macOS)
        "rayfish/rayfish/compare",  # cliff.toml changelog compare link
    ]
    targets = list(Path("src").rglob("*.rs"))
    if Path(".github").is_dir():
        targets += [p for p in Path(".github").rglob("*") if p.is_file()]
    if Path("cliff.toml").exists():
        targets.append(Path("cliff.toml"))
    leaks = 0
    for p in targets:
        text = p.read_text()
        for t in tokens:
            leaks += text.count(t)
    return {"unexpected_count": leaks}


def check_cli_reference_identity() -> dict:
    """CON-010/RENAME-016: no `ray <verb>` reference to the pre-fork binary name
    may remain in src (comments or strings). The lookbehind makes this clean
    where a bare `rayfish` grep could not be: it matches `ray ` + a lowercase
    word only when NOT preceded by `.`, a word char, or `-`, so it never trips
    on the KEEP forms — `.ray` (Magic-DNS TLD), `ray-proto`/`ray-mobile` (crate
    names), `stingray`/`array` (substrings), or `rayfish`. Every match is a
    stale CLI/binary reference that should read `torpedo`."""
    pat = re.compile(r"(?<![.\w-])ray (?=[a-z])")
    targets = list(Path("src").rglob("*.rs"))
    if Path("tests").is_dir():
        targets += [p for p in Path("tests").rglob("*") if p.is_file()]
    n = 0
    for p in targets:
        n += len(pat.findall(p.read_text()))
    return {"unexpected_count": n}


def check_test_harness_identity() -> dict:
    """CON-011/RENAME-017: the e2e/bench harness under `tests/` must not carry the
    pre-fork `rayfish` service/config/marker/record identity — those references
    are FUNCTIONAL (the scripts run against a `torpedo` binary/service), so a
    stale token silently breaks the test rather than being cosmetic. Curated set,
    so it never trips on the KEEP `NAMES=(rayfish-*)` Scaleway instance labels
    (bare `rayfish`, deliberately retained) or the `.ray` TLD."""
    tokens = [
        "systemctl stop rayfish",
        "systemctl start rayfish",
        "systemctl restart rayfish",
        "/etc/rayfish",
        "/root/.config/rayfish",
        "Added by rayfish",  # /etc/resolv.conf takeover marker (RENAME-006)
        "_rayfish_certgen",  # pkarr cert-generation record name
        "rayfish/files/1",  # FILES_ALPN
    ]
    n = 0
    if Path("tests").is_dir():
        for p in Path("tests").rglob("*"):
            if not p.is_file():
                continue
            text = p.read_text()
            for t in tokens:
                n += text.count(t)
    return {"unexpected_count": n}


def check_test_subnet_identity() -> dict:
    """CON-012/SUBNET-015: the tests/ harness must not assume the pre-fork
    100.64.0.0/10 CGNAT range or the fixed 100.100.100.53 magic-DNS IP. The fork
    defaults to 10.88.0.0/16 and derives the resolver at 10.88.100.53, so a stale
    literal is FUNCTIONAL breakage: `own_ip`'s `grep` extracts nothing from a real
    10.88.x.x address, and the DNS test queries the wrong magic IP. Counts the two
    upstream literals; must be 0. (`grep_hardcoded_cgnat`/CON-002 covers src.)"""
    tokens = ["100.64", "100.100.100"]
    n = 0
    if Path("tests").is_dir():
        for p in Path("tests").rglob("*"):
            if p.is_file():
                text = p.read_text()
                for t in tokens:
                    n += text.count(t)
    return {"unexpected_count": n}


def check_dependency_absence() -> dict:
    """CON-M01: Cargo.toml's [dependencies] section must not name any dep owned
    by a removed subsystem. iroh, iroh-blobs, and iroh-tor-transport are exempt
    (iroh-tor-transport is optional behind the `tor` feature, per MINIMAL-008)."""
    banned = [
        "reqwest",
        "rustls",
        "self-replace",
        "sha2",
        "semver",
        "russh",
        "pty-process",
        "uzers",
        "zbus",
        "inotify",
        "iroh-mdns-address-lookup",
        "indicatif",
        "crossterm",
        "unicode-width",
        "humansize",
        "mime_guess",
    ]
    p = Path("Cargo.toml")
    text = p.read_text()
    # Extract everything under [dependencies] — stop at the next section header.
    # (Takes the first [dependencies] block; works for Cargo.toml with one.)
    deps_section = text.split("[dependencies]")[1]
    deps_section = deps_section.split("\n[")[0]
    unexpected = []
    for name in banned:
        if re.search(rf"(?m)^{name}\s*=", deps_section):
            unexpected.append(name)
    # opentelemetry* (the base crate and all its sub-crates)
    for m in re.finditer(r"(?m)^opentelemetry[\w-]*\s*=", deps_section):
        name = m.group().split("=")[0].strip()
        if name not in unexpected:
            unexpected.append(name)
    return {"unexpected_count": len(unexpected), "unexpected": unexpected}


def check_crate_identity() -> dict:
    """CON-M03: After RENAME-M01, the bare `rayfish` token must not appear in
    .rs files outside the deliberately kept places: src/config.rs (relay preset,
    CON-001) and src/main.rs help text documenting the relay preset keyword."""
    leaks = 0
    for p in list(Path("src").rglob("*.rs")) + (list(Path("benches").rglob("*.rs")) if Path("benches").is_dir() else []):
        if str(p) == "src/config.rs":
            continue
        text = p.read_text()
        if "rayfish" not in text:
            continue
        if str(p) == "src/main.rs":
            for line in text.splitlines():
                if "rayfish" in line and "presets (rayfish/n0)" not in line:
                    leaks += line.count("rayfish")
        else:
            leaks += text.count("rayfish")
    return {"leak_count": leaks}


def check_wire_compat() -> dict:
    """CON-M02: transport::MESH_PROTOCOL_VERSION must equal 1 and the GroupBlob
    struct in src/membership.rs must retain its `suggested_firewall` and
    `reusable_keys` fields (ignored/preserved, never enforced or minted)."""
    transport = Path("src/transport.rs").read_text()
    membership = Path("src/membership.rs").read_text()
    mesh_version = None
    for line in transport.splitlines():
        # Matches `MESH_PROTOCOL_VERSION: u32 = 1` or `MESH_PROTOCOL_VERSION = 1`
        m = re.search(r"MESH_PROTOCOL_VERSION[^;]*?=\s*(\d+)", line)
        if m:
            mesh_version = int(m.group(1))
            break
    blob_fields = (
        "suggested_firewall" in membership and "reusable_keys" in membership
    )
    return {
        "mesh_version": mesh_version,
        "blob_fields_present": blob_fields,
    }


if __name__ == "__main__":
    ctx = {
        "build": check_build(),
        "clippy": check_clippy(),
        "test": check_tests(),
        "grep_hardcoded_cgnat": check_hardcoded_cgnat(),
        "relay_preset_untouched": check_relay_preset(),
        "host_identity": check_host_identity(),
        "build_tooling_identity": check_build_tooling_identity(),
        "report_identity": check_report_identity(),
        "cli_reference_identity": check_cli_reference_identity(),
        "test_harness_identity": check_test_harness_identity(),
        "test_subnet_identity": check_test_subnet_identity(),
        "dependency_absence": check_dependency_absence(),
        "wire_compat": check_wire_compat(),
        "crate_identity": check_crate_identity(),
    }
    print(json.dumps(ctx, indent=2))
    ok = (
        ctx["build"]["success"]
        and ctx["clippy"]["warnings"] == 0
        and ctx["test"]["pass"]
        and ctx["grep_hardcoded_cgnat"]["unexpected_count"] == 0
        and ctx["relay_preset_untouched"]["value"] == "rayfish"
        and ctx["host_identity"]["leak_count"] == 0
        and ctx["build_tooling_identity"]["unexpected_count"] == 0
        and ctx["report_identity"]["unexpected_count"] == 0
        and ctx["cli_reference_identity"]["unexpected_count"] == 0
        and ctx["test_harness_identity"]["unexpected_count"] == 0
        and ctx["test_subnet_identity"]["unexpected_count"] == 0
        and ctx["dependency_absence"]["unexpected_count"] == 0
        and ctx["wire_compat"]["mesh_version"] == 1
        and ctx["wire_compat"]["blob_fields_present"]
        and ctx["crate_identity"]["leak_count"] == 0
    )
    sys.exit(0 if ok else 1)
