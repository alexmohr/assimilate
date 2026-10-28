#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Builds the project website into website_html/: the static marketing pages
# from website/ at the root, and the MkDocs documentation under docs/.
# GitHub Pages publishes exactly this tree (see .github/workflows/pages.yml).
#
#   scripts/build-website.sh            # build only
#   scripts/build-website.sh --serve    # build, then serve on :8000
#
# Images under docs/assets/screenshots and website/assets/shots come from the
# capture job; missing ones get untracked placeholders (never commit them).
#
# Uses mkdocs from PATH when it is already installed (as in CI), otherwise uv,
# otherwise a throwaway virtualenv.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT=website_html
rm -rf "$OUT"
# Pages may show an image the capture job on main has not taken yet (it is
# committed after the merge); a one-pixel stand-in keeps the strict build
# from failing on it. Images nobody captures at all are still an error.
python3 scripts/screenshot_fingerprint.py --placeholders
python3 scripts/render_website.py website "$OUT"

if command -v mkdocs >/dev/null 2>&1; then
    MKDOCS=(mkdocs)
elif command -v uv >/dev/null 2>&1; then
    MKDOCS=(uv run --no-project --with-requirements docs/requirements.txt mkdocs)
else
    python3 -m venv /tmp/mkdocs-venv
    /tmp/mkdocs-venv/bin/pip install -q -r docs/requirements.txt
    MKDOCS=(/tmp/mkdocs-venv/bin/mkdocs)
fi
"${MKDOCS[@]}" build --strict --site-dir "$OUT/docs"

echo "Website built in $OUT/"

if [ "${1:-}" = "--serve" ]; then
    echo "Serving on http://localhost:8000/"
    python3 -m http.server 8000 --directory "$OUT"
fi
