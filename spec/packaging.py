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
