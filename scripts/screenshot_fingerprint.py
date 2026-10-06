# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Fingerprint of everything that decides what the screenshots look like.

The images under docs/assets/screenshots/ and website/assets/shots/ are
captured from the seeded demo by frontend/e2e/docs.screenshots.ts. Comparing
fresh captures against committed ones can't work: the demo's dates and
relative times move with the clock. So freshness is tracked through the
inputs instead. `npm run screenshots` records a hash of the frontend sources,
the demo seed, and the capture script in FINGERPRINT_FILE after a full run,
and CI fails when the committed hash no longer matches the sources.

    python3 scripts/screenshot_fingerprint.py           # print the hash
    python3 scripts/screenshot_fingerprint.py --write   # record it
    python3 scripts/screenshot_fingerprint.py --check   # exit 1 if stale
"""

import argparse
import hashlib
import sys
from collections.abc import Iterator
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
FINGERPRINT_FILE = REPO_ROOT / "frontend" / "e2e" / "screenshots.fingerprint"

# Directories whose files shape the screenshots, and single files that do.
INPUT_DIRS = ("frontend/src", "frontend/public")
INPUT_FILES = (
    "frontend/index.html",
    "frontend/e2e/docs.screenshots.ts",
    "frontend/e2e/fixtures.ts",
    "frontend/playwright.screenshots.config.ts",
    ".devcontainer/demo/seed-demo.sh",
)
# Test-only code, generated API types and licence sidecars never change a
# rendered pixel, so changing them must not demand a recapture.
EXCLUDED_DIRS = ("frontend/src/test-utils", "frontend/src/types/generated")
EXCLUDED_DIR_NAMES = ("__mocks__", "__tests__")
EXCLUDED_SUFFIXES = (".test.ts", ".spec.ts", ".license")

STALE_HELP = """\
The screenshots are stale: frontend sources, the demo seed, or the capture
script changed since they were last captured.

Refresh them in one of two ways, then commit the images together with
frontend/e2e/screenshots.fingerprint:

  * Download the "screenshots" artifact from this PR's Playwright E2E job and
    unpack it at the repository root. It holds fresh captures of this commit.
  * Or capture locally (see skills/screenshots/SKILL.md):
        .devcontainer/start.sh --demo
        cd frontend && npm run screenshots
"""


def is_excluded(rel: str) -> bool:
    if rel.endswith(EXCLUDED_SUFFIXES):
        return True
    if any(rel == d or rel.startswith(d + "/") for d in EXCLUDED_DIRS):
        return True
    return any(part in EXCLUDED_DIR_NAMES for part in Path(rel).parts)


def input_files(root: Path) -> Iterator[str]:
    for directory in INPUT_DIRS:
        for path in sorted((root / directory).rglob("*")):
            rel = path.relative_to(root).as_posix()
            if path.is_file() and not is_excluded(rel):
                yield rel
    for rel in INPUT_FILES:
        if (root / rel).is_file():
            yield rel


def fingerprint(root: Path | None = None) -> str:
    root = REPO_ROOT if root is None else root
    digest = hashlib.sha256()
    for rel in sorted(set(input_files(root))):
        content = hashlib.sha256((root / rel).read_bytes()).hexdigest()
        digest.update(f"{rel}\0{content}\n".encode())
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true", help="record the current fingerprint")
    mode.add_argument("--check", action="store_true", help="exit 1 if the recorded one is stale")
    args = parser.parse_args(argv)

    current = fingerprint()
    if args.write:
        FINGERPRINT_FILE.write_text(current + "\n", encoding="utf-8")
        print(f"recorded {current} in {FINGERPRINT_FILE.relative_to(REPO_ROOT)}")
        return 0
    if args.check:
        recorded = (
            FINGERPRINT_FILE.read_text(encoding="utf-8").strip()
            if FINGERPRINT_FILE.is_file()
            else "(none)"
        )
        if recorded == current:
            print(f"screenshots are fresh ({current})")
            return 0
        print(f"recorded: {recorded}\ncurrent:  {current}\n\n{STALE_HELP}", file=sys.stderr)
        return 1
    print(current)
    return 0


if __name__ == "__main__":
    sys.exit(main())
