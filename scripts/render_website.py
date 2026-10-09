# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Render the static project website from website/ into an output directory.

Pages share their header, footer and comparison-table head through partials in
website/_partials/. A page pulls one in with a line of its own:

    <!-- include: header -->

Inside pages and partials, ``{{root}}`` becomes the relative path back to the
site root ("./" for the home page, "../" one level down), and
``{{current:NAME}}`` becomes ``aria-current="page"`` on the page whose first
path segment is NAME. Everything else in website/ is copied unchanged.

    python3 scripts/render_website.py website website_html
"""

import re
import shutil
import sys
from pathlib import Path

PARTIALS_DIR = "_partials"
INCLUDE = re.compile(r"^(?P<indent>[ \t]*)<!-- include: (?P<name>[a-z0-9-]+) -->[ \t]*$", re.M)
CURRENT = re.compile(r"\{\{current:(?P<name>[a-z0-9-]+)\}\}")
LICENSE_HEADER = re.compile(r"\A<!--\s*SPDX-License-Identifier:.*?-->\n", re.S)


def load_partials(src: Path) -> dict[str, str]:
    partials = {}
    for path in sorted((src / PARTIALS_DIR).glob("*.html")):
        text = LICENSE_HEADER.sub("", path.read_text(encoding="utf-8"))
        partials[path.stem] = text.rstrip("\n")
    return partials


def render_page(text: str, rel: Path, partials: dict[str, str]) -> str:
    def include(match: re.Match[str]) -> str:
        name = match.group("name")
        if name not in partials:
            raise SystemExit(f"{rel}: unknown partial '{name}'")
        indent = match.group("indent")
        return "\n".join(indent + line if line else line for line in partials[name].split("\n"))

    # Partials may include other partials; resolve until nothing is left.
    for _ in range(5):
        expanded = INCLUDE.sub(include, text)
        if expanded == text:
            break
        text = expanded
    depth = len(rel.parts) - 1
    root = "../" * depth if depth else "./"
    section = rel.parts[0] if depth else "home"
    text = text.replace("{{root}}", root)
    text = CURRENT.sub(lambda m: ' aria-current="page"' if m.group("name") == section else "", text)
    leftover = re.search(r"\{\{[^}]*\}\}|<!-- include:", text)
    if leftover:
        raise SystemExit(f"{rel}: unresolved placeholder near '{leftover.group(0)}'")
    return text


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: render_website.py SRC OUT")
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    partials = load_partials(src)
    for path in sorted(src.rglob("*")):
        rel = path.relative_to(src)
        if rel.parts[0] == PARTIALS_DIR or path.is_dir():
            continue
        target = out / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        if path.suffix == ".html":
            target.write_text(
                render_page(path.read_text(encoding="utf-8"), rel, partials), encoding="utf-8"
            )
        else:
            shutil.copy2(path, target)


if __name__ == "__main__":
    main()
