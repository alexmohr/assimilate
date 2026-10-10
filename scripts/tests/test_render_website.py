# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Tests for scripts/render_website.py, using only the standard library.

python3 -m unittest discover -s scripts/tests
"""

import importlib.util
import re
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
_SPEC = importlib.util.spec_from_file_location(
    "render_website", REPO_ROOT / "scripts" / "render_website.py"
)
assert _SPEC is not None
assert _SPEC.loader is not None
render_website = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(render_website)

LICENSE = "<!--\nSPDX-License-Identifier: Apache-2.0\n-->\n"


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def exit_message(call: Callable[[], object]) -> str:
    """Runs `call`, which must raise SystemExit, and returns its message."""
    try:
        call()
    except SystemExit as e:
        return str(e)
    raise AssertionError("expected SystemExit")


def render(text: str, rel: str, partials: dict[str, str] | None = None) -> str:
    return render_website.render_page(text, Path(rel), partials or {})


def run_main(*args: str) -> None:
    argv = sys.argv
    sys.argv = ["render_website.py", *args]
    try:
        render_website.main()
    finally:
        sys.argv = argv


class LoadPartialsTest(unittest.TestCase):
    def test_strips_license_header_and_trailing_newlines(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            write(src / "_partials" / "footer.html", LICENSE + "<footer>x</footer>\n\n")
            assert render_website.load_partials(src) == {"footer": "<footer>x</footer>"}


class RenderPageTest(unittest.TestCase):
    def test_include_keeps_the_including_line_indent(self) -> None:
        partials = {"nav": "<nav>\n  <a>Home</a>\n</nav>"}
        out = render("<body>\n    <!-- include: nav -->\n</body>", "index.html", partials)
        assert out == "<body>\n    <nav>\n      <a>Home</a>\n    </nav>\n</body>"

    def test_blank_partial_lines_stay_unindented(self) -> None:
        assert render("  <!-- include: p -->", "index.html", {"p": "<a>\n\n<b>"}) == (
            "  <a>\n\n  <b>"
        )

    def test_partials_can_include_partials(self) -> None:
        partials = {"outer": "<div>\n<!-- include: inner -->\n</div>", "inner": "<p>hi</p>"}
        assert render("<!-- include: outer -->", "index.html", partials) == (
            "<div>\n<p>hi</p>\n</div>"
        )

    def test_root_is_relative_to_page_depth(self) -> None:
        page = '<a href="{{root}}docs/">'
        assert render(page, "index.html") == '<a href="./docs/">'
        assert render(page, "compare/index.html") == '<a href="../docs/">'
        assert render(page, "a/b/index.html") == '<a href="../../docs/">'

    def test_current_marks_only_the_matching_section(self) -> None:
        page = "<a{{current:compare}}>Compare</a><a{{current:privacy}}>Privacy</a>"
        assert render(page, "compare/index.html") == (
            '<a aria-current="page">Compare</a><a>Privacy</a>'
        )
        assert render(page, "index.html") == "<a>Compare</a><a>Privacy</a>"

    def test_unknown_partial_fails(self) -> None:
        message = exit_message(lambda: render("<!-- include: missing -->", "index.html"))
        assert "unknown partial 'missing'" in message

    def test_unresolved_placeholder_fails(self) -> None:
        message = exit_message(lambda: render("{{rot}}", "index.html"))
        assert "unresolved placeholder" in message

    def test_include_cycle_fails_instead_of_looping(self) -> None:
        partials = {"a": "<!-- include: a -->"}
        message = exit_message(lambda: render("<!-- include: a -->", "index.html", partials))
        assert "unresolved placeholder" in message


class MainTest(unittest.TestCase):
    def test_renders_pages_copies_assets_and_skips_partials(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            src, out = Path(tmp) / "src", Path(tmp) / "out"
            write(src / "_partials" / "head.html", LICENSE + '<link href="{{root}}site.css">')
            write(src / "index.html", "<!-- include: head -->")
            write(src / "compare" / "index.html", "<!-- include: head -->")
            write(src / "assets" / "site.css", "body{}")
            run_main(str(src), str(out))
            assert read(out / "index.html") == '<link href="./site.css">'
            assert read(out / "compare" / "index.html") == '<link href="../site.css">'
            assert read(out / "assets" / "site.css") == "body{}"
            assert not (out / "_partials").exists()

    def test_rejects_wrong_argument_count(self) -> None:
        assert exit_message(run_main).startswith("usage")

    def test_real_website_renders(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            run_main(str(REPO_ROOT / "website"), str(out))
            pages = sorted(out.rglob("*.html"))
            assert out / "index.html" in pages
            for page in pages:
                leftover = re.search(r"\{\{|<!-- include:", read(page))
                assert leftover is None, f"{page}: unrendered {leftover}"


if __name__ == "__main__":
    unittest.main()
