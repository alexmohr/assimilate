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
# Uses uv when it is installed, otherwise a throwaway virtualenv.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT=website_html
rm -rf "$OUT"
python3 scripts/render_website.py website "$OUT"

if command -v uv >/dev/null 2>&1; then
    uv run --no-project --with-requirements docs/requirements.txt \
        mkdocs build --strict --site-dir "$OUT/docs"
else
    python3 -m venv /tmp/mkdocs-venv
    source /tmp/mkdocs-venv/bin/activate
    pip install -q -r docs/requirements.txt
    mkdocs build --strict --site-dir "$OUT/docs"
fi

echo "Website built in $OUT/"

if [ "${1:-}" = "--serve" ]; then
    echo "Serving on http://localhost:8000/"
    python3 -m http.server 8000 --directory "$OUT"
fi
