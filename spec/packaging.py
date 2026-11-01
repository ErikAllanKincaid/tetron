from libspec import Requirement

# ==========================================================================
# Debian packaging and package-aware install (PACKAGING-*)
#
# Origin: USER request 2026-10-26 (TetronOS): publish .deb assets from release
# CI and make tetron behave when the daemon is installed by a package manager
# instead of by `tetron install` / install-tetron-suite.sh.
# Plan: ~/code/tetron-os/DO-NOT-COMMIT/PLAN_tetron_suite_deb_in_github_ci_and_install_modes.md
#
# DEPENDENCY ORDERING (derived from the docstrings below; commit in this order):
#   PACKAGING-001, -002, -003 have no dependency on each other and can land in
#     any order or in parallel.
#   PACKAGING-004 (install) and PACKAGING-005 (uninstall) each need -001 (owner
#     detection) and -003 (vendor-dir unit detection). 004 and 005 are
#     independent of each other.
#   PACKAGING-006 (release deb assets) needs -002 (the packaged unit file) and
#     the tetron-packaging build-deb action (separate repo, already published).
#   PACKAGING-007 (suite script --method) needs -006 to exist as a release
#     asset, but not the Rust changes.
#   PACKAGING-008 (shadow detection in the suite script) needs -007.
# ==========================================================================


class PackageOwnerDetection(Requirement):
    """REQUIREMENT-ID: PACKAGING-001

    A helper `package_owner(path) -> Option<String>` in `src/cli/service.rs`
    (or a small module it uses) returns the name of the package that owns the
    file at `path`, or `None` when no package manager owns it.

    - Debian family only for now: asks `dpkg -S <canonical path>`. dpkg being
      absent, the command failing, or the file being unowned all yield `None`;
      it never returns an error and never prints.
    - The path is canonicalised first (symlinks resolved) and the Linux
      ` (deleted)` suffix is stripped with the existing `strip_deleted_suffix`.
    - The command runner is injected (a closure/trait parameter) so the logic
      is unit-tested without dpkg; the production wrapper passes the real
      runner. Parsing handles `pkg: /path`, `pkg:arch: /path`, and diversion
      lines (`diversion by X from: ...`) without panicking.
    - No network, no root required.

    Used by PACKAGING-004 and PACKAGING-005. No dependency on other
    PACKAGING requirements.
    """
    req_id = "PACKAGING-001"


class PackagedUnitFile(Requirement):
    """REQUIREMENT-ID: PACKAGING-002

    `contrib/tetron.packaged.service` is the systemd unit a package installs
    at `/usr/lib/systemd/system/tetron.service`: identical to
    `contrib/tetron.service` except `ExecStart=/usr/bin/tetron daemon` and
    with no `__TETRON_*__` placeholder `Environment=` lines. A unit test
    asserts the two files differ only in those respects, so the packaged unit
    cannot drift from the installed one. The packaged unit is not enabled by
    the package.

    No dependency on other PACKAGING requirements. PACKAGING-006 consumes it.
    """
    req_id = "PACKAGING-002"


class VendorServiceUnitDetection(Requirement):
    """REQUIREMENT-ID: PACKAGING-003

    `service_unit_exists()` is true on Linux when the tetron unit exists in
    any of `/etc/systemd/system/tetron.service`,
    `/usr/lib/systemd/system/tetron.service`, or
    `/lib/systemd/system/tetron.service`. Without this, `tetron start`,
    `stop`, `restart` would report a package-installed service as not
    installed. The directory list is a parameter of an inner function so a
    unit test can use a temp directory. macOS is unchanged.

    No dependency on other PACKAGING requirements.
    """
    req_id = "PACKAGING-003"


class InstallIsPackageAware(Requirement):
    """REQUIREMENT-ID: PACKAGING-004

    Depends on PACKAGING-001 (owner detection) and PACKAGING-003 (vendor unit
    detection). Independent of PACKAGING-005.

    On Linux, when the running executable is owned by a package
    (`package_owner(current_exe)` is `Some`) and a vendor unit exists,
    `tetron install`:

    - does NOT write `/etc/systemd/system/tetron.service`;
    - still runs `systemctl enable` and `restart`, waits for the daemon,
      sends Resume and grants operator to `$SUDO_USER` exactly as today;
    - for each of `--config-dir`, `--log-dir`, `--socket-path` that is given,
      writes a systemd drop-in `/etc/systemd/system/tetron.service.d/10-paths.conf`
      with the matching `Environment=` lines (the standard way to override a
      vendor unit) and runs `daemon-reload`; with none given it leaves any
      existing drop-in untouched;
    - if a legacy `/etc/systemd/system/tetron.service` already exists it keeps
      taking precedence over the vendor unit (systemd rule), so install prints
      one line naming that file and how to remove it;
    - prints one line saying the service is managed by package `<name>`.

    When the executable is not package-owned, behavior is exactly as before
    (writes the /etc unit with the exe path substituted).
    """
    req_id = "PACKAGING-004"


