# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Tests for scripts/screenshot_fingerprint.py, using only the standard library.

python3 -m unittest discover -s scripts/tests
"""

import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
_SPEC = importlib.util.spec_from_file_location(
    "screenshot_fingerprint", REPO_ROOT / "scripts" / "screenshot_fingerprint.py"
)
assert _SPEC is not None
assert _SPEC.loader is not None
fp = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(fp)


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def make_tree(root: Path) -> None:
    write(root / "frontend/src/App.vue", "<template>app</template>")
    write(root / "frontend/src/views/Dash.vue", "<template>dash</template>")
    write(root / "frontend/src/views/Dash.test.ts", "test('x', () => {})")
    write(root / "frontend/src/test-utils/mount.ts", "export {}")
    write(root / "frontend/src/api/__mocks__/client.ts", "export {}")
    write(root / "frontend/src/types/generated/Repo.ts", "export type Repo = {}")
    write(root / "frontend/public/icon.png", "png")
    write(root / "frontend/public/icon.png.license", "SPDX")
    write(root / "frontend/index.html", "<html></html>")
    write(root / "frontend/e2e/docs.screenshots.ts", "shots")
    write(root / ".devcontainer/demo/seed-demo.sh", "seed")


class InputFilesTest(unittest.TestCase):
    def test_lists_rendering_inputs_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            assert sorted(fp.input_files(root)) == [
                ".devcontainer/demo/seed-demo.sh",
                "frontend/e2e/docs.screenshots.ts",
                "frontend/index.html",
                "frontend/public/icon.png",
                "frontend/src/App.vue",
                "frontend/src/views/Dash.vue",
            ]

    def test_real_repository_has_inputs(self) -> None:
        files = list(fp.input_files(REPO_ROOT))
        assert ".devcontainer/demo/seed-demo.sh" in files
        assert "frontend/e2e/docs.screenshots.ts" in files
        assert not [f for f in files if f.endswith(".test.ts")]


class FingerprintTest(unittest.TestCase):
    def changed_by(self, rel: str) -> bool:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            before = fp.fingerprint(root)
            write(root / rel, "changed")
            return fp.fingerprint(root) != before

    def test_is_stable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            assert fp.fingerprint(root) == fp.fingerprint(root)

    def test_changes_with_rendering_inputs(self) -> None:
        assert self.changed_by("frontend/src/views/Dash.vue")
        assert self.changed_by("frontend/src/views/New.vue")
        assert self.changed_by(".devcontainer/demo/seed-demo.sh")
        assert self.changed_by("frontend/e2e/docs.screenshots.ts")

    def test_ignores_tests_and_generated_types(self) -> None:
        assert not self.changed_by("frontend/src/views/Dash.test.ts")
        assert not self.changed_by("frontend/src/test-utils/mount.ts")
        assert not self.changed_by("frontend/src/types/generated/Repo.ts")
        assert not self.changed_by("frontend/public/icon.png.license")

    def test_renaming_a_file_changes_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            before = fp.fingerprint(root)
            (root / "frontend/src/App.vue").rename(root / "frontend/src/Main.vue")
            assert fp.fingerprint(root) != before


class MainTest(unittest.TestCase):
    def run_main(self, root: Path, *args: str) -> tuple[int, str]:
        saved = fp.REPO_ROOT, fp.FINGERPRINT_FILE
        fp.REPO_ROOT = root
        fp.FINGERPRINT_FILE = root / "frontend/e2e/screenshots.fingerprint"
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
                code = fp.main(list(args))
        finally:
            fp.REPO_ROOT, fp.FINGERPRINT_FILE = saved
        return code, out.getvalue()

    def test_check_fails_without_a_recorded_fingerprint(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            code, out = self.run_main(root, "--check")
            assert code == 1
            assert "recorded: (none)" in out

    def test_write_then_check_passes_until_an_input_changes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            assert self.run_main(root, "--write")[0] == 0
            assert self.run_main(root, "--check")[0] == 0
            write(root / "frontend/src/App.vue", "<template>new</template>")
            code, out = self.run_main(root, "--check")
            assert code == 1
            assert "screenshots are stale" in out

    def test_prints_the_fingerprint_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            code, out = self.run_main(root)
            assert code == 0
            assert out.strip() == fp.fingerprint(root)


if __name__ == "__main__":
    unittest.main()
