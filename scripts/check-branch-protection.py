#!/usr/bin/env python3
"""Read-only smoke check verifying GitHub branch protection rules on main."""

import json
import os
import sys
import urllib.error
import urllib.request


def check_branch_protection(repo: str) -> bool:
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    url = f"https://api.github.com/repos/{repo}/branches/main"
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "branch-guard-check",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.load(resp)
    except urllib.error.URLError as e:
        print(f"ERROR: Failed to fetch branch status: {e}", file=sys.stderr)
        return False

    is_protected = data.get("protected", False)
    if not is_protected:
        print(f"WARN: {repo}:main has no branch protection enabled.", file=sys.stderr)
        return False

    print(f"PASS: {repo}:main is marked protected in repository settings.")
    return True


if __name__ == "__main__":
    repo = os.environ.get("GITHUB_REPOSITORY", "abanach-com/rust-discord-bot")
    success = check_branch_protection(repo)
    sys.exit(0 if success else 1)