class UninstallIsPackageAware(Requirement):
    """REQUIREMENT-ID: PACKAGING-005

    Depends on PACKAGING-001 and PACKAGING-003. Independent of PACKAGING-004.

    `tetron uninstall` on Linux:

    - removes `/etc/systemd/system/tetron.service` (a file `tetron install`
      itself created) exactly as today;
    - never deletes a vendor unit under `/usr/lib` or `/lib`: when only a
      vendor unit exists (or after removing the /etc one, a vendor unit
      remains) it runs `systemctl disable --now tetron` and prints that the
      service is package-managed and the package is removed with the package
      manager (`sudo apt remove tetron` when the owner is a dpkg package);
    - prints `Service not installed.` only when no unit exists in any of the
      three directories;
    - leaves the selfcapture teardown at the top unchanged.
    """
    req_id = "PACKAGING-005"


class ReleaseDebAssets(Requirement):
    """REQUIREMENT-ID: PACKAGING-006

    Depends on PACKAGING-002.

    Each GitHub release publishes `tetron_<version>-1_amd64.deb` and
    `tetron_<version>-1_arm64.deb` with `.sha256` files next to the existing
    raw binary assets, which are unchanged. The debs are built in
    `.github/workflows/release.yml` from the same musl binaries the job
    uploads (`tetron-linux-x86_64-musl`, `tetron-linux-aarch64-musl`) by the
    `build-deb` action of `ErikAllanKincaid/tetron-packaging`, pinned by full
    commit SHA, and are tested in a clean debian:trixie container before the
    upload step.

    `packaging/deb.toml` installs `/usr/bin/tetron` (0755) and the packaged
    unit from PACKAGING-002 at `/usr/lib/systemd/system/tetron.service`
    (0644); Depends `iproute2` (the daemon runs `ip`); `postinst` only
    creates the `tetron` system group with `groupadd --system` (never
    `addgroup`) and does not start or enable the service or create any
    configuration or identity; the version is the release tag without the v
    plus revision 1. The maintainer comes from the repository variable
    `DEB_MAINTAINER`; nothing is hardcoded.
    """
    req_id = "PACKAGING-006"


class SuiteInstallerMethodFlag(Requirement):
    """REQUIREMENT-ID: PACKAGING-007

    Depends on PACKAGING-006 existing as a release asset.

    `contrib/install-tetron-suite.sh` gains `--method auto|binary|deb`
    (default `auto`). `binary` is today's behavior. `deb` downloads the
    matching `.deb` asset for the component, verifies its sha256, and installs
    it with `sudo apt-get install -y ./file.deb`, then runs the component's
    own `install` subcommand (now package-aware). `auto` upgrades a component
    with its `.deb` if dpkg owns the installed binary, and with the raw binary
    otherwise; with no existing install it uses `binary` (no behavior change
    for current users). `deb` on a host without dpkg and apt-get (or on
    macOS) is an error with a clear message. `--check` reports, per
    component, whether the installed copy is a package or a binary.

    The script never writes into a file a package owns:
    - `auto` with a package-owned installed copy and no `.deb` asset in the
      latest release for that component (for example a component that does
      not publish debs yet) skips the component with a warning that names the
      package and how to override, instead of overwriting the packaged file;
    - `binary` with a package-owned installed copy installs the raw binary to
      the default directory (`/usr/local/bin`) instead of overwriting the
      packaged file, and PACKAGING-008 then reports the shadowing;
    - `deb` with no `.deb` asset for the component is an error naming the
      component and release.

    The `.deb` is installed to its own path (`/usr/bin/<binary>`), and the
    component's `install` subcommand is run on that path, not on whichever
    copy comes first on `PATH`. Asset discovery reads the asset list of the
    latest release (names `<binary>_<version>-<revision>_<amd64|arm64>.deb`)
    so a packaging-only revision bump is found without guessing.
    """
    req_id = "PACKAGING-007"


