# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Tests for scripts/screenshot_fingerprint.py, using only the standard library.

python3 -m unittest discover -s scripts/tests
"""

import contextlib
import importlib.util
import io
import json
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

ROUTER = """\
import { useAuthStore } from '../stores/auth'
const routes = [
  { path: '/', component: () => import('../views/Dash.vue'), name: 'dashboard' },
  {
    path: '/repos/:id',
    component: () => import('../views/RepoDetail.vue'),
    name: 'repo-detail',
  },
  { path: '/repos/new', component: () => import('../views/RepoNew.vue'), name: 'repo-new' },
  { path: '/:pathMatch(.*)*', component: () => import('../views/NotFound.vue') },
]
"""

CAPTURE = """\
import { loginAsAdmin } from './fixtures'
async function shot(name: string): Promise<void> {}
test('dashboard', async () => {
  await shot('dash')
})
test.describe('signed in', () => {
  test.beforeEach(async ({ page }) => { await loginAsAdmin(page) })
  test('repos', async () => {
    await shot('repo')
  })
  for (const device of [{ name: 'phone' }]) {
    test(`${device.name} views`, async () => {
      await shot(`${device.name}-dash`)
    })
  }
})
"""

FIXTURES = """\
const BACKOFF_MS = 5_000
async function login(page: Page): Promise<void> {
  await page.waitForTimeout(BACKOFF_MS)
}
export async function loginAsAdmin(page: Page): Promise<void> {
  await login(page)
}
export async function unrelatedHelper(): Promise<void> {}
"""


# A script that visits two of the four routes of ROUTER, and names two images.
VISITING_CAPTURE = """\
test('views', async ({ page }) => {
  await visit(page, '/')
  await visit(page, `/repos/${id}?tab=archives`)
  await page.request.get('/api/repos')
  await shot(page, 'repo-new')
  await shotTo(page, WEBSITE_DIR, 'phone-view')
})
"""


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def make_tree(root: Path) -> None:
    write(root / "frontend/src/main.ts", "import App from './App.vue'\nimport './style.css'\n")
    write(root / "frontend/src/App.vue", "<script>import { router } from './router'</script>")
    write(root / "frontend/src/style.css", "@import 'tailwindcss';\n@import './theme.css';\n")
    write(root / "frontend/src/theme.css", ":root {}")
    write(root / "frontend/src/router/index.ts", ROUTER)
    write(root / "frontend/src/stores/auth.ts", "export {}")
    write(
        root / "frontend/src/views/Dash.vue",
        "<script>import Card from '../components/Card.vue'</script>",
    )
    write(root / "frontend/src/views/Dash.test.ts", "test('x', () => {})")
    write(
        root / "frontend/src/views/RepoDetail.vue",
        '<template><img src="../assets/repo.png"></template>',
    )
    write(root / "frontend/src/views/RepoNew.vue", "<template>new</template>")
    write(root / "frontend/src/views/NotFound.vue", "<template>404</template>")
    write(
        root / "frontend/src/components/Card.vue",
        "<script>import type { Repo } from '../types/generated/Repo'</script>",
    )
    write(root / "frontend/src/assets/repo.png", "png")
    write(root / "frontend/src/test-utils/mount.ts", "export {}")
    write(root / "frontend/src/types/generated/Repo.ts", "export type Repo = {}")
    write(root / "frontend/public/icon.png", "png")
    write(root / "frontend/public/icon.png.license", "SPDX")
    write(root / "frontend/index.html", "<html></html>")
    write(root / "frontend/playwright.screenshots.config.ts", "config")
    write(root / "frontend/e2e/docs.screenshots.ts", CAPTURE)
    write(root / "frontend/e2e/fixtures.ts", FIXTURES)
    write(root / ".devcontainer/demo/seed-demo.sh", "seed")
    write(root / "desktop/splash/index.html", "<html>starting</html>")


class ScannerTest(unittest.TestCase):
    def test_blanks_strings_comments_and_regexes_in_place(self) -> None:
        text = "a('x)', \"y}\") // z(\n/* { */ b(/[(]/i, `t ${c('}')} u`)"
        code = fp.blank_literals(text)
        assert len(code) == len(text)
        assert code == "a('  ', \"  \")      \n        b(/   /i, `  ${c(' ')}  `)"

    def test_can_blank_comments_only(self) -> None:
        text = "a('// x') // y\nb(/* z */ `/* w */`)"
        assert fp.blank_literals(text, comments_only=True) == "a('// x')     \nb(        `/* w */`)"

    def test_division_is_not_a_regex(self) -> None:
        assert fp.blank_literals("x = a / b / c") == "x = a / b / c"

    def test_finds_each_test_with_its_title(self) -> None:
        titles = [block.title for block in fp.test_blocks(CAPTURE)]
        assert titles == ["'dashboard'", "'repos'", "`${device.name} views`"]

    def test_test_source_matches_generated_titles(self) -> None:
        source = fp.test_source(CAPTURE, "phone views")
        assert source.startswith("test(`${device.name} views`")
        assert source.endswith("})")

    def test_test_source_rejects_an_unknown_title(self) -> None:
        message = ""
        try:
            fp.test_source(CAPTURE, "gone")
        except fp.FingerprintError as err:
            message = str(err)
        assert message == "no test in frontend/e2e/docs.screenshots.ts titled 'gone'"

    def test_shared_source_keeps_hooks_and_helpers_only(self) -> None:
        shared = fp.shared_source(CAPTURE)
        assert "async function shot" in shared
        assert "test.beforeEach" in shared
        assert "shot('dash')" not in shared
        assert "shot('repo')" not in shared

    def test_used_declarations_follow_local_helpers(self) -> None:
        used = fp.used_declarations(FIXTURES, fp.imported_names(CAPTURE, "./fixtures"))
        assert "function loginAsAdmin" in used
        assert "function login(" in used
        assert "BACKOFF_MS = 5_000" in used
        assert "unrelatedHelper" not in used


class PagesTest(unittest.TestCase):
    def test_maps_urls_to_the_route_view(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            cases = {
                "http://demo:8080/": "frontend/src/views/Dash.vue",
                "http://demo:8080/repos/3?tab=archives": "frontend/src/views/RepoDetail.vue",
                "http://demo:8080/repos/new": "frontend/src/views/RepoNew.vue",
                "http://demo:8080/no/such/page": "frontend/src/views/NotFound.vue",
            }
            for url, view in cases.items():
                assert fp.pages_for(root, url) == [view], url

    def test_a_static_page_stands_for_its_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            url = (root / "desktop/splash/index.html").resolve().as_uri()
            assert fp.pages_for(root, url) == ["desktop/splash"]


class InputsTest(unittest.TestCase):
    def test_shared_inputs_are_the_app_shell_without_its_views(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            assert sorted(fp.shared_inputs(root)) == [
                ".devcontainer/demo/seed-demo.sh",
                "frontend/index.html",
                "frontend/playwright.screenshots.config.ts",
                "frontend/public/icon.png",
                "frontend/src/App.vue",
                "frontend/src/main.ts",
                "frontend/src/router/index.ts",
                "frontend/src/stores/auth.ts",
                "frontend/src/style.css",
                "frontend/src/theme.css",
            ]

    def test_a_page_closure_follows_components_and_assets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            dash = fp.closure(root, ["frontend/src/views/Dash.vue"])
            assert dash == {"frontend/src/views/Dash.vue", "frontend/src/components/Card.vue"}
            repo = fp.closure(root, ["frontend/src/views/RepoDetail.vue"])
            assert repo == {"frontend/src/views/RepoDetail.vue", "frontend/src/assets/repo.png"}

    def test_real_repository_routes_resolve(self) -> None:
        views = {route.view for route in fp.routes(REPO_ROOT)}
        assert "frontend/src/views/DashboardView.vue" in views
        assert all((REPO_ROOT / view).is_file() for view in views)
        assert not [v for v in fp.shared_inputs(REPO_ROOT) if v.startswith(fp.VIEWS_DIR + "/")]


class FingerprintTest(unittest.TestCase):
    DASH = ("dashboard", ["frontend/src/views/Dash.vue"])
    REPO = ("repos", ["frontend/src/views/RepoDetail.vue"])

    def changes(self, rel: str, text: str = "changed") -> tuple[bool, bool]:
        """Whether writing `rel` changes the dashboard's and the repo's image."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            before = [fp.fingerprint(root, *image) for image in (self.DASH, self.REPO)]
            write(root / rel, text)
            after = [fp.fingerprint(root, *image) for image in (self.DASH, self.REPO)]
            return before[0] != after[0], before[1] != after[1]

    def test_is_stable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            assert fp.fingerprint(root, *self.DASH) == fp.fingerprint(root, *self.DASH)

    def test_a_view_change_reaches_only_its_images(self) -> None:
        assert self.changes("frontend/src/views/Dash.vue") == (True, False)
        assert self.changes("frontend/src/components/Card.vue") == (True, False)
        assert self.changes("frontend/src/assets/repo.png") == (False, True)
        assert self.changes("frontend/src/views/RepoNew.vue") == (False, False)

    def test_the_shell_and_the_seed_reach_every_image(self) -> None:
        for rel in (
            "frontend/src/App.vue",
            "frontend/src/theme.css",
            "frontend/index.html",
            "frontend/public/new.png",
            ".devcontainer/demo/seed-demo.sh",
        ):
            assert self.changes(rel) == (True, True), rel

    def test_a_capture_step_change_reaches_only_its_images(self) -> None:
        own = CAPTURE.replace("shot('repo')", "shot('repo-detail')")
        assert self.changes("frontend/e2e/docs.screenshots.ts", own) == (False, True)
        shared = CAPTURE.replace("async function shot", "async function shoot")
        assert self.changes("frontend/e2e/docs.screenshots.ts", shared) == (True, True)
        added = CAPTURE + "\n// A new step.\ntest('new', async () => {\n  await shot('new')\n})\n"
        assert self.changes("frontend/e2e/docs.screenshots.ts", added) == (False, False)
        reworded = CAPTURE.replace("async function shot", "// Takes one.\nasync function  shot")
        assert self.changes("frontend/e2e/docs.screenshots.ts", reworded) == (False, False)

    def test_only_the_fixtures_it_uses_count(self) -> None:
        used = FIXTURES.replace("5_000", "6_000")
        assert self.changes("frontend/e2e/fixtures.ts", used) == (True, True)
        unused = FIXTURES + "export function anotherHelper(): void {}\n"
        assert self.changes("frontend/e2e/fixtures.ts", unused) == (False, False)

    def test_ignores_tests_and_generated_types(self) -> None:
        assert self.changes("frontend/src/views/Dash.test.ts") == (False, False)
        assert self.changes("frontend/src/test-utils/mount.ts") == (False, False)
        assert self.changes("frontend/src/types/generated/Repo.ts") == (False, False)
        assert self.changes("frontend/public/icon.png.license") == (False, False)


