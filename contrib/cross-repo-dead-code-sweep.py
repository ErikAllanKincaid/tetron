#!/usr/bin/env python3
# cross-repo-dead-code-sweep.py -- automates steps 1-2 of the procedure at
# docs/tetron-workflow.md #12 ("Cross-repo dead-code sweep"). Run by hand
# before cutting a release, or after a feature removal -- NOT part of
# reconcile.py or CI, deliberately: it is addon-aware (it greps
# tetron-mobile/tetron-webui/tetron-systray), and core's per-commit gate
# must not require sibling repos to exist on disk or be coupled to
# addon-repo layout. Lives in contrib/, same as this repo's other
# addon-aware tooling (install-tetron-suite.sh).
#
# SUNSET CANDIDATE, tracked at DO-NOT-COMMIT/TODO_DETAILS.md
# #migration-era-tooling -- built 2026-08-09 specifically to catch orphans
# left behind by the pitopi->rayfish->torpedo->tetron rename and its
# MINIMAL-*/TREE-SHAKE-* cleanup passes. A from-scratch codebase with no
# rename/mass-removal history behind it should rarely produce this class
# of dead code, so this script's whole reason to exist goes away once
# that migration's cleanup is done. Removal trigger: a clean run (all
# three real sibling repos present) reports zero candidates on two
# consecutive release cuts -- delete this file at that point, no
# replacement needed.
#
# Usage (from ~/code/tetron):
#   python3 contrib/cross-repo-dead-code-sweep.py
#   python3 contrib/cross-repo-dead-code-sweep.py --consumer ../tetron-mobile --consumer ../tetron-webui
#
# What this does NOT do (still manual, per docs/tetron-workflow.md #12
# steps 3-4): exclude trait-impl methods, derive-macro-only fields, or
# documented compatibility scaffolding; determine *why* something went
# dead via `git log --follow -S<symbol>`. Treat this script's output as
# candidates to review, not confirmed findings -- it says so in its own
# output.
import argparse
import re
import sys
from pathlib import Path

PUB_ITEM = re.compile(
    r"^pub\s+(fn|struct|enum|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
PUB_METHOD = re.compile(r"^\s+pub\s+(fn)\s+([A-Za-z_][A-Za-z0-9_]*)")
IMPL_HEADER = re.compile(r"^impl\b")
TRAIT_IMPL_HEADER = re.compile(r"^impl(<[^>]*>)?\s+[\w:]+(<[^>]*>)?\s+for\s+")

DEFAULT_CONSUMERS = ["../tetron-mobile", "../tetron-webui", "../tetron-systray"]


def rust_files(root: Path) -> list[Path]:
    if not root.is_dir():
        return []
    return [p for p in root.rglob("*.rs") if "target" not in p.parts]


def enumerate_pub_items(repo_root: Path) -> list[tuple[str, str, Path, int]]:
    """Top-level pub fn/struct/enum/const/static, PLUS pub fn methods inside
    inherent (non-trait) impl blocks, in src/**/*.rs (excluding main.rs and
    cli/**, the binary's own dispatch surface) + tetron-proto/src/**/*.rs.
    Trait-impl method bodies are skipped (rustfmt convention: impl blocks
    close with a column-0 `}`, so a simple "last impl header seen" state
    machine works without a real parser -- same pragmatic-regex spirit as
    reconcile.py's own checks). Returns (kind, name, file, line_no)."""
    items = []
    targets = [
        p
        for p in rust_files(repo_root / "src")
        if p != repo_root / "src" / "main.rs" and "cli" not in p.relative_to(repo_root / "src").parts
    ]
    targets += rust_files(repo_root / "tetron-proto" / "src")
    for f in targets:
        in_trait_impl = False
        for i, line in enumerate(f.read_text().splitlines(), start=1):
            if IMPL_HEADER.match(line):
                in_trait_impl = bool(TRAIT_IMPL_HEADER.match(line))
                continue
            if line == "}":
                in_trait_impl = False
                continue
            m = PUB_ITEM.match(line)
            if m:
                items.append((m.group(1), m.group(2), f, i))
                continue
            if not in_trait_impl:
                m = PUB_METHOD.match(line)
                if m:
                    items.append((m.group(1), m.group(2), f, i))
    return items


def count_usages(name: str, files: list[Path]) -> int:
    pat = re.compile(rf"\b{re.escape(name)}\b")
    total = 0
    for f in files:
        total += len(pat.findall(f.read_text()))
    return total


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--consumer",
        action="append",
        default=None,
        help=f"path to a downstream repo to grep (repeatable). Default: {DEFAULT_CONSUMERS}",
    )
    args = ap.parse_args()
    consumer_paths = [Path(p) for p in (args.consumer or DEFAULT_CONSUMERS)]

    repo_root = Path(__file__).resolve().parent.parent
    present = [p for p in consumer_paths if p.is_dir()]
    missing = [p for p in consumer_paths if not p.is_dir()]
    for p in missing:
        print(f"[warn] consumer repo not found, skipping: {p}", file=sys.stderr)

    own_files = rust_files(repo_root / "src") + rust_files(repo_root / "benches") + rust_files(repo_root / "tests") + rust_files(repo_root / "tetron-proto" / "src")
    consumer_files = [f for p in present for f in rust_files(p)]
    search_files = own_files + consumer_files

    items = enumerate_pub_items(repo_root)
    candidates = []
    for kind, name, def_file, def_line in items:
        count = count_usages(name, search_files)
        # Subtract the definition's own occurrence(s) on its declaration line.
        def_line_hits = len(re.findall(rf"\b{re.escape(name)}\b", def_file.read_text().splitlines()[def_line - 1]))
        if count - def_line_hits <= 0:
            candidates.append((kind, name, def_file, def_line))

    print(f"Scanned {len(items)} top-level pub items across this repo + {len(present)}/{len(consumer_paths)} consumer repos.")
    if missing:
        print(f"INCONCLUSIVE for items only reachable from a missing consumer -- {len(missing)} consumer repo(s) not checked out locally.")
    print()
    if not candidates:
        print("No dead-code candidates found (given the consumer repos actually scanned).")
        return 0

    print(f"{len(candidates)} candidate(s) -- zero hits outside their own definition line, across everything scanned.")
    print("NOT confirmed dead: still exclude trait impls / derive-only fields / documented")
    print("compat scaffolding by hand, then get provenance via `git log --follow -S<symbol> -- <file>`")
    print("before treating any of these as a real finding (docs/tetron-workflow.md #12 steps 3-4).\n")
    for kind, name, def_file, def_line in candidates:
        print(f"  {kind:6} {name:40} {def_file}:{def_line}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