class SuiteInstallerShadowDetection(Requirement):
    """REQUIREMENT-ID: PACKAGING-008

    Depends on PACKAGING-007.

    `install-tetron-suite.sh` detects a component installed twice: a
    package-owned copy (for example `/usr/bin/tetron`) and a raw copy in
    `/usr/local/bin`, which comes first on `PATH` and shadows the packaged
    one. It reports the shadowing in `--check` and before installing, and
    never deletes either copy without the user answering yes.
    """
    req_id = "PACKAGING-008"


class PurgeKeepsNodeDataAndSaysSo(Requirement):
    """REQUIREMENT-ID: PACKAGING-013

    Depends on PACKAGING-006. Independent of PACKAGING-014..016.

    USER decision 2026-10-30: purging the package keeps everything the daemon
    created: `/etc/tetron` (node identity `secret_key`, settings, networks,
    blobs, peer cache), `/var/log/tetron` and the `tetron` group. The identity
    is user data (it is created when the user runs `tetron install`, like
    `~/.ssh` keys and unlike a package-generated host key); losing it drops
    the node from every network and its admin role.

    - No maintainer script ever removes `/etc/tetron`, `/var/log/tetron` or
      the `tetron` group.
    - `postrm purge` prints a notice when `/etc/tetron` or `/var/log/tetron`
      exists: what was kept, why, and the manual removal command
      `sudo rm -rf /etc/tetron /var/log/tetron && sudo groupdel tetron`.
    - The same statement is in the deb `long_description`, the README
      install section and the `tetron uninstall` output (some apt frontends
      hide maintainer script output). `tetron uninstall` names the config
      and log directories actually in use.

    Test: purge in a VM keeps `secret_key` (same checksum) and prints the
    notice.
    """
    req_id = "PACKAGING-013"


class InstallGroupCheckIsSilent(Requirement):
    """REQUIREMENT-ID: PACKAGING-014

    Independent of the other requirements.

    `tetron install` printed a stray `tetron:x:989:` line: the check whether
    the `tetron` group exists ran `getent group tetron` with its output going
    to the terminal. The check discards `getent` output; only its exit status
    is used.
    """
    req_id = "PACKAGING-014"


class InstallStartedLineReadsCorrectly(Requirement):
    """REQUIREMENT-ID: PACKAGING-015

    Independent of the other requirements.

    `tetron install` printed "tetron service started. already active" when
    the VPN was already active: the daemon's reply to the resume request was
    appended to a fixed sentence. The line reads
    "tetron service started; VPN: <reply>" (for example
    "tetron service started; VPN: already active"), and just
    "tetron service started." when the reply is empty.
    """
    req_id = "PACKAGING-015"


class InstallMigratesLegacyEtcUnit(Requirement):
    """REQUIREMENT-ID: PACKAGING-016

    Depends on PACKAGING-004 and amends its notice-only handling. Independent
    of PACKAGING-013..015.

    USER decision 2026-10-31. When the `tetron` executable is package-owned,
    a vendor unit exists, and `/etc/systemd/system/tetron.service` is a unit
    `tetron install` wrote earlier (its `Description=` is the tetron template
    description and its `ExecStart` runs a `tetron` binary with the `daemon`
    argument), that unit overrides the packaged one. `tetron install` then:

    - at a terminal (stdin is a TTY), asks once whether to move it aside
      (default yes). On yes: path overrides found in it
      (`Environment=TETRON_CONFIG_DIR/LOG_DIR/SOCKET_PATH`) are carried into
      the `10-paths.conf` drop-in, with flags given on this run taking
      precedence; the unit is disabled (removing its enable links while the
      file exists), moved to `/var/backups/tetron/tetron.service.<timestamp>`
      (never deleted), and `systemctl daemon-reload` runs; the normal enable
      and restart that follow then use the packaged unit;
    - without a terminal, or on no, prints the existing notice and changes
      nothing.

    A unit in `/etc` that tetron did not write (other description or
    ExecStart) is never touched; the notice is printed.
    """
    req_id = "PACKAGING-016"


