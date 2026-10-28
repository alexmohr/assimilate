# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Per-image fingerprints of everything that decides what a screenshot shows.

The images under docs/assets/screenshots/ and website/assets/shots/ are
captured from the seeded demo by frontend/e2e/docs.screenshots.ts. Comparing
fresh captures against committed ones can't work: the demo's dates and
relative times move with the clock. So freshness is tracked through the
inputs instead, one image at a time, so that two changes to different views
never touch the same files.

Every image has a record under RECORD_DIR holding the capture-script test that
takes it, the pages it shows (the route's view, or a static page), and a hash
of its inputs:

  * the app shell: the import closure of frontend/src/main.ts, without the
    views the router loads lazily, plus frontend/index.html, frontend/public,
    the capture config and the demo seed
  * the import closure of the image's own pages
  * the capture script's shared code (everything outside its test blocks), the
    image's own test block, and the fixtures it imports

A capture run leaves every image in CAPTURE_DIR with the test and URL it was
taken at. `--update` turns those into records and copies only the images whose
record changed into the repository, so an image of an untouched view keeps its
committed bytes; images the run no longer takes are removed.

Nobody commits the images by hand: after each merge the Screenshot capture job
recaptures on main and commits what changed, so two pull requests never fight
over a PNG. A pull request instead proves that the capture script can take
every view (`--coverage`) and that it leaves the generated files alone
(`--placeholders` lets its documentation build before its images exist).

    python3 scripts/screenshot_fingerprint.py --check         # exit 1 if any is stale
    python3 scripts/screenshot_fingerprint.py --update        # after a full capture
    python3 scripts/screenshot_fingerprint.py --coverage      # exit 1 if a view has no step
    python3 scripts/screenshot_fingerprint.py --placeholders  # stand-ins for missing images
"""

import argparse
import hashlib
import json
import re
import shutil
import struct
import sys
import zlib
from collections.abc import Iterable, Iterator
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlparse

REPO_ROOT = Path(__file__).resolve().parents[1]
RECORD_DIR = "frontend/e2e/screenshot-fingerprints"
CAPTURE_DIR = "frontend/test-results/screenshots"
IMAGE_DIRS = ("docs/assets/screenshots", "website/assets/shots")

APP_ENTRY = "frontend/src/main.ts"
ROUTER = "frontend/src/router/index.ts"
VIEWS_DIR = "frontend/src/views"
CAPTURE_SCRIPT = "frontend/e2e/docs.screenshots.ts"
FIXTURES = "frontend/e2e/fixtures.ts"
SHARED_DIRS = ("frontend/public",)
SHARED_FILES = (
    "frontend/index.html",
    "frontend/playwright.screenshots.config.ts",
    ".devcontainer/demo/seed-demo.sh",
)
# Test-only code, generated API types and licence sidecars never change a
# rendered pixel, so changing them must not demand a recapture.
EXCLUDED_DIRS = ("frontend/src/test-utils", "frontend/src/types/generated")
EXCLUDED_DIR_NAMES = ("__mocks__", "__tests__")
EXCLUDED_SUFFIXES = (".test.ts", ".spec.ts", ".license")

# Files whose relative references are followed; anything else is only hashed.
SOURCE_SUFFIXES = (".ts", ".js", ".mjs", ".vue", ".css")
RESOLVE_SUFFIXES = ("", ".ts", ".js", ".vue", "/index.ts")
# Any quoted relative path counts as a reference: imports, dynamic imports,
# template asset URLs and CSS url()/@import alike. Following a path that is
# only mentioned in a comment adds an input but never misses one.
RELATIVE_REF = re.compile(r"""['"`(]\s*(\.{1,2}/[^'"`)\s?#]*)""")

STALE_HELP = """\
The Screenshot capture job recaptures them on main after the merge; a pull
request does not commit images. To look at them before then, capture locally
(see skills/screenshots/SKILL.md):
    .devcontainer/start.sh --demo
    cd frontend && npm run screenshots
"""

UNCAPTURED_HELP = """\
Every view the router loads needs a step in frontend/e2e/docs.screenshots.ts
that visits it, so its image is taken (by the Screenshot capture job on main)
as soon as the view exists. Add a `visit(page, '<path>')` and a `shot(...)` for
each view listed above; the images themselves are not committed in the pull
request.
"""

LICENSE_SIDECAR = (
    "SPDX-License-Identifier: Apache-2.0\nSPDX-FileCopyrightText: 2026 Alexander Mohr\n"
)
# How the docs and the website name the generated images they show.
IMAGE_REFERENCE = re.compile(r"assets/(screenshots|shots)/([A-Za-z0-9_-]+)\.png")
REFERENCE_DIRS = {"screenshots": "docs/assets/screenshots", "shots": "website/assets/shots"}
REFERENCING_DIRS = ("docs", "website")
REFERENCING_SUFFIXES = (".md", ".html")
# Examples in a fenced block are not links, so MkDocs doesn't need their files.
FENCED_BLOCK = re.compile(r"^(```|~~~).*?^\1", re.MULTILINE | re.DOTALL)


class FingerprintError(Exception):
    """A record or capture that can't be fingerprinted as it stands."""


_REGEX_PRECEDERS = set("(,=:[!&|?{};+-*%<>~^")
_REGEX_KEYWORDS = {"return", "typeof", "case", "do", "else", "in", "of", "void", "yield"}


def blank_literals(text: str, comments_only: bool = False) -> str:
    """`text` with the insides of strings, comments and regexes blanked.

    The result has the same length, so an index into it is an index into
    `text`; quotes and template `${}` code stay, so brackets and keywords can be
    matched on it without being fooled by what a string or comment says. With
    `comments_only`, strings and regexes are kept.
    """
    out = list(text)
    n = len(text)
    # One entry per open template expression: the braces opened inside it.
    templates: list[int] = []
    last = ""  # the last code character, or word, before the current one

    def blank(start: int, end: int, comment: bool = False) -> None:
        if comments_only and not comment:
            return
        for k in range(start, end):
            if out[k] != "\n":
                out[k] = " "

    def template_text(i: int) -> int:
        """Blanks template text from `i`; returns the index after it ends."""
        start = i
        while i < n:
            if text[i] == "\\":
                i += 2
            elif text[i] == "`":
                blank(start, i)
                return i + 1
            elif text.startswith("${", i):
                blank(start, i)
                templates.append(0)
                return i + 2
            else:
                i += 1
        blank(start, n)
        return n

    i = 0
    while i < n:
        c = text[i]
        if c in " \t\r\n":
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            end = n if end < 0 else end
            blank(i, end, comment=True)
            i = end
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            end = n if end < 0 else end + 2
            blank(i, end, comment=True)
            i = end
        elif c in "'\"":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            blank(i + 1, j)
            i = j + 1
            last = c
        elif c == "`":
            i = template_text(i + 1)
            last = "`"
        elif c == "}" and templates and templates[-1] == 0:
            templates.pop()
            i = template_text(i + 1)
            last = "`"
        elif c == "/" and (last == "" or last in _REGEX_PRECEDERS or last in _REGEX_KEYWORDS):
            j, in_class = i + 1, False
            while j < n and text[j] != "\n" and (in_class or text[j] != "/"):
                if text[j] == "\\":
                    j += 1
                elif text[j] == "[":
                    in_class = True
                elif text[j] == "]":
                    in_class = False
                j += 1
            blank(i + 1, j)
            i = j + 1
            last = "/regex"
        else:
            if templates and c == "{":
                templates[-1] += 1
            elif templates and c == "}":
                templates[-1] -= 1
            word = re.match(r"[\w$]+", text[i:])
            if word:
                last = word.group()
                i += len(last)
            else:
                last = c
                i += 1
    return "".join(out)


_PAIRS = {"(": ")", "[": "]", "{": "}"}


def matching_close(code: str, open_at: int) -> int:
    """Index of the bracket closing the one at `open_at` in blanked `code`."""
    stack = []
    for k in range(open_at, len(code)):
        c = code[k]
        if c in _PAIRS:
            stack.append(_PAIRS[c])
        elif c in ")]}":
            if not stack or stack.pop() != c:
                raise FingerprintError(f"unbalanced {c!r} at offset {k}")
            if not stack:
                return k
    raise FingerprintError(f"{code[open_at]!r} at offset {open_at} is never closed")


def enclosing_open(code: str, at: int) -> int:
    """Index of the innermost bracket still open at `at` in blanked `code`."""
    depth = 0
    for k in range(at - 1, -1, -1):
        if code[k] in ")]}":
            depth += 1
        elif code[k] in _PAIRS:
            if depth == 0:
                return k
            depth -= 1
    raise FingerprintError(f"offset {at} is not inside any bracket")


@dataclass(frozen=True)
class TestBlock:
    title: str  # as written: a quoted string or a template literal
    start: int
    end: int  # one past the call's closing parenthesis

    def matches(self, title: str) -> bool:
        quote, body = self.title[0], self.title[1:-1]
        if quote != "`":
            return re.sub(r"\\(.)", r"\1", body) == title
        # A generated title such as `${device.name} views`: any text may
        # stand in for each placeholder.
        parts = re.split(r"\$\{[^}]*\}", body)
        return re.fullmatch(".+".join(map(re.escape, parts)), title) is not None


_TEST_CALL = re.compile(r"(?<![\w$.])test\s*\(\s*")


def test_blocks(text: str) -> list[TestBlock]:
    """Every `test(...)` call in a Playwright spec, with its title."""
    code = blank_literals(text)
    blocks = []
    for match in _TEST_CALL.finditer(code):
        quote = code[match.end()] if match.end() < len(code) else ""
        if quote not in "'\"`" or not quote:
            continue
        title_end = code.index(quote, match.end() + 1) + 1
        open_at = code.index("(", match.start())
        end = matching_close(code, open_at) + 1
        blocks.append(TestBlock(text[match.end() : title_end], match.start(), end))
    return blocks


def test_source(text: str, title: str) -> str:
    """The source of the one test titled `title`."""
    found = [b for b in test_blocks(text) if b.matches(title)]
    if len(found) != 1:
        what = "no test" if not found else f"{len(found)} tests"
        raise FingerprintError(f"{what} in {CAPTURE_SCRIPT} titled {title!r}")
    return text[found[0].start : found[0].end]


def shared_source(text: str) -> str:
    """The spec without its test blocks: helpers, hooks and describe blocks."""
    kept, pos = [], 0
    for block in test_blocks(text):
        kept.append(text[pos : block.start])
        pos = block.end
    kept.append(text[pos:])
    return "".join(kept)


_DECLARATION = re.compile(
    r"^(?:export\s+)?(?:async\s+)?(?:function\*?|const|let|class|interface|type)\s+([\w$]+)",
    re.MULTILINE,
)


def declarations(text: str) -> dict[str, str]:
    """Top-level declarations by name, each running to the next one."""
    code = blank_literals(text)
    found = list(_DECLARATION.finditer(code))
    ends = [m.start() for m in found[1:]] + [len(text)]
    return {m.group(1): text[m.start() : end] for m, end in zip(found, ends, strict=True)}


def imported_names(text: str, module: str) -> list[str]:
    """The names a spec imports from `module`, e.g. './fixtures'."""
    pattern = re.compile(r"import\s*\{([^}]*)\}\s*from\s*['\"]" + re.escape(module) + r"['\"]")
    names = []
    for match in pattern.finditer(text):
        for part in match.group(1).split(","):
            name = part.strip().removeprefix("type ").split(" as ")[0].strip()
            if name:
                names.append(name)
    return names


def mentions(code: str, name: str) -> bool:
    """Whether `code` uses the identifier `name`."""
    return re.search(rf"(?<![\w$]){re.escape(name)}(?![\w$])", code) is not None


def used_declarations(text: str, names: Iterable[str]) -> str:
    """The declarations of `names` and of everything in `text` they use."""
    decls = declarations(text)
    wanted = [n for n in names if n in decls]
    seen: set[str] = set()
    while wanted:
        name = wanted.pop()
        if name in seen:
            continue
        seen.add(name)
        body = blank_literals(decls[name])
        wanted.extend(other for other in decls if mentions(body, other))
    return "".join(decls[n] for n in sorted(seen))


def is_excluded(rel: str) -> bool:
    if rel.endswith(EXCLUDED_SUFFIXES):
        return True
    if any(rel == d or rel.startswith(d + "/") for d in EXCLUDED_DIRS):
        return True
    return any(part in EXCLUDED_DIR_NAMES for part in Path(rel).parts)


def files_under(root: Path, rel_dir: str) -> Iterator[str]:
    for path in sorted((root / rel_dir).rglob("*")):
        rel = path.relative_to(root).as_posix()
        if path.is_file() and not is_excluded(rel):
            yield rel


def resolve(root: Path, from_rel: str, spec: str) -> str | None:
    base = (root / from_rel).parent / spec
    for suffix in RESOLVE_SUFFIXES:
        candidate = Path(str(base) + suffix).resolve()
        if candidate.is_file() and candidate.is_relative_to(root.resolve()):
            return candidate.relative_to(root.resolve()).as_posix()
    return None


def references(root: Path, rel: str) -> Iterator[str]:
    if not rel.endswith(SOURCE_SUFFIXES):
        return
    for match in RELATIVE_REF.finditer((root / rel).read_text(encoding="utf-8")):
        target = resolve(root, rel, match.group(1))
        # The router loads every view; each image follows only its own.
        if target is None or (rel == ROUTER and target.startswith(VIEWS_DIR + "/")):
            continue
        yield target


def closure(root: Path, entries: Iterable[str]) -> set[str]:
    """`entries` and every file they reference, transitively.

    An entry that is a directory (a static page with its assets) stands for
    every file in it.
    """
    pending = []
    for entry in entries:
        if (root / entry).is_dir():
            pending.extend(files_under(root, entry))
        elif (root / entry).is_file():
            pending.append(entry)
        else:
            raise FingerprintError(f"page {entry} does not exist")
    seen: set[str] = set()
    while pending:
        rel = pending.pop()
        if rel in seen or is_excluded(rel):
            continue
        seen.add(rel)
        pending.extend(references(root, rel))
    return seen


def shared_inputs(root: Path) -> set[str]:
    files = closure(root, [APP_ENTRY])
    for directory in SHARED_DIRS:
        files.update(files_under(root, directory))
    files.update(rel for rel in SHARED_FILES if (root / rel).is_file())
    return files


def normalized(source: str) -> str:
    """`source` without comments or layout, which never change an image."""
    return " ".join(blank_literals(source, comments_only=True).split())


def fingerprint(root: Path, test: str, pages: Iterable[str]) -> str:
    """Hash of everything that decides what one image looks like."""
    script = (root / CAPTURE_SCRIPT).read_text(encoding="utf-8")
    fixtures_path = root / FIXTURES
    fixtures = fixtures_path.read_text(encoding="utf-8") if fixtures_path.is_file() else ""
    parts = {
        "capture:shared": shared_source(script),
        "capture:test": test_source(script, test),
        "capture:fixtures": used_declarations(fixtures, imported_names(script, "./fixtures")),
    }
    digest = hashlib.sha256()
    for name, text in parts.items():
        content = hashlib.sha256(normalized(text).encode()).hexdigest()
        digest.update(f"{name}\0{content}\n".encode())
    for rel in sorted(shared_inputs(root) | closure(root, pages)):
        content = hashlib.sha256((root / rel).read_bytes()).hexdigest()
        digest.update(f"{rel}\0{content}\n".encode())
    return digest.hexdigest()


@dataclass(frozen=True)
class Route:
    path: str
    view: str

    def pattern(self) -> re.Pattern[str]:
        parts = []
        for segment in self.path.strip("/").split("/"):
            if segment.startswith(":") and "(" in segment:
                parts.append(".*")
            elif segment.startswith(":"):
                parts.append("[^/]+")
            else:
                parts.append(re.escape(segment))
        return re.compile("/" + "/".join(parts))

    def rank(self) -> tuple[bool, int]:
        """Higher wins: a catch-all last, then the most static segments."""
        segments = self.path.strip("/").split("/")
        return ("(" not in self.path, sum(not s.startswith(":") for s in segments))


def routes(root: Path) -> list[Route]:
    """The router's routes with the view each one loads."""
    text = (root / ROUTER).read_text(encoding="utf-8")
    code = blank_literals(text)
    found = []
    for match in re.finditer(r"component:\s*\(\)\s*=>\s*import\(\s*'", code):
        spec = text[match.end() : code.index("'", match.end())]
        start = enclosing_open(code, match.start())
        if code[start] != "{":
            continue
        record = text[start : matching_close(code, start) + 1]
        path = re.search(r"path:\s*'([^']*)'", record)
        view = resolve(root, ROUTER, spec)
        if path and view:
            found.append(Route(path.group(1), view))
    return found


def pages_for(root: Path, url: str) -> list[str]:
    """The repository files a page at `url` is rendered from."""
    parsed = urlparse(url)
    if parsed.scheme == "file":
        path = Path(unquote(parsed.path)).resolve()
        if not path.is_relative_to(root.resolve()):
            raise FingerprintError(f"{url} is outside the repository")
        return [path.parent.relative_to(root.resolve()).as_posix()]
    matching = [r for r in routes(root) if r.pattern().fullmatch(parsed.path.rstrip("/") or "/")]
    if not matching:
        raise FingerprintError(f"no route matches {url}")
    return [max(matching, key=Route.rank).view]


def placeholder_png() -> bytes:
    """A valid one-pixel PNG, standing in for an image not captured yet."""

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    pixel = zlib.compress(b"\x00\x00\x00\x00\x00")
    return (
        b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", pixel) + chunk(b"IEND", b"")
    )


# A path the capture script visits: the argument of visit()/goto(), a `path:`
# property, or the first entry of a [path, name] pair. Matching only those keeps
# API URLs and the file paths typed into dialogs from counting as pages.
VISITED_PATH = re.compile(r"""(?:\bvisit\(\s*\w+\s*,|\bgoto\(|\bpath:|\[)\s*(['"`])(/[^'"`]*)\1""")
PLACEHOLDER_EXPR = re.compile(r"\$\{[^}]*\}")


def visited_paths(root: Path) -> set[str]:
    """The URL paths the capture script visits, ids and other `${...}` as `x`.

    Comments are left out: a commented-out step takes no image.
    """
    script = blank_literals((root / CAPTURE_SCRIPT).read_text(encoding="utf-8"), comments_only=True)
    paths = set()
    for match in VISITED_PATH.finditer(script):
        path = PLACEHOLDER_EXPR.sub("x", match.group(2))
        paths.add(re.split(r"[?#]", path)[0].rstrip("/") or "/")
    return paths


def uncaptured_routes(root: Path) -> list[Route]:
    """The routes whose view no step of the capture script visits."""
    all_routes = routes(root)
    covered = set()
    for path in visited_paths(root):
        matching = [r for r in all_routes if r.pattern().fullmatch(path)]
        if matching:
            covered.add(max(matching, key=Route.rank))
    return [r for r in all_routes if r not in covered]


def referenced_images(root: Path) -> set[str]:
    """Every generated image the docs and the website show, by repository path."""
    found = set()
    for directory in REFERENCING_DIRS:
        for rel in files_under(root, directory):
            if not rel.endswith(REFERENCING_SUFFIXES):
                continue
            text = (root / rel).read_text(encoding="utf-8", errors="replace")
            text = FENCED_BLOCK.sub("", text)
            found.update(
                f"{REFERENCE_DIRS[kind]}/{name}.png" for kind, name in IMAGE_REFERENCE.findall(text)
            )
    return found


# The helpers that take an image, with the position of the argument naming it.
SHOT_CALL = re.compile(r"\b(shot|shotOf|shotTo|shotOfTo)\s*\(")
SHOT_NAME_ARG = {"shot": 1, "shotOf": 1, "shotTo": 2, "shotOfTo": 2}
# A [path, name] pair, which a loop turns into one shot() per entry.
PATH_NAME_PAIR = re.compile(r"""\[\s*(['"`])/[^'"`]*\1\s*,\s*(['"])([A-Za-z0-9_-]+)\2\s*\]""")
PLAIN_LITERAL = re.compile(r"""\s*(['"])([A-Za-z0-9_-]+)\1\s*""")


def top_level_args(text: str, code: str, open_at: int) -> list[str]:
    """The arguments of the call opened at `open_at`, split at top-level commas.

    `code` is `text` with literals blanked, so only real commas split.
    """
    close = matching_close(code, open_at)
    args, depth, start = [], 0, open_at + 1
    for k in range(open_at + 1, close):
        if code[k] in "([{":
            depth += 1
        elif code[k] in ")]}":
            depth -= 1
        elif code[k] == "," and depth == 0:
            args.append(text[start:k])
            start = k + 1
    args.append(text[start:close])
    return args


def capture_names(root: Path) -> set[str]:
    """The image names the capture script gives to its shot helpers.

    Only a plain literal in the name position of a helper call counts, or the
    name of a [path, name] pair; a name built from `${...}` is left out, since
    the images made that way already exist.
    """
    script = blank_literals((root / CAPTURE_SCRIPT).read_text(encoding="utf-8"), comments_only=True)
    code = blank_literals(script)
    names = {match.group(3) for match in PATH_NAME_PAIR.finditer(script)}
    for call in SHOT_CALL.finditer(code):
        args = top_level_args(script, code, call.end() - 1)
        position = SHOT_NAME_ARG[call.group(1)]
        literal = PLAIN_LITERAL.fullmatch(args[position]) if position < len(args) else None
        if literal:
            names.add(literal.group(2))
    return names


def placeholders(root: Path) -> tuple[list[str], list[str]]:
    """Writes a stand-in for each referenced image that doesn't exist yet.

    Returns the images written, and the missing ones no shot helper call of the
    capture script names (a typo, or a step that was never written): those are not
    written.
    """
    names = capture_names(root)
    written, unknown = [], []
    for image in sorted(referenced_images(root)):
        if (root / image).is_file():
            continue
        if Path(image).stem in names:
            (root / image).parent.mkdir(parents=True, exist_ok=True)
            (root / image).write_bytes(placeholder_png())
            written.append(image)
        else:
            unknown.append(image)
    return written, unknown


def record_path(root: Path, image: str) -> Path:
    return root / RECORD_DIR / f"{image}.json"


def make_record(root: Path, test: str, pages: list[str]) -> dict[str, object]:
    return {"test": test, "pages": pages, "fingerprint": fingerprint(root, test, pages)}


def read_record(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def ensure_license(path: Path) -> None:
    """The REUSE sidecar a new image or record needs; pre-commit adds the same."""
    sidecar = path.with_name(path.name + ".license")
    if not sidecar.is_file():
        sidecar.write_text(LICENSE_SIDECAR, encoding="utf-8")


def write_record(path: Path, record: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    # One key per line with each value on it, the way Prettier lays it out.
    lines = [f"  {json.dumps(key)}: {json.dumps(value)}" for key, value in record.items()]
    path.write_text("{\n" + ",\n".join(lines) + "\n}\n", encoding="utf-8")


def committed_images(root: Path) -> set[str]:
    return {
        rel
        for directory in IMAGE_DIRS
        if (root / directory).is_dir()
        for rel in files_under(root, directory)
        if rel.endswith(".png")
    }


def recorded_images(root: Path) -> set[str]:
    records = root / RECORD_DIR
    if not records.is_dir():
        return set()
    return {
        path.relative_to(records).as_posix().removesuffix(".json")
        for path in records.rglob("*.png.json")
    }


def stale_images(root: Path) -> list[str]:
    """Each image whose record is missing or no longer matches, with why."""
    problems = []
    images, recorded = committed_images(root), recorded_images(root)
    problems += [f"{image}: no record" for image in sorted(images - recorded)]
    problems += [f"{image}: recorded but missing" for image in sorted(recorded - images)]
    for image in sorted(images & recorded):
        record = read_record(record_path(root, image))
        test, pages = str(record["test"]), [str(p) for p in record["pages"]]
        try:
            current = fingerprint(root, test, pages)
        except FingerprintError as err:
            problems.append(f"{image}: {err}")
            continue
        if current != record["fingerprint"]:
            problems.append(f"{image}: inputs changed")
    return problems


def update(root: Path) -> tuple[list[str], list[str], int]:
    """Records a capture run; returns the images updated, removed, and kept."""
    captures = root / CAPTURE_DIR
    captured = (
        sorted(p.relative_to(captures).as_posix() for p in captures.rglob("*.png"))
        if captures.is_dir()
        else []
    )
    if not captured:
        raise FingerprintError(f"{CAPTURE_DIR} holds no images; run the capture first")
    updated, kept = [], 0
    for image in captured:
        if not image.startswith(tuple(d + "/" for d in IMAGE_DIRS)):
            raise FingerprintError(f"{image} is outside {', '.join(IMAGE_DIRS)}")
        meta = json.loads((captures / f"{image}.json").read_text(encoding="utf-8"))
        test = str(meta["test"])
        record = make_record(root, test, pages_for(root, str(meta["url"])))
        target = record_path(root, image)
        if (root / image).is_file() and target.is_file() and read_record(target) == record:
            kept += 1
            continue
        (root / image).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(captures / image, root / image)
        write_record(target, record)
        ensure_license(root / image)
        ensure_license(target)
        updated.append(image)
    removed = sorted((committed_images(root) | recorded_images(root)) - set(captured))
    for image in removed:
        for path in (root / image, root / f"{image}.license", record_path(root, image)):
            path.unlink(missing_ok=True)
    return updated, removed, kept


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="exit 1 if any image is stale")
    mode.add_argument("--update", action="store_true", help="record a full capture run")
    mode.add_argument(
        "--coverage", action="store_true", help="exit 1 if a view has no capture step"
    )
    mode.add_argument(
        "--placeholders",
        action="store_true",
        help="write stand-ins for referenced images not captured yet (for docs builds)",
    )
    args = parser.parse_args(argv)

    if args.coverage:
        missing = uncaptured_routes(REPO_ROOT)
        if not missing:
            print(f"all {len(routes(REPO_ROOT))} views have a capture step")
            return 0
        listing = "\n".join(f"  {r.path} ({r.view})" for r in missing)
        print(
            f"{len(missing)} views have no capture step:\n{listing}\n\n{UNCAPTURED_HELP}",
            file=sys.stderr,
        )
        return 1

    if args.placeholders:
        written, unknown = placeholders(REPO_ROOT)
        for image in written:
            print(f"placeholder {image}")
        for image in unknown:
            print(
                f"error: {image} is shown but the capture script has no step named it",
                file=sys.stderr,
            )
        return 1 if unknown else 0

    if args.update:
        try:
            updated, removed, kept = update(REPO_ROOT)
        except FingerprintError as err:
            print(f"error: {err}", file=sys.stderr)
            return 1
        for image in updated:
            print(f"updated {image}")
        for image in removed:
            print(f"removed {image}")
        print(f"{len(updated)} updated, {len(removed)} removed, {kept} unchanged")
        return 0

    problems = stale_images(REPO_ROOT)
    if not problems:
        print(f"all {len(committed_images(REPO_ROOT))} screenshots are fresh")
        return 0
    listing = "\n".join(f"  {p}" for p in problems)
    print(f"{len(problems)} screenshots are stale:\n{listing}\n\n{STALE_HELP}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
