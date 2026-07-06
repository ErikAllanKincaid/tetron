# Spec proposal: rayfish fork with configurable IPv4 overlay subnet ("torpedo")

**Status: proposal, pending review. Not started. Nothing has been created or built yet.**

This document is written to be handed, verbatim, as the opening context to a fresh Claude Code
instance running on **590I-AORUS-ULTRA** ("AURUS"), which will do the actual implementation work.
It assumes that instance has no memory of any prior conversation, everything it needs is below.

---

## 0. Background (for the implementing instance)

`rayfish` is a P2P mesh VPN (`~/code/rayfish` on the machine this spec was written on; the
implementing instance should clone its own copy, see Phase 0). It hardcodes its overlay IPv4
range to `100.64.0.0/10` and actively refuses to start if any other interface (e.g. Tailscale)
already holds an address in that range. That makes rayfish and Tailscale mutually exclusive on
the same host. The machines this fork targets (this laptop, and AURUS itself) are already active
members of a real, in-daily-use Tailscale tailnet, so this is a real, hit-in-practice blocker,
not hypothetical.

An upstream feature request proposing a fully general, backward-compatible fix already exists
(not filed yet): see the "full production version" contrast in section 4 below. **This spec is
for a smaller, scrappier private fork**, to be used for personal testing on machines the user
controls, accepting that it may need rework later and will not track upstream automatically.
That tradeoff has already been discussed and accepted, do not re-litigate it.

---

## 1. Naming (provisional, confirm before writing code)

Working name for this fork: **`torpedo`** (the genus name for electric rays, a real ray species,
keeping the fish lineage without reusing "ray" itself). Checked against prior use before settling
on it:

- `manta` was checked and dropped: it collides with an existing Rust crate (`manta-cli` on
  crates.io, the exact registry that would matter if this fork is ever published), plus separate
  npm, PyPI, and Illumina bioinformatics packages of the same name.
- `sideband` was checked: clean on package managers, but is already the name of a client for
  **Reticulum**, a real off-grid mesh-networking project, a worse kind of collision than an
  unrelated app sharing a name, since it's in the exact same problem domain.
- `chimaera` was checked: a loose, non-exact collision (`chimera-cli` on crates.io, ChimeraOS the
  distro), not disqualifying but not as clean as torpedo.
- `torpedo` came back clean across general search, crates.io, and no domain-adjacent collision.

**Before writing any code, confirm this name with the user, or use whatever they specify
instead.** Every path/string below assumes `torpedo`, substitute freely if a different name is
chosen, and re-check it for prior use the same way before locking it in.

---

## 2. Phase 0 — One-time setup on AURUS

Run these once. Do not skip the verification steps, this environment has not been checked
directly on AURUS yet for some of these.

```bash
# 1. Create the fork's directory and clone rayfish into it
mkdir -p ~/code/torpedo
cd ~/code/torpedo
git clone https://github.com/rayfish/rayfish.git .
git log -1 --format="%H %ci"   # confirm; the spec below was written against
                                # commit 9e142411008f70228fccf9eb109af6a5d058c4e6 (2026-07-05).
                                # If AURUS clones a newer commit, the file/line references in
                                # section 4 may have drifted, re-check them before editing.

# 2. Re-point the remote so this is clearly a fork, not a checkout of upstream
git remote rename origin upstream
git remote add origin <leave unset until there's somewhere to push to, or set to a private repo>

# 3. Confirm the Rust toolchain (already present on AURUS as of this writing: cargo/rustc 1.92.0)
cargo --version
rustc --version   # need 1.85+ (2024 edition); 1.92 already confirms this

# 4. Confirm libspec is usable (already cloned on AURUS as of this writing, at ~/code/libspec,
#    with a working .venv, no `uv` needed, `uv` is NOT installed on AURUS, use the venv binary
#    directly)
~/code/libspec/.venv/bin/libspec --version
~/code/libspec/.venv/bin/libspec --help   # confirm the command set below still matches; this
                                            # copy is from 2026-06-07 and may have drifted
```