class SuiteInstallerVerifiesWhatRuns(Requirement):
    """REQUIREMENT-ID: PACKAGING-010

    Depends on PACKAGING-007. Independent of PACKAGING-009; either can land
    first.

    Found 2026-10-31 in the raw-to-deb VM test (tetron-os
    `TEST_tetron_raw_to_deb_upgrade_VM_2026-10-31.md`): after
    `--method deb` installed `tetron` 0.14.5-1, the script printed
    `[pass] core: version 0.14.5` and exited 0, while the daemon and the
    `tetron` first on `PATH` were still the raw 0.14.3 in `/usr/local/bin`,
    started by the old `/etc/systemd/system/tetron.service`. The version was
    read from the path just installed, not from what runs.

    After installing a component, `install-tetron-suite.sh` checks what will
    actually run, comparing resolved paths (so `/bin` and `/usr/bin` on a
    merged-/usr system are the same file):

    - the first copy of the binary on `PATH` must be the installed copy;
    - for a component whose service is a system unit (core), on Linux with
      `systemctl`, the path in the unit's `ExecStart` must be the installed
      copy. When systemd reports no `ExecStart` (no systemd, a timer unit),
      this check is skipped.

    If either check fails the component is reported with `[fail]`, naming
    the copy that runs instead and its version, and is listed in the failed
    summary (exit 1). The version printed on success is the version of the
    copy that runs. This replaces the warning-only `warn_if_shadowed` after
    an install.
    """
    req_id = "PACKAGING-010"


class SuiteInstallerConvertsRawInstallToPackage(Requirement):
    """REQUIREMENT-ID: PACKAGING-009

    Depends on PACKAGING-007 (the `--method deb` conversion) and amends
    PACKAGING-008 for this one case. Independent of PACKAGING-010.

    Found 2026-10-31 in the same VM test: `--method deb` on a raw install
    installed the package but left the raw copy in `/usr/local/bin` (first on
    `PATH`) and the `/etc/systemd/system/tetron.service` that starts it, so
    the old daemon kept running. Finishing by hand (remove the `/etc` unit and
    the raw copy, `daemon-reload`, `tetron install`) worked and survived a
    reboot, but left the `multi-user.target.wants` link pointing at the
    deleted `/etc` unit.

    When `--method deb` converts a component whose first copy on `PATH` is a
    raw (not package-owned) binary, the conversion is one step:

    1. Before anything changes, the plan is printed (package to install, unit
       file to move, raw copy to remove) and confirmed once at a terminal.
       Without a terminal it proceeds with a warning, because `--method deb`
       is the explicit request to convert. Declining skips the component.
    2. The `.deb` is installed. If that fails nothing else is touched.
    3. Unit migration, before the raw copy is removed: when the component has
       a system unit (core: `tetron.service`) in `/etc/systemd/system` that no
       package owns and whose `ExecStart` runs the raw copy, it is disabled
       (removing its enable links while the file still exists), moved to
       `/var/backups/tetron/<unit>.<timestamp>` (never deleted, since it may
       hold local edits), and `systemctl daemon-reload` runs. A unit that
       runs some other path is left alone; PACKAGING-010 then reports it.
    4. The raw copy is removed (it is a release download, not user data).
    5. The packaged binary's `install` subcommand runs as before, enabling
       and restarting the service from the packaged unit, so the enable link
       is re-made for the packaged unit.

    If step 3 or 4 fails the component is reported as failed and later steps
    are skipped. The daemon keeps running the old binary until step 5
    restarts it, so peers drop only once. Running `--method deb` again on a
    half-converted host (package installed, raw copy still first on `PATH`)
    completes the conversion.

    The unit directory, backup directory and package binary directory can be
    overridden with `TETRON_SUITE_UNIT_DIR`, `TETRON_SUITE_BACKUP_DIR` and
    `TETRON_SUITE_PKG_BIN_DIR` for the hermetic tests only. apt-get runs with
    `DEBIAN_FRONTEND=noninteractive`, since the script already asked.
    """
    req_id = "PACKAGING-009"


