# tetron TODO

## Recently completed

- **Invite key admission** (Phases 1-4): invite store, IPC handlers, CLI (create/list/revoke), post-create auto-mint, e2e tested on 3 machines. Room-id joins still queue for live approval (both paths coexist).
- **Old torpedo cleanup**: service stopped, binary/config removed on AORUS, xps-17, and SB-OS.
- **E2E test results** logged in `docs/TESTING.md` Stage 9.

## Packaging

- **Build a .deb package** for tetron: systemd service file, config dir, binary, postinst/prerm scripts. Simplifies install on Debian/Ubuntu vs the current `sudo tetron install` from a loose binary.

## UX cleanup

- **`tetron join --name` rename to `--local-nickname`**: the current `--name` flag on join is a local-only alias, but `--name` on create sets the published network name. Same flag, different scopes, confusing. Rename to `--local-nickname` on join, keep `--name` on create.

## High priority

- **Reusable keys (--reusable)**: add `--reusable` flag to `tetron invite <net> create` — adds hash to `GroupBlob.reusable_keys`, signs + republishes blob. Any coordinator validates against the blob.
- **Cross-coordinator invite gossip**: propagate `InviteShare`/`InviteUsed` between coordinators so any coordinator can validate a single-use invite, not just the minting one. Required for multi-coordinator networks where the minter may be offline.