```bash
# 5. Scaffold the spec directory inside the fork and initialize libspec there
cd ~/code/torpedo
~/code/libspec/.venv/bin/libspec init
```

This should create a `spec/` directory and install a git post-commit hook. Confirm both exist
before moving on (`ls spec/`, `cat .git/hooks/post-commit`).

**Do not run `libspec build` yet**, that comes after section 5 below, once `spec/design_spec.py`
exists with real content, not before.

---

## 3. Scope of the fork (genuinely configurable, not a hardcoded swap)

The user explicitly wants this to be **configurable** (a `--subnet` flag at network-creation
time), not a second hardcoded constant swapped in for the first one. Every location below was
found by reading the source directly, not guessed:

| Location | What's hardcoded today |
| --- | --- |
| `src/membership.rs`, `derive_ip_with_index()` | base `0x6440_0000` (100.64.0.0), and a 22-bit host mask sized specifically for a /10 |
| `src/membership.rs`, `ensure_in_cgnat_range()` | validation that rejects any assigned IP outside `100.64.0.0/10` |
| `src/tun.rs`, `is_cgnat()` / `check_cgnat_conflict()` | the foreign-VPN conflict detector, this is the check that currently blocks `ray up` (soon `torpedo up`) from starting next to Tailscale |
| `src/tun.rs`, `create()` | hardcoded netmask `(255, 192, 0, 0)` (/10) and hardcoded gateway `100.64.0.1` |
| `src/tun.rs`, `route_peer_range()` (macOS branch) | hardcoded literal string `"100.64.0.0/10"` passed to the BSD `route` command |
| `src/dns.rs`, `MAGIC_DNS_V4` | the Magic DNS resolver's own reserved address, `100.100.100.53`, inside that same range |
| `src/dns.rs`, PTR/reverse-lookup handler (~line 246 as of the commit above) | a second, independent hardcoded `100.64.0.0/10` check for NXDOMAIN logic |

### 3.1 Design, scoped down for a personal-test fork

This is deliberately **not** the full upstream-quality design. The following simplifications are
accepted on purpose, do not add back the complexity they remove unless something below turns out
to be wrong in practice:

- **Where the subnet lives:** add `pub subnet: Option<(Ipv4Addr, u8)>` (base address + prefix
  length) to `GroupBlob` in `src/membership.rs`. This is the correct place, it's the
  signed, network-wide record every peer fetches and agrees on (confirmed by reading the struct:
  it already has `pub name: Option<String>` with `#[serde(default, skip_serializing_if =
  "Option::is_none")]`, follow that exact pattern for the new field). Do **not** put this on
  `NetworkConfig` in `src/config.rs`, that struct is confirmed to be per-node local cache state
  (this node's own assigned IP, hostname), not the mesh-wide source of truth.
- **CLI:** add `--subnet <CIDR>` to `ray create`'s (soon `torpedo create`'s) argument struct in
  `src/main.rs`, parsed into `(Ipv4Addr, u8)`, following the existing style of `--name`/`--hostname`.
  When omitted, fall back to `100.64.0.0/10` as today (no reason to break the no-flag case).
- **`derive_ip_with_index`**: take the subnet as a parameter instead of using the hardcoded
  constant. Host-bit mask width becomes `32 - prefix_len` bits, computed at call time, not a
  fixed 22. This is plain bit-shift arithmetic, not an algorithmic challenge, but it is the one
  place genuine care is needed: the mask computation, the netmask below, and the gateway
  address must all agree on the same prefix length or peers will silently derive inconsistent
  addresses.
- **`ensure_in_cgnat_range`**: rename in spirit (can keep the function name if that's less
  churn) to validate against the network's own configured subnet, read from the `GroupBlob`,
  rather than the single global constant.
- **`tun::create()`**: compute netmask from prefix length (small helper: prefix length → dotted
  netmask, standard CIDR arithmetic) and gateway as `base + 1`, both as parameters instead of
  hardcoded values.
