#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

set -euo pipefail
python3 -m venv /tmp/mkdocs-venv
source /tmp/mkdocs-venv/bin/activate
pip install -q -r docs/requirements.txt
mkdocs build --strict
