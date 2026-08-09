#!/usr/bin/env python3
# pr-body.py -- generates a ready-to-open GitHub PR-creation link, with the
# body (and title, on a single-commit branch) already filled in from this
# branch's own commits. No `gh`, no copy-paste: GitHub's compare page
# accepts `title`/`body` query params and pre-fills the "Open a pull
# request" form when the page loads (`quick_pull=1` skips straight to it).
# This workflow uses the web UI; this script only ever prints text/a URL,
# never invokes `gh` or makes a network call itself.
#
# Usage (from ~/code/tetron, on the feature branch, base defaults to main):
#   contrib/pr-body.py               # prints a URL -- open it, review, click "Create pull request"
#   contrib/pr-body.py --base upstream/main
#   contrib/pr-body.py --text        # print the raw markdown instead, for manual paste
#
# What goes into the body, and how much to trust it:
#   - "What this does": every commit's subject as a bullet, with its body
#     (if any) indented underneath -- this is where "why" content already
#     lives, so there's no separate Why heading duplicating the same source.
#     Accurate by construction; not necessarily well-phrased for a reader --
#     the PR form is still editable before you click "Create," this is a
#     draft, not a submission.
#   - "Manual verification beyond CI": scans every commit subject+body for
#     phrases that mark real live/hardware/testsuite verification (this
#     repo's own convention: "live-verified", "verified against", etc.) and
#     quotes the matching lines. If none of the branch's commits mention
#     any, states plainly that nothing beyond CI was done -- always a
#     definite answer, never a blank you have to go investigate.
#
# URL length: GitHub/browsers can choke on very long URLs. Past
# MAX_URL_LEN this falls back to printing the markdown for manual paste
# instead of handing you a link that silently truncates.
import argparse
import re
import subprocess
import sys
import urllib.parse

VERIFICATION_MARKERS = re.compile(
    r"live[- ]verif\w*|verified against|confirmed (?:live|working)|tested on real|real hardware|real device",
    re.IGNORECASE,
)
MAX_URL_LEN = 7000


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True)


def owner_repo() -> tuple[str, str] | None:
    r = run(["git", "config", "--get", "remote.origin.url"])
    url = r.stdout.strip()
    m = re.search(r"github\.com[:/]([^/]+)/([^/.]+?)(?:\.git)?$", url)
    return (m.group(1), m.group(2)) if m else None


def build_body(base: str) -> tuple[str, str] | None:
    """Returns (body, single_commit_subject_or_empty)."""
    r = run(["git", "log", "--reverse", f"--format=%s%n%b\x1e", f"{base}..HEAD"])
    if r.returncode != 0 or not r.stdout.strip():
        print(f"[error] no commits found on {base}..HEAD -- wrong base, or nothing to describe?", file=sys.stderr)
        return None

    entries = [e for e in r.stdout.split("\x1e") if e.strip()]
    what_lines = []
    verification_hits: list[str] = []
    subjects = []
    for entry in entries:
        lines = entry.strip("\n").splitlines()
        if not lines:
            continue
        subject, body_lines = lines[0], lines[1:]
        subjects.append(subject)
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
    body = f"""## What this does

{what}

## Manual verification beyond CI

{verification}"""
    single_subject = subjects[0] if len(subjects) == 1 else ""
    return body, single_subject


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default="main", help="base branch/ref to diff against (default: main)")
    ap.add_argument("--text", action="store_true", help="print the raw markdown body instead of a URL")
    args = ap.parse_args()

    result = build_body(args.base)
    if result is None:
        return 1
    body, single_subject = result

    if args.text:
        print(body)
        return 0

    branch = run(["git", "branch", "--show-current"]).stdout.strip()
    if not branch:
        print("[error] detached HEAD -- checkout the feature branch first", file=sys.stderr)
        return 1

    or_ = owner_repo()
    if or_ is None:
        print("[warn] couldn't parse owner/repo from `git remote get-url origin` -- printing markdown instead", file=sys.stderr)
        print(body)
        return 0
    owner, repo = or_

    params = {"quick_pull": "1", "body": body}
    if single_subject:
        params["title"] = single_subject
    query = urllib.parse.urlencode(params, quote_via=urllib.parse.quote)
    url = f"https://github.com/{owner}/{repo}/compare/{args.base}...{branch}?{query}"

    if len(url) > MAX_URL_LEN:
        print(
            f"[warn] generated URL is {len(url)} chars, over the {MAX_URL_LEN}-char safety margin "
            "-- GitHub/browsers can silently truncate long URLs. Printing markdown for manual paste instead.",
            file=sys.stderr,
        )
        print(body)
        return 0

    print(url)
    if not single_subject:
        print(
            "[info] multiple commits on this branch -- no title pre-filled, GitHub will default to "
            "the branch name; type a real conventional-commit title yourself.",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