- **`check_cgnat_conflict()`**: **delete the call entirely.** For a personal fork where you are
  deliberately choosing a subnet outside `100.64.0.0/10`, there is nothing to detect. (The
  upstream feature request keeps this check pointed at `100.64.0.0/10` specifically, since its
  job there is detecting *other* tools like Tailscale, that nuance does not apply here, it is
  fine to just remove it for this fork.)
- **Magic DNS resolver address**: compute an offset address relative to whichever subnet is
  configured (e.g. `base + 0x00_64_35` truncated appropriately) rather than the fixed
  `100.100.100.53` literal. Assume the configured subnet is `/24` or larger, no need to handle
  degenerate tiny subnets for this use case.
- **PTR/reverse-lookup handler**: mirror whatever range check is used in
  `ensure_in_cgnat_range` so both stay consistent.
- **Skip the macOS branch in `route_peer_range()` entirely.** All machines in this test (this
  laptop, AURUS, and whichever others) are Linux. Do not spend time generalizing code that will
  never execute.
- **Skip ALPN/wire-version gating for mixed old/new peers.** Every machine in this test will run
  the same patched build. If an unpatched binary somehow tries to join a network with a custom
  subnet, it is acceptable for that to fail in a possibly-confusing way, this was explicitly
  accepted by the user ("even if it breaks later, I would like to use it for now as a test").
- **Skip backward-compatibility handling for pre-existing default-range networks.** This is a
  brand-new test network, not a migration of an existing one.

---

## 4. Rename scope

**Read this section carefully before doing a find-and-replace.** A literal search for the string
`rayfish` across the codebase turns up far more than "the binary name", roughly 20+ locations
across `src/` and `ray-proto/src/`, and **not all of them refer to this project's own identity**.
Some refer to upstream's own external infrastructure and renaming them would silently break a
feature rather than rebrand it.

### 4.1 Rename these (this project's own identity, essential for a clean personal fork)