class MainTest(unittest.TestCase):
    def run_main(self, root: Path, *args: str) -> tuple[int, str]:
        saved = fp.REPO_ROOT
        fp.REPO_ROOT = root
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
                code = fp.main(list(args))
        finally:
            fp.REPO_ROOT = saved
        return code, out.getvalue()

    def capture(self, root: Path, image: str, test: str, url: str, png: str) -> None:
        write(root / fp.CAPTURE_DIR / image, png)
        write(root / fp.CAPTURE_DIR / f"{image}.json", json.dumps({"test": test, "url": url}))

    def capture_all(self, root: Path, png: str = "png") -> None:
        self.capture(root, "docs/assets/screenshots/dash.png", "dashboard", "http://h/", png)
        self.capture(root, "docs/assets/screenshots/repo.png", "repos", "http://h/repos/1", png)

    def test_check_fails_for_an_image_without_a_record(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(root / "docs/assets/screenshots/dash.png", "png")
            code, out = self.run_main(root, "--check")
            assert code == 1
            assert "docs/assets/screenshots/dash.png: no record" in out

    def test_update_then_check_passes_until_an_input_changes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            self.capture_all(root)
            assert self.run_main(root, "--update")[0] == 0
            assert (root / "docs/assets/screenshots/dash.png").read_text() == "png"
            record = json.loads(
                (root / fp.RECORD_DIR / "docs/assets/screenshots/dash.png.json").read_text()
            )
            assert record["test"] == "dashboard"
            assert record["pages"] == ["frontend/src/views/Dash.vue"]
            assert self.run_main(root, "--check")[0] == 0

            write(root / "frontend/src/views/Dash.vue", "<template>new</template>")
            code, out = self.run_main(root, "--check")
            assert code == 1
            assert "1 screenshots are stale" in out
            assert "dash.png: inputs changed" in out

    def test_update_rewrites_only_stale_images(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            self.capture_all(root, "first")
            assert self.run_main(root, "--update")[0] == 0
            write(root / "frontend/src/views/Dash.vue", "<template>new</template>")
            # The clock moved, so every new capture differs from the last.
            self.capture_all(root, "second")
            code, out = self.run_main(root, "--update")
            assert code == 0
            assert "1 updated, 0 removed, 1 unchanged" in out
            assert (root / "docs/assets/screenshots/dash.png").read_text() == "second"
            assert (root / "docs/assets/screenshots/repo.png").read_text() == "first"
            assert self.run_main(root, "--check")[0] == 0

    def test_update_removes_images_no_longer_captured(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            self.capture_all(root)
            assert self.run_main(root, "--update")[0] == 0
            write(root / "docs/assets/screenshots/repo.png.license", "SPDX")
            (root / fp.CAPTURE_DIR / "docs/assets/screenshots/repo.png").unlink()
            code, out = self.run_main(root, "--update")
            assert code == 0
            assert "removed docs/assets/screenshots/repo.png" in out
            assert not (root / "docs/assets/screenshots/repo.png").exists()
            assert not (root / "docs/assets/screenshots/repo.png.license").exists()
            assert not (root / fp.RECORD_DIR / "docs/assets/screenshots/repo.png.json").exists()
            assert self.run_main(root, "--check")[0] == 0

    def test_update_fails_without_a_capture(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            code, out = self.run_main(root, "--update")
            assert code == 1
            assert "holds no images" in out

    def test_check_reports_a_record_whose_test_is_gone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            self.capture_all(root)
            assert self.run_main(root, "--update")[0] == 0
            write(
                root / "frontend/e2e/docs.screenshots.ts",
                CAPTURE.replace("'repos'", "'repositories'"),
            )
            code, out = self.run_main(root, "--check")
            assert code == 1
            assert "repo.png: no test in frontend/e2e/docs.screenshots.ts titled 'repos'" in out

    def test_update_adds_the_licence_sidecars_new_files_need(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            self.capture_all(root)
            assert self.run_main(root, "--update")[0] == 0
            record = root / fp.RECORD_DIR / "docs/assets/screenshots/dash.png.json"
            for path in (root / "docs/assets/screenshots/dash.png", record):
                sidecar = path.with_name(path.name + ".license")
                assert sidecar.read_text() == fp.LICENSE_SIDECAR, sidecar

    def test_coverage_names_each_view_without_a_step(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(root / fp.CAPTURE_SCRIPT, VISITING_CAPTURE)
            code, out = self.run_main(root, "--coverage")
            assert code == 1
            assert "2 views have no capture step" in out
            assert "/repos/new (frontend/src/views/RepoNew.vue)" in out
            assert "(.*)* (frontend/src/views/NotFound.vue)" in out
            assert "Dash.vue" not in out
            assert "RepoDetail.vue" not in out

    def test_coverage_passes_when_every_view_is_visited(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(
                root / fp.CAPTURE_SCRIPT,
                VISITING_CAPTURE + "await visit(page, '/repos/new')\nawait visit(page, '/gone')\n",
            )
            code, out = self.run_main(root, "--coverage")
            assert code == 0, out
            assert "all 4 views have a capture step" in out

    def test_coverage_counts_the_real_repository(self) -> None:
        missing = fp.uncaptured_routes(REPO_ROOT)
        assert missing == [], [route.path for route in missing]

    def test_placeholders_stand_in_for_images_the_capture_has_not_taken(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(root / fp.CAPTURE_SCRIPT, VISITING_CAPTURE)
            write(
                root / "docs/page.md",
                "![a](assets/screenshots/repo-new.png)\n![b](assets/screenshots/dash.png)\n"
                "```markdown\n![c](assets/screenshots/unreal.png)\n```\n",
            )
            write(root / "website/index.html", '<img src="assets/shots/phone-view.png">')
            write(root / "docs/assets/screenshots/dash.png", "real")
            code, out = self.run_main(root, "--placeholders")
            assert code == 0, out
            assert "placeholder docs/assets/screenshots/repo-new.png" in out
            assert "placeholder website/assets/shots/phone-view.png" in out
            made = (root / "docs/assets/screenshots/repo-new.png").read_bytes()
            assert made == fp.placeholder_png()
            assert made.startswith(b"\x89PNG\r\n\x1a\n")
            assert (root / "docs/assets/screenshots/dash.png").read_text() == "real"
            assert not (root / "docs/assets/screenshots/unreal.png").exists()

    def test_commented_out_steps_do_not_count(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(
                root / fp.CAPTURE_SCRIPT,
                "await visit(page, '/')\n"
                "// await visit(page, '/repos/new')\n"
                "/* await visit(page, '/gone')\n"
                "   await shot(page, 'hidden') */\n"
                "// await shot(page, 'also-hidden')\n"
                "for (const [path, name] of [['/a', 'five']]) {}\n"
                "// for (const [path, name] of [['/b', 'six']]) {}\n",
            )
            assert fp.visited_paths(root) == {"/", "/a"}
            assert fp.capture_names(root) == {"five"}

    def test_only_names_passed_to_shot_helpers_count(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(
                root / fp.CAPTURE_SCRIPT,
                "const label = 'dash'\n"
                "await visit(page, '/repos')\n"
                "await shot(page, 'one', { fullPage: true })\n"
                "await shotOf(page.locator('.a, .b').first(), 'two')\n"
                "await shotTo(page, WEBSITE_DIR, 'three')\n"
                "await shotOfTo(locator, DIR, `${device}-four`)\n"
                "for (const [path, name] of [['/a', 'five']]) { await shot(page, name) }\n",
            )
            assert fp.capture_names(root) == {"one", "two", "three", "five"}

    def test_placeholders_reject_an_image_no_step_takes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root)
            write(root / fp.CAPTURE_SCRIPT, VISITING_CAPTURE)
            write(root / "docs/page.md", "![a](assets/screenshots/typo-name.png)\n")
            code, out = self.run_main(root, "--placeholders")
            assert code == 1
            assert "docs/assets/screenshots/typo-name.png is shown but" in out
            assert not (root / "docs/assets/screenshots/typo-name.png").exists()


if __name__ == "__main__":
    unittest.main()
