#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# /// script
# dependencies = []
# ///

"""Reject typographic / Unicode punctuation inside source-code comments.

Flags em dashes, en dashes, ellipsis characters, and curly quotes when they
appear in comment lines.  String literals and template text are intentionally
not checked — only stripped comment content is tested.
"""

import argparse
import re
import sys

COMMENT_PREFIXES = ("///", "//!", "//", "/*", "*/", "#!", "#", "*")

TYPOGRAPHY = {
    "—": "em dash (—) — use -",
    "–": "en dash (–) — use -",
    "―": "horizontal bar (―) — use -",
    "…": "ellipsis (…) — use ...",
    "“": "left double quote (“) — use \"",
    "”": "right double quote (”) — use \"",
    "‘": "left single quote (‘) — use '",
    "’": "right single quote (’) — use '",
}

PATTERN = re.compile("[" + "".join(re.escape(c) for c in TYPOGRAPHY) + "]")


def strip_comment_content(line: str) -> str | None:
    stripped = line.lstrip()
    for prefix in COMMENT_PREFIXES:
        if stripped.startswith(prefix):
            return stripped[len(prefix) :].strip()
    return None


def check_file(path: str) -> list[tuple[int, str, str]]:
    violations: list[tuple[int, str, str]] = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for lineno, line in enumerate(f, start=1):
                content = strip_comment_content(line)
                if not content:
                    continue
                for match in PATTERN.finditer(content):
                    char = match.group()
                    violations.append((lineno, line.rstrip(), TYPOGRAPHY[char]))
    except OSError as exc:
        print(f"Error reading {path}: {exc}", file=sys.stderr)
        sys.exit(1)
    return violations


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Reject typographic punctuation in source-code comments.",
    )
    parser.add_argument("files", nargs="*")
    args = parser.parse_args()

    if not args.files:
        sys.exit(0)

    found = False
    for path in args.files:
        for lineno, text, description in check_file(path):
            print(f"{path}:{lineno}: typography in comment ({description}): {text}")
            found = True

    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