| What | Current value | New value | Where |
| --- | --- | --- | --- |
| Binary name | `ray` | `torpedo` | build output name, `Cargo.toml` `[[bin]]`, `contrib/rayfish.service`'s `ExecStart` path |
| Systemd service | `rayfish.service`, `systemctl ... rayfish` | `torpedo.service`, `systemctl ... torpedo` | `src/cli/service.rs` (multiple), `src/cli/update.rs`, `src/update.rs`, `contrib/rayfish.service` (rename the file too) |
| Unix group | `"rayfish"` | `"torpedo"` | `src/cli/service.rs:16` |
| Config/state directory | `/etc/rayfish` | `/etc/torpedo` | `src/config.rs` (multiple, includes a migration routine at line ~703-755 that relocates an old config tree into this path, check whether that migration logic should be kept, disabled, or repointed) |
| Log directory | `/var/log/rayfish` (Linux), `/Library/Logs/rayfish` (macOS, can skip) | `/var/log/torpedo` | `src/logdir.rs` |
| Daemon socket path | `/var/run/rayfish/rayfish.sock` (Linux; note the macOS branch uses a different path, `/var/run/rayfish.sock`, can skip) | `/var/run/torpedo/torpedo.sock` | `ray-proto/src/ipc.rs::socket_path()` |
| ALPN protocol prefix | `rayfish/net/<version>/<pubkey-prefix>` | `torpedo/net/<version>/<pubkey-prefix>` | wherever `MESH_PROTOCOL_VERSION`/ALPN strings are built (search `transport::` per `CLAUDE.md`'s architecture notes); this is a wire-level string, changing it is what actually guarantees this fork's traffic is never confused with genuine rayfish traffic |

### 4.2 Do NOT rename these (they reference something else, not this project's identity)

- `src/config.rs`, the `"rayfish"` **relay preset name** (used by `ray config set relay rayfish`,
  confirmed at `src/config.rs:229` and its test cases around line 1496-1588). This refers to
  **upstream's own hosted relay servers**, an external service you would still want to be able
  to point at by that name. Renaming this string would silently break that feature rather than
  rebrand anything. Leave it alone.
- `src/stats.rs`, `#[metrics(name = "rayfish", default)]` and `src/main.rs`'s OpenTelemetry
  `with_service_name("rayfish")` / `provider.tracer("rayfish")`. Purely cosmetic (metric/trace
  labeling), no functional effect either way. Optional, not required for this fork, skip unless
  there's spare time.
- Comment/doc-string mentions of "rayfish" describing what the original project is (e.g. module
  doc comments). Leave descriptive comments alone unless they're actively confusing; this is
  still a fork *of* rayfish, saying so in a comment is accurate, not a rename target.

---

## 5. `spec/design_spec.py` (write this content, adapt as needed)

Populate `spec/design_spec.py` in `~/code/torpedo` with the following before writing any
implementation code (per the libspec workflow: spec first, code second, the spec is the target,
not something you edit to match whatever the code ends up doing). Follow the existing
`Requirement`/`Constraint`/`Feature`/`UserStory`/`Spec` class shapes exactly as libspec expects
them (see `~/.claude/MEMORY/RESEARCH_libspec.md` for the class reference if anything below is
unclear).

```python
# spec/design_spec.py
from libspec import Requirement, Constraint, Feature, UserStory, Spec


class ForkIntent(UserStory):
    """Fork rayfish so its overlay IPv4 subnet is configurable at network-creation
    time, instead of hardcoded to 100.64.0.0/10, so it can run alongside an
    already-active Tailscale client on the same host.

    Priority: high.
    User journey: create a network with a custom --subnet -> join it from a
    second machine also running Tailscale -> both machines reach each other over
    the fork's mesh while Tailscale keeps working unaffected on both.
    Acceptance: `torpedo create --subnet <cidr>` succeeds on a host with an active
    Tailscale client; a second host joins successfully; `torpedo status` on both
    shows a live peer; Tailscale connectivity is unaffected throughout.
    """


class SubnetField(Requirement):
    """GroupBlob (src/membership.rs) gains `subnet: Option<(Ipv4Addr, u8)>`,
    following the existing `name: Option<String>` field's serde pattern
    (#[serde(default, skip_serializing_if = "Option::is_none")]). This is the
    network-wide signed source of truth every peer derives addresses against."""
    req_id = "SUBNET-001"


class SubnetCliFlag(Requirement):
    """`torpedo create` gains `--subnet <CIDR>` (parsed to Ipv4Addr + prefix len).
    Omitting it falls back to 100.64.0.0/10, unchanged from today's behavior."""
    req_id = "SUBNET-002"


class DeriveIpParameterized(Requirement):
    """derive_ip_with_index() (src/membership.rs) takes the network's subnet as
    a parameter instead of the hardcoded 0x6440_0000 base and fixed 22-bit host
    mask. Host-bit width is computed as 32 - prefix_len at call time."""
    req_id = "SUBNET-003"


class RangeValidationParameterized(Requirement):
    """ensure_in_cgnat_range() (src/membership.rs) validates a candidate IP
    against the network's own configured subnet (read from GroupBlob), not a
    single hardcoded 100.64.0.0/10 constant."""
    req_id = "SUBNET-004"


class TunCreateParameterized(Requirement):
    """tun::create() (src/tun.rs) computes its netmask from the configured
    prefix length and its gateway as (base + 1), instead of the hardcoded
    (255, 192, 0, 0) netmask and 100.64.0.1 gateway."""
    req_id = "SUBNET-005"


class ConflictCheckRemoved(Requirement):
    """check_cgnat_conflict() (src/tun.rs) and its call site are removed. This
    fork deliberately uses a subnet outside 100.64.0.0/10, so there is nothing
    for this check to protect against, and it is what currently blocks startup
    next to Tailscale."""
    req_id = "SUBNET-006"


class MagicDnsRelocated(Requirement):
    """MAGIC_DNS_V4 (src/dns.rs) is computed as an offset within the configured
    subnet instead of the fixed 100.100.100.53 literal. Assumes the configured
    subnet is /24 or larger."""
    req_id = "SUBNET-007"


class PtrHandlerParameterized(Requirement):
    """The PTR/reverse-lookup NXDOMAIN range check in src/dns.rs (~line 246 as
    of commit 9e142411) mirrors whichever range check RangeValidationParameterized
    (SUBNET-004) implements, so both stay consistent."""
    req_id = "SUBNET-008"


class BinaryRenamed(Requirement):
    """The `ray` binary is renamed `torpedo` (Cargo.toml [[bin]], build output,
    contrib/rayfish.service's ExecStart path)."""
    req_id = "RENAME-001"


class ServiceRenamed(Requirement):
    """systemd service, unit file, and all systemctl invocations referring to
    "rayfish" are renamed to "torpedo" (src/cli/service.rs, src/cli/update.rs,
    src/update.rs, contrib/rayfish.service renamed to contrib/torpedo.service)."""
    req_id = "RENAME-002"


class PathsRenamed(Requirement):
    """Config dir (/etc/rayfish -> /etc/torpedo, src/config.rs), log dir
    (/var/log/rayfish -> /var/log/torpedo, src/logdir.rs), socket path
    (/var/run/rayfish/rayfish.sock -> /var/run/torpedo/torpedo.sock,
    ray-proto/src/ipc.rs), and the Unix group name (rayfish -> torpedo,
    src/cli/service.rs) are all updated consistently."""
    req_id = "RENAME-003"


class AlpnRenamed(Requirement):
    """The mesh ALPN protocol prefix (rayfish/net/<version>/...) is changed to
    torpedo/net/<version>/... so this fork's wire traffic can never be confused
    with genuine rayfish traffic."""
    req_id = "RENAME-004"


class RelayPresetUntouched(Constraint):
    """The "rayfish" relay preset name in src/config.rs (used by `ray config set
    relay rayfish`) must NOT be renamed. It refers to upstream's own hosted
    relay infrastructure, an external service name, not this fork's identity.
    Renaming it would silently break that feature."""
    constraint_id = "CON-001"
    enforcement_logic = "{{ relay_preset_untouched.value == 'rayfish' }}"


class NoLeftoverHardcodedCgnat(Constraint):
    """No remaining hardcoded 100.64.0.0/10-family literals in the touched
    files, other than the CLI default fallback value itself (which is an
    intentional, explicit default, not a hidden hardcode)."""
    constraint_id = "CON-002"
    enforcement_logic = "{{ grep_hardcoded_cgnat.unexpected_count }} == 0"


class BuildPasses(Constraint):
    """cargo build succeeds."""
    constraint_id = "CON-003"
    enforcement_logic = "{{ build.success }}"


class ClippyClean(Constraint):
    """cargo clippy --all-targets is warning-free, per this repo's own
    CONTRIBUTING.md convention."""
    constraint_id = "CON-004"
    enforcement_logic = "{{ clippy.warnings }} == 0"


class TestsPass(Constraint):
    """cargo test passes."""
    constraint_id = "CON-005"
    enforcement_logic = "{{ test.pass }}"


class ForkSpec(Spec):
    def modules(self):
        return [
            ForkIntent,
            SubnetField, SubnetCliFlag, DeriveIpParameterized,
            RangeValidationParameterized, TunCreateParameterized,
            ConflictCheckRemoved, MagicDnsRelocated, PtrHandlerParameterized,
            BinaryRenamed, ServiceRenamed, PathsRenamed, AlpnRenamed,
            RelayPresetUntouched, NoLeftoverHardcodedCgnat,
            BuildPasses, ClippyClean, TestsPass,
        ]
```

Then, per the libspec workflow:

```bash
cd ~/code/torpedo
~/code/libspec/.venv/bin/libspec build spec/design_spec.py
~/code/libspec/.venv/bin/libspec diff
```

Review the diff output before writing any implementation code.

---

## 6. Verification script (`reconcile.py`)

libspec's `Constraint.enforcement_logic` is a Jinja2 expression evaluated against a context dict
you supply, there is no built-in Rust/cargo awareness (its example usage elsewhere in this
environment is for a CAD tool with a completely different context schema, do not copy that
schema, it does not apply here). Write `reconcile.py` in `~/code/torpedo` fresh, producing a
context matching what `design_spec.py` above expects:

```python
#!/usr/bin/env python3
# reconcile.py -- run from ~/code/torpedo
# Usage: python reconcile.py
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


def check_hardcoded_cgnat(allowed_default_line_substrings=("100.64.0.0/10",)) -> dict:
    """Grep the touched files for leftover 100.64/100.100 literals. The CLI
    default fallback value is expected to still mention 100.64.0.0/10 once
    (as the documented default), anything beyond that is unexpected."""
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


if __name__ == "__main__":
    ctx = {
        "build": check_build(),
        "clippy": check_clippy(),
        "test": check_tests(),
        "grep_hardcoded_cgnat": check_hardcoded_cgnat(),
        "relay_preset_untouched": check_relay_preset(),
    }
    print(json.dumps(ctx, indent=2))
    ok = (
        ctx["build"]["success"]
        and ctx["clippy"]["warnings"] == 0
        and ctx["test"]["pass"]
        and ctx["grep_hardcoded_cgnat"]["unexpected_count"] == 0
        and ctx["relay_preset_untouched"]["value"] == "rayfish"
    )
    sys.exit(0 if ok else 1)
```

Run it from the fork's root once implementation is underway:

```bash
cd ~/code/torpedo
python3 reconcile.py
```

This checks the automatable constraints (`CON-001` through `CON-005`). It does **not** check the
`Requirement` classes (SUBNET-*, RENAME-*), those are structural/design requirements, not
boolean-checkable facts, confirm them by reading the diff and the code directly against section 3
and section 4 above, then call `store_implemented()` per requirement once satisfied (see
`~/.claude/MEMORY/RESEARCH_libspec.md` if the exact MCP tool call signature is needed).

---

## 7. Manual acceptance test (outside libspec, cannot be automated from source alone)

Once `reconcile.py` passes clean, the actual proof this fork works is a live test, not just a
green build:

```bash
# On AURUS, with Tailscale already active (it will be, this is the whole point):
sudo torpedo up                              # must NOT fail with a CGNAT conflict error
torpedo create --subnet 10.88.0.0/16 --hostname aurus
# note the invite code / room id printed

# Confirm Tailscale is unaffected throughout:
tailscale status                           # should show the same peers as before, unchanged

# From a second machine (this laptop, or wherever), also with Tailscale active:
sudo torpedo up
torpedo join <invite-code> --hostname laptop

# From either machine:
torpedo status                               # both peers visible
torpedo ping <other-hostname>                # real round-trip over the new mesh
tailscale status                           # still fine, on both machines
```

If any step here fails, that is the real signal, not `reconcile.py`'s output. Report back with
the exact command and output rather than guessing at a fix.

---

## 8. Explicitly out of scope for this fork

- Upstreaming (separate, already-written document: the `FEATURE_REQUEST_configurable_subnet.md`
  in the original `~/code/rayfish` checkout covers the full, general, backward-compatible
  version of this same idea, for submission to the actual project).
- ALPN version-gating for mixed old/new-binary peers.
- macOS support for the new subnet logic.
- IPv6-only mode (a different idea, discussed separately, not part of this spec).
- Cosmetic renames listed as optional in section 4.2.

---

## 9. Rules for the implementing instance

- **Spec first, code second.** Do not start editing `src/` before `spec/design_spec.py` exists
  and `libspec build` + `libspec diff` have been reviewed.
- **Never edit the spec to match code that doesn't satisfy it.** If a requirement turns out to be
  wrong or infeasible, stop and report back rather than quietly loosening it.
- **Confirm the fork name before writing code.** Section 1 is provisional.
- **Do not blindly find-and-replace "rayfish" → "torpedo".** Section 4.2 exists because a naive
  replace breaks the relay-preset feature. Check every occurrence individually.
- **This fork's file/line references were verified against commit `9e142411`** in the rayfish
  repo. If the clone in Phase 0 lands on a different commit, re-verify each location in section 3
  before editing, do not assume line numbers still match.
