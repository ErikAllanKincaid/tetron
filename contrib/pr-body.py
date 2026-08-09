#!/usr/bin/env python3
# pr-body.py -- fills in .github/PULL_REQUEST_TEMPLATE.md's two sections
# from the current branch's own commits, so opening a PR doesn't mean
# retyping the same content by hand. Prints to stdout only -- paste the
# result into GitHub's web UI. This workflow does not use `gh`; nothing
# here invokes it.
#
# Usage (from ~/code/tetron, on the feature branch, base defaults to main):
#   contrib/pr-body.py               # prints the draft to stdout
#   contrib/pr-body.py --base upstream/main
#
# What this fills in, and how much to trust it:
#   - "What this does": every commit's subject as a bullet, with its body
#     (if any) indented underneath -- this is where "why" content already
#     lives, so there's no separate Why heading duplicating the same source.
#     Accurate by construction; not necessarily well-phrased for a reader --
#     edit before submitting, especially on a branch with several small
#     fixup commits.
#   - "Manual verification beyond CI": scans every commit subject+body for
#     phrases that mark real live/hardware/testsuite verification (this
#     repo's own convention: "live-verified", "verified against", etc.) and
#     quotes the matching lines. If none of the branch's commits mention
#     any, states plainly that nothing beyond CI was done -- always a
#     definite answer, never a blank the reader has to investigate.
import argparse
import re
import subprocess
import sys

VERIFICATION_MARKERS = re.compile(
    r"live[- ]verif\w*|verified against|confirmed (?:live|working)|tested on real|real hardware|real device",
    re.IGNORECASE,
)


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default="main", help="base branch/ref to diff against (default: main)")
    args = ap.parse_args()

    r = run(["git", "log", "--reverse", f"--format=%s%n%b\x1e", f"{args.base}..HEAD"])
    if r.returncode != 0 or not r.stdout.strip():
        print(f"[error] no commits found on {args.base}..HEAD -- wrong base, or nothing to describe?", file=sys.stderr)
        return 1

    entries = [e for e in r.stdout.split("\x1e") if e.strip()]
    what_lines = []
    verification_hits: list[str] = []
    for entry in entries:
        lines = entry.strip("\n").splitlines()
        if not lines:
            continue
        subject, body_lines = lines[0], lines[1:]
        what_lines.append(f"- {subject}")
        for line in body_lines:
            if line.strip():
                what_lines.append(f"  {line}")
        for line in [subject] + body_lines:
            if VERIFICATION_MARKERS.search(line) and line.strip() not in verification_hits:
                verification_hits.append(line.strip())

    what = "\n".join(what_lines)
    verification = (
        "\n".join(f"- {h}" for h in verification_hits)
        if verification_hits
        else "No live/manual verification beyond CI found in these commits -- CI-only."
    )

    print(f"""## What this does

{what}

## Manual verification beyond CI

{verification}""")
    return 0


if __name__ == "__main__":
    sys.exit(main())