class SuiteInstallerAutoPrefersDebOnFreshHost(Requirement):
    """REQUIREMENT-ID: PACKAGING-011

    Depends on PACKAGING-007 and amends its rule that a fresh install uses
    the raw binary. Independent of PACKAGING-012.

    USER decision 2026-10-31: on a host with dpkg and apt-get the deb is the
    install method. With `--method auto` (the default) and no installed copy
    of the component, `install-tetron-suite.sh` installs the release `.deb`
    when the latest release has one for this architecture, and the raw
    binary otherwise (no deb asset, no dpkg/apt-get, macOS).

    An existing raw install under `auto` is still upgraded with the raw
    binary, as before: converting it changes how the host is managed, so it
    stays an explicit `--method deb` (PACKAGING-009). In that case the script
    prints one info line saying that `--method deb` converts it to the
    package.
    """
    req_id = "PACKAGING-011"


class SuiteInstallerBinaryReplacesPackage(Requirement):
    """REQUIREMENT-ID: PACKAGING-012

    Depends on PACKAGING-007 and PACKAGING-010, and replaces PACKAGING-007's
    rule that `--method binary` on a package-owned copy installs a second,
    shadowing raw copy in `/usr/local/bin`. Independent of PACKAGING-011.

    USER decision 2026-10-31: one install method per host. `--method binary`
    on a host where a package owns the component replaces the package with
    the raw install in one confirmed step:

    1. Guards, checked before anything changes. The script refuses, prints
       the manual commands (`sudo apt-get remove <pkg>`, then rerun with
       `--method binary`), and lists the component as failed when
       (a) `apt-get -s remove <pkg>` shows that other packages would be
       removed too (they are named; for example a TetronOS metapackage that
       depends on it), or (b) there is no terminal: removing a package
       unattended needs a human who saw the plan. This is a deliberate
       difference from the deb conversion (PACKAGING-009), which only
       installs.
    2. The plan is printed and confirmed once. Declining skips the
       component.
    3. The raw binary is downloaded, verified and installed to
       `/usr/local/bin` before the package is removed, so the service
       restarts only once.
    4. `apt-get remove` (never purge) removes the package. The node identity
       and everything else the daemon created stay (purge policy).
    5. The raw binary's `install` subcommand runs, which writes the `/etc`
       unit and restarts the service.
    6. PACKAGING-010 checks that the raw copy is what runs.

    If step 3 or 4 fails, later steps are skipped and the component is
    reported as failed; a raw copy placed in step 3 before a failed removal
    is left in place and named.
    """
    req_id = "PACKAGING-012"


class ReleaseBumpScript(Requirement):
    """REQUIREMENT-ID: RELEASE-001

    `contrib/bump-version.sh [X.Y.Z|patch|minor|major] [-n] [-y]` prepares a
    release commit so a bump can not miss a site. A version is kept at three
    sites, all in lockstep: `[package] version` in `Cargo.toml`, the
    `tetron-proto = { path = "tetron-proto", version = "X.Y.Z" }` dependency
    requirement in `Cargo.toml`, and `[package] version` in
    `tetron-proto/Cargo.toml`. `Cargo.lock` and the `CHANGELOG.md` heading
    follow them.

    The script:
    - prints the current state: the three sites, the two `Cargo.lock`
      entries, the branch, whether the tree is clean, the newest `v*` tag and
      the number of entries under `## [Unreleased]`;
    - takes the new version as an argument, `patch` (the default), `minor`
      or `major`;
    - refuses, naming each reason, when the three sites or `Cargo.lock`
      disagree, the new version is not `X.Y.Z` or not greater than the
      current one, tag `vX.Y.Z` or branch `chore/release-X.Y.Z` exists, the
      tree is not clean, the branch is not `main`, or `## [Unreleased]` has
      no entries;
    - with `-n` stops after the plan and changes nothing; otherwise asks
      before changing anything unless `-y` is given;
    - creates `chore/release-X.Y.Z`, writes the three sites, turns
      `## [Unreleased]` into `## [Unreleased]` followed by
      `## [X.Y.Z] - <today>`, re-reads all three sites, runs `cargo build`
      then `cargo build --locked` (the v0.9.1 failure: a stale lockfile fails
      every release matrix target) and `python3 reconcile.py`, and commits
      `chore: bump version to X.Y.Z` with no other text;
    - never pushes, tags or contacts a remote; it prints the merge, tag and
      push commands for USER;
    - on a failure after the branch exists, prints how to return to `main`
      and delete the branch.

    The build and gate commands can be replaced through `BUMP_BUILD_CMD` and
    `BUMP_GATE_CMD` so the hermetic test runs in seconds.
    """
    req_id = "RELEASE-001"
