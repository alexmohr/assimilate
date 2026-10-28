# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Tests for scripts/desktop_bundle.py, using only the standard library.

python3 -m unittest discover -s scripts/tests
"""

import contextlib
import dataclasses
import hashlib
import importlib.util
import io
import re
import tarfile
import tempfile
import unittest
import zipfile
from collections.abc import Iterator
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
_SPEC = importlib.util.spec_from_file_location(
    "desktop_bundle", REPO_ROOT / "scripts" / "desktop_bundle.py"
)
assert _SPEC is not None
assert _SPEC.loader is not None
db = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(db)

TARGET = "x86_64-unknown-linux-gnu"


@contextlib.contextmanager
def expect_error(error: type[BaseException], pattern: str = "") -> Iterator[None]:
    """Fails unless the block raises `error` with a message matching `pattern`."""
    raised: BaseException | None = None
    try:
        yield
    except error as caught:
        raised = caught
    if raised is None:
        raise AssertionError(f"{error.__name__} not raised")
    assert re.search(pattern, str(raised)), f"{raised!s} doesn't match {pattern!r}"


def make_archive(path: Path, files: dict[str, bytes], links: dict[str, str] | None = None) -> str:
    """Writes a .tar.gz of `files` (name -> content) and `links` (symlink name
    -> target) and returns its SHA-256."""
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = 0o644
            tar.addfile(info, io.BytesIO(content))
        for name, target in (links or {}).items():
            info = tarfile.TarInfo(name)
            info.type = tarfile.SYMTYPE
            info.linkname = target
            tar.addfile(info)
    return hashlib.sha256(path.read_bytes()).hexdigest()


def make_zip(path: Path, files: dict[str, bytes]) -> str:
    """Writes a .zip of `files` (name -> content, names as given, backslashes
    included) and returns its SHA-256."""
    with zipfile.ZipFile(path, "w") as zipped:
        for name, content in files.items():
            zipped.writestr(zipfile.ZipInfo(name), content)
    return hashlib.sha256(path.read_bytes()).hexdigest()


WINDOWS = "x86_64-pc-windows-msvc"


def manifest(pg_sha: str = "a" * 64, borg_sha: str = "b" * 64) -> dict:
    return {
        "postgresql": {
            "version": "18.6.0",
            "attested": False,
            "url": "https://github.com/theseus-rs/postgresql-binaries/releases/download/{version}/postgresql-{version}-{target}.tar.gz",
            "sha256": {TARGET: pg_sha},
        },
        "borg": {
            "version": "1.4.4",
            "attested": True,
            "url": "https://github.com/borgbackup/borg/releases/download/{version}/{asset}",
            "assets": {TARGET: {"name": "borg-linux-glibc235-x86_64-gh.tgz", "sha256": borg_sha}},
        },
    }


class DownloadsForTest(unittest.TestCase):
    def test_builds_the_pinned_urls_for_a_target(self) -> None:
        downloads = db.downloads_for(manifest(), TARGET)

        assert (
            downloads.postgresql.url
            == "https://github.com/theseus-rs/postgresql-binaries/releases/download/18.6.0/postgresql-18.6.0-x86_64-unknown-linux-gnu.tar.gz"
        )
        assert downloads.postgresql.name == "postgresql-18.6.0-x86_64-unknown-linux-gnu.tar.gz"
        assert downloads.postgresql.sha256 == "a" * 64
        assert (
            downloads.borg.url
            == "https://github.com/borgbackup/borg/releases/download/1.4.4/borg-linux-glibc235-x86_64-gh.tgz"
        )
        assert downloads.borg.sha256 == "b" * 64
        assert not downloads.postgresql.attested
        assert downloads.borg.attested
        assert downloads.notices == ()
        assert downloads.postgresql.github_repo == "theseus-rs/postgresql-binaries"
        assert downloads.borg.github_repo == "borgbackup/borg"

    def test_an_asset_may_name_its_own_url_and_attestation(self) -> None:
        pinned = manifest()
        pinned["postgresql"]["sha256"][WINDOWS] = "c" * 64
        pinned["borg"]["assets"][WINDOWS] = {
            "name": "borg-windows-1.4.5-win1.zip",
            "url": "https://github.com/marcpope/borg-windows/releases/download/v1.4.5-win1/borg-windows.zip",
            "sha256": "d" * 64,
            "attested": False,
        }

        windows = db.downloads_for(pinned, WINDOWS).borg
        linux = db.downloads_for(pinned, TARGET).borg

        assert windows.name == "borg-windows-1.4.5-win1.zip"
        assert windows.github_repo == "marcpope/borg-windows"
        assert not windows.attested
        assert linux.github_repo == "borgbackup/borg"
        assert linux.attested

    def test_only_windows_takes_borg_from_outside_borgbackup(self) -> None:
        pinned = db.load_manifest(db.MANIFEST)
        repos = {
            target: db.downloads_for(pinned, target).borg.github_repo
            for target in pinned["borg"]["assets"]
        }

        assert {target for target, repo in repos.items() if repo != "borgbackup/borg"} == {WINDOWS}

    def test_an_unknown_target_names_the_known_ones(self) -> None:
        with expect_error(db.BundleError, f"no pinned downloads for riscv64.*{TARGET}"):
            db.downloads_for(manifest(), "riscv64")

    def test_only_targets_listed_for_libxml2_get_its_source(self) -> None:
        pinned = manifest()
        pinned["postgresql"]["sha256"][WINDOWS] = "c" * 64
        pinned["borg"]["assets"][WINDOWS] = {"name": "borg.zip", "sha256": "d" * 64}
        pinned["libxml2"] = {
            "version": "2.13.9",
            "url": "https://download.gnome.org/sources/libxml2/2.13/libxml2-{version}.tar.xz",
            "sha256": "e" * 64,
            "targets": [TARGET],
        }

        linux = db.downloads_for(pinned, TARGET).libxml2
        assert linux is not None
        assert linux.name == "libxml2-2.13.9.tar.xz"
        assert linux.url == "https://download.gnome.org/sources/libxml2/2.13/libxml2-2.13.9.tar.xz"
        assert linux.sha256 == "e" * 64
        assert not linux.attested
        assert db.downloads_for(pinned, WINDOWS).libxml2 is None

    def test_the_checked_in_manifest_builds_libxml2_for_linux_only(self) -> None:
        pinned = db.load_manifest(db.MANIFEST)
        with_libxml2 = {
            target
            for target in pinned["postgresql"]["sha256"]
            if db.downloads_for(pinned, target).libxml2 is not None
        }

        assert with_libxml2 == {"x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"}
        libxml2 = db.downloads_for(pinned, TARGET).libxml2
        assert libxml2 is not None
        assert re.search("^[0-9a-f]{64}$", libxml2.sha256)

    def test_a_non_github_url_has_no_repository(self) -> None:
        download = db.Download("x.tgz", "https://example.com/x.tgz", "0" * 64)
        with expect_error(db.BundleError, "not a GitHub release download"):
            _ = download.github_repo

    def test_the_checked_in_manifest_covers_every_built_target(self) -> None:
        pinned = db.load_manifest(db.MANIFEST)
        for target in (
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            WINDOWS,
        ):
            downloads = db.downloads_for(pinned, target)
            assert [(notice.name, notice.component) for notice in downloads.notices] == [
                ("postgresql.txt", db.Component.POSTGRESQL),
                ("borgbackup.txt", db.Component.BORG),
            ]
            for download in (downloads.postgresql, downloads.borg, *downloads.notices):
                assert re.search("^[0-9a-f]{64}$", download.sha256), download.name


class FetchTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.source = self.root / "source.tgz"
        self.sha = make_archive(self.source, {"top/file": b"content"})
        self.cache = self.root / "cache"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def download(self, sha: str) -> object:
        return db.Download("pinned.tgz", self.source.as_uri(), sha)

    def test_downloads_a_file_with_the_pinned_digest(self) -> None:
        path = db.fetch(self.download(self.sha), self.cache)

        assert path == self.cache / "pinned.tgz"
        assert path.read_bytes() == self.source.read_bytes()

    def test_refuses_a_file_with_another_digest_and_keeps_nothing(self) -> None:
        with expect_error(
            db.BundleError, f"^pinned.tgz: SHA-256 is {self.sha}, but bundle.toml pins 0+$"
        ):
            db.fetch(self.download("0" * 64), self.cache)

        assert list(self.cache.iterdir()) == []

    def test_reuses_a_verified_copy_in_the_cache(self) -> None:
        db.fetch(self.download(self.sha), self.cache)
        self.source.unlink()

        path = db.fetch(self.download(self.sha), self.cache)

        assert path.is_file()

    def test_replaces_a_cached_copy_that_no_longer_matches(self) -> None:
        self.cache.mkdir()
        (self.cache / "pinned.tgz").write_bytes(b"tampered")

        path = db.fetch(self.download(self.sha), self.cache)

        assert path.read_bytes() == self.source.read_bytes()


class ExtractTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.archive = self.root / "a.tgz"
        self.dest = self.root / "out"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_drops_the_top_level_directory_and_keeps_links(self) -> None:
        make_archive(
            self.archive,
            {"./pg-18/bin/initdb": b"initdb", "./pg-18/lib/libpq.so.5": b"lib"},
            {"./pg-18/lib/libpq.so": "libpq.so.5"},
        )

        db.extract_without_top_dir(self.archive, self.dest)

        assert (self.dest / "bin" / "initdb").read_bytes() == b"initdb"
        assert (self.dest / "lib" / "libpq.so").is_symlink()
        assert (self.dest / "lib" / "libpq.so").read_bytes() == b"lib"

    def test_refuses_an_archive_without_a_single_top_directory(self) -> None:
        make_archive(self.archive, {"a/file": b"", "b/file": b""})

        with expect_error(db.BundleError, r"one top-level directory, found \['a', 'b'\]"):
            db.extract_without_top_dir(self.archive, self.dest)

    def test_refuses_a_link_leading_out_of_the_destination(self) -> None:
        make_archive(self.archive, {"top/file": b""}, {"top/escape": "../../outside"})

        with expect_error(tarfile.FilterError):
            db.extract_without_top_dir(self.archive, self.dest)


class ExtractZipTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.archive = self.root / "a.zip"
        self.dest = self.root / "out"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_keeps_the_layout_and_reads_backslash_paths(self) -> None:
        make_zip(
            self.archive,
            {"borg.exe": b"borg", "_internal\\": b"", "_internal\\python3.dll": b"py"},
        )

        db.extract(self.archive, self.dest)

        assert (self.dest / "borg.exe").read_bytes() == b"borg"
        assert (self.dest / "_internal" / "python3.dll").read_bytes() == b"py"

    def test_refuses_paths_leading_out_of_the_destination(self) -> None:
        for name in ("..\\evil.dll", "../evil.dll", "C:\\evil.dll", "/evil.dll"):
            with self.subTest(name=name):
                make_zip(self.archive, {name: b""})
                with expect_error(db.BundleError, "unsafe path in archive"):
                    db.extract(self.archive, self.dest)
        assert not (self.root / "evil.dll").exists()


class StageTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.desktop = self.root / "desktop"
        self.server = self.root / "server"
        self.server.write_bytes(b"server")
        self.agent = self.root / "agent"
        self.agent.write_bytes(b"agent")
        self.web = self.root / "dist"
        (self.web / "assets").mkdir(parents=True)
        (self.web / "index.html").write_text("<html></html>")
        self.docs = self.root / "docs_html"
        self.docs.mkdir()
        (self.docs / "index.html").write_text("docs")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def downloads(self, pg_files: dict[str, bytes], borg_files: dict[str, bytes]) -> object:
        pg = self.root / "pg.tgz"
        borg = self.root / "borg.tgz"
        notice = self.root / "COPYRIGHT"
        notice.write_text("license text")
        return db.TargetDownloads(
            postgresql=db.Download("pg.tgz", pg.as_uri(), make_archive(pg, pg_files)),
            borg=db.Download("borg.tgz", borg.as_uri(), make_archive(borg, borg_files)),
            notices=(
                db.Download(
                    "postgresql.txt", notice.as_uri(), hashlib.sha256(b"license text").hexdigest()
                ),
            ),
        )

    def stage(self, downloads: object, docs: Path | None) -> None:
        db.stage(
            target=TARGET,
            downloads=downloads,
            server=self.server,
            agent=self.agent,
            web=self.web,
            docs=docs,
            cache=self.root / "cache",
            desktop_dir=self.desktop,
        )

    def test_lays_out_everything_the_installer_ships(self) -> None:
        stale = self.desktop / "resources" / "web" / "stale.js"
        stale.parent.mkdir(parents=True)
        stale.write_text("old")

        self.stage(
            self.downloads(
                {"pg/bin/initdb": b"initdb", "pg/share/postgres.bki": b""},
                {"borg-dir/borg.exe": b"borg", "borg-dir/_internal/lib.so": b""},
            ),
            self.docs,
        )

        resources = self.desktop / "resources"
        assert (resources / "postgres" / "bin" / "initdb").is_file()
        assert (resources / "postgres" / "share" / "postgres.bki").is_file()
        borg = resources / "borg" / "borg.exe"
        assert borg.read_bytes() == b"borg"
        assert borg.stat().st_mode & 0o777 == 0o755
        assert (resources / "borg" / "_internal" / "lib.so").is_file()
        assert (resources / "licenses" / "postgresql.txt").read_text() == "license text"
        assert (resources / "web" / "index.html").is_file()
        assert not stale.exists()
        assert (resources / "docs" / "index.html").read_text() == "docs"
        server = self.desktop / "binaries" / f"assimilate-server-{TARGET}"
        assert server.read_bytes() == b"server"
        assert server.stat().st_mode & 0o777 == 0o755
        assert (self.desktop / "binaries" / f"assimilate-agent-{TARGET}").read_bytes() == b"agent"

    def test_windows_names_executables_with_exe_and_takes_a_flat_borg_zip(self) -> None:
        pg = self.root / "pg.tgz"
        borg = self.root / "borg.zip"
        downloads = db.TargetDownloads(
            postgresql=db.Download(
                "pg.tgz", pg.as_uri(), make_archive(pg, {"pg/bin/initdb.exe": b""})
            ),
            borg=db.Download(
                "borg.zip",
                borg.as_uri(),
                make_zip(borg, {"borg.exe": b"borg", "_internal\\base_library.zip": b""}),
            ),
        )

        db.stage(
            target=WINDOWS,
            downloads=downloads,
            server=self.server,
            agent=self.agent,
            web=self.web,
            docs=None,
            cache=self.root / "cache",
            desktop_dir=self.desktop,
        )

        resources = self.desktop / "resources"
        assert (resources / "borg" / "borg.exe").read_bytes() == b"borg"
        assert (resources / "borg" / "_internal" / "base_library.zip").is_file()
        binaries = self.desktop / "binaries"
        assert (binaries / f"assimilate-server-{WINDOWS}.exe").read_bytes() == b"server"
        assert (binaries / f"assimilate-agent-{WINDOWS}.exe").read_bytes() == b"agent"

    def test_a_windows_postgresql_needs_initdb_exe(self) -> None:
        pg = self.root / "pg.tgz"
        borg = self.root / "borg.zip"
        downloads = db.TargetDownloads(
            postgresql=db.Download("pg.tgz", pg.as_uri(), make_archive(pg, {"pg/bin/initdb": b""})),
            borg=db.Download("borg.zip", borg.as_uri(), make_zip(borg, {"borg.exe": b""})),
        )
        with expect_error(db.BundleError, "pg.tgz has no bin/initdb.exe"):
            db.stage(
                target=WINDOWS,
                downloads=downloads,
                server=self.server,
                agent=self.agent,
                web=self.web,
                docs=None,
                cache=self.root / "cache",
                desktop_dir=self.desktop,
            )

    def test_docs_are_optional(self) -> None:
        self.stage(self.downloads({"pg/bin/initdb": b""}, {"borg-dir/borg.exe": b""}), None)

        assert not (self.desktop / "resources" / "docs").exists()

    def test_refuses_a_postgresql_archive_without_initdb(self) -> None:
        with expect_error(db.BundleError, "pg.tgz has no bin/initdb"):
            self.stage(self.downloads({"pg/bin/postgres": b""}, {"borg-dir/borg.exe": b""}), None)

    def test_refuses_a_borg_archive_without_the_binary(self) -> None:
        with expect_error(db.BundleError, "borg.tgz has no borg.exe"):
            self.stage(self.downloads({"pg/bin/initdb": b""}, {"borg-dir/borg": b""}), None)

    def libxml2_downloads(self, source_files: dict[str, bytes]) -> object:
        downloads = self.downloads({"pg/bin/initdb": b""}, {"borg-dir/borg.exe": b""})
        source = self.root / "libxml2.tar.gz"
        return dataclasses.replace(
            downloads,
            libxml2=db.Download(
                "libxml2.tar.gz", source.as_uri(), make_archive(source, source_files)
            ),
        )

    def stage_with_libxml2(self, downloads: object, libxml2: Path | None) -> None:
        db.stage(
            target=TARGET,
            downloads=downloads,
            server=self.server,
            agent=self.agent,
            web=self.web,
            docs=None,
            cache=self.root / "cache",
            desktop_dir=self.desktop,
            libxml2=libxml2,
        )

    def test_ships_libxml2_in_postgresqls_lib_with_its_license(self) -> None:
        built = self.root / "build" / "libxml2.so.2"
        built.parent.mkdir()
        built.write_bytes(b"libxml2")

        self.stage_with_libxml2(
            self.libxml2_downloads(
                {"libxml2-2.13.9/Copyright": b"libxml2 license", "libxml2-2.13.9/README": b""}
            ),
            built,
        )

        resources = self.desktop / "resources"
        assert (resources / "postgres" / "lib" / "libxml2.so.2").read_bytes() == b"libxml2"
        assert (resources / "licenses" / "libxml2.txt").read_bytes() == b"libxml2 license"
        assert (resources / "licenses" / "postgresql.txt").read_text() == "license text"

    def test_a_target_with_libxml2_needs_the_built_library(self) -> None:
        downloads = self.libxml2_downloads({"libxml2-2.13.9/Copyright": b""})
        with expect_error(db.BundleError, "ships libxml2: pass the built libxml2.so.2"):
            self.stage_with_libxml2(downloads, None)

    def test_a_target_without_libxml2_refuses_one(self) -> None:
        built = self.root / "libxml2.so.2"
        built.write_bytes(b"")
        downloads = self.downloads({"pg/bin/initdb": b""}, {"borg-dir/borg.exe": b""})
        with expect_error(db.BundleError, "ships no libxml2"):
            self.stage_with_libxml2(downloads, built)

    def test_refuses_a_libxml2_source_without_its_license(self) -> None:
        built = self.root / "libxml2.so.2"
        built.write_bytes(b"")
        downloads = self.libxml2_downloads({"libxml2-2.13.9/README": b""})
        with expect_error(db.BundleError, "libxml2.tar.gz has no Copyright"):
            self.stage_with_libxml2(downloads, built)

    def test_refuses_a_missing_server_binary(self) -> None:
        self.server.unlink()
        with expect_error(db.BundleError, "server is not a file"):
            self.stage(self.downloads({"pg/bin/initdb": b""}, {"borg-dir/borg.exe": b""}), None)


class StageAgentTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.agent = self.root / "agent.exe"
        self.agent.write_bytes(b"agent")
        self.out = self.root / "dist" / "assimilate-agent.zip"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def downloads(self, borg_files: dict[str, bytes]) -> object:
        borg = self.root / "borg.zip"
        notices = []
        for name, component in (
            ("postgresql.txt", db.Component.POSTGRESQL),
            ("borgbackup.txt", db.Component.BORG),
        ):
            source = self.root / name
            source.write_text(name)
            notices.append(
                db.Download(
                    name,
                    source.as_uri(),
                    hashlib.sha256(name.encode()).hexdigest(),
                    component=component,
                )
            )
        return db.TargetDownloads(
            postgresql=db.Download("unused.tgz", "file:///unused", "0" * 64),
            borg=db.Download("borg.zip", borg.as_uri(), make_zip(borg, borg_files)),
            notices=tuple(notices),
        )

    def stage(self, downloads: object) -> None:
        db.stage_agent(
            target=WINDOWS,
            downloads=downloads,
            agent=self.agent,
            cache=self.root / "cache",
            out=self.out,
        )

    def test_packs_the_agent_with_borg_and_only_borgs_license(self) -> None:
        self.stage(self.downloads({"borg.exe": b"borg", "_internal\\python3.dll": b""}))

        with zipfile.ZipFile(self.out) as zipped:
            assert sorted(zipped.namelist()) == [
                "assimilate-agent/assimilate-agent.exe",
                "assimilate-agent/borg/_internal/python3.dll",
                "assimilate-agent/borg/borg.exe",
                "assimilate-agent/licenses/borgbackup.txt",
            ]
            assert zipped.read("assimilate-agent/assimilate-agent.exe") == b"agent"
            assert zipped.read("assimilate-agent/borg/borg.exe") == b"borg"

    def test_refuses_a_borg_archive_without_the_binary(self) -> None:
        with expect_error(db.BundleError, "borg.zip has no borg.exe"):
            self.stage(self.downloads({"borg": b""}))
        assert not self.out.exists()


class MainTest(unittest.TestCase):
    def test_reports_errors_without_a_traceback(self) -> None:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = db.main(["fetch", "--target", "riscv64"])

        assert code == 1
        assert "error: no pinned downloads for riscv64" in stderr.getvalue()

    def test_libxml2_source_prints_the_verified_tarball(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache = root / "cache"
            cache.mkdir()
            pinned = manifest()
            pinned["libxml2"] = {
                "version": "2.13.9",
                "url": "https://download.gnome.org/sources/libxml2/2.13/libxml2-{version}.tar.xz",
                "sha256": make_archive(cache / "libxml2-2.13.9.tar.xz", {"l/Copyright": b""}),
                "targets": [TARGET],
            }
            with_libxml2 = root / "bundle.toml"
            with_libxml2.write_text(_to_toml(pinned))
            pinned["libxml2"]["targets"] = []
            without_libxml2 = root / "without-libxml2.toml"
            without_libxml2.write_text(_to_toml(pinned))
            command = ["libxml2-source", "--target", TARGET, "--cache", str(cache)]
            stdout, stderr = io.StringIO(), io.StringIO()
            # Already verified in the cache: nothing is downloaded.
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                shipped = db.main([*command, "--manifest", str(with_libxml2)])
                not_shipped = db.main([*command, "--manifest", str(without_libxml2)])

            assert shipped == 0
            assert stdout.getvalue().splitlines() == [str(cache / "libxml2-2.13.9.tar.xz")]
            assert not_shipped == 1
            assert f"error: {TARGET} ships no libxml2" in stderr.getvalue()

    def test_fetch_prints_each_attested_file_and_its_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache = root / "cache"
            cache.mkdir()
            pinned = manifest(
                make_archive(cache / f"postgresql-18.6.0-{TARGET}.tar.gz", {"p/x": b""}),
                make_archive(cache / "borg-linux-glibc235-x86_64-gh.tgz", {"b/x": b""}),
            )
            manifest_path = root / "bundle.toml"
            manifest_path.write_text(_to_toml(pinned))
            stdout = io.StringIO()
            # Both files are already verified in the cache: nothing is downloaded.
            with contextlib.redirect_stdout(stdout):
                code = db.main(
                    [
                        "fetch",
                        "--target",
                        TARGET,
                        "--manifest",
                        str(manifest_path),
                        "--cache",
                        str(cache),
                    ]
                )

            assert code == 0
            # PostgreSQL is pinned by digest only, so only borg is listed.
            assert stdout.getvalue().splitlines() == [
                f"{cache / 'borg-linux-glibc235-x86_64-gh.tgz'} borgbackup/borg",
            ]

    def test_fetch_reports_every_file_that_differs_from_its_pin(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache = root / "cache"
            cache.mkdir()
            make_archive(cache / f"postgresql-18.6.0-{TARGET}.tar.gz", {"p/x": b""})
            make_archive(cache / "borg-linux-glibc235-x86_64-gh.tgz", {"b/x": b""})
            manifest_path = root / "bundle.toml"
            # Pins that match nothing: both cached copies are dropped and
            # downloaded again from file URLs serving the same bytes.
            pinned = manifest("0" * 64, "1" * 64)
            for name, key in (
                (f"postgresql-18.6.0-{TARGET}.tar.gz", "postgresql"),
                ("borg-linux-glibc235-x86_64-gh.tgz", "borg"),
            ):
                source = root / name
                make_archive(source, {"x/y": b""})
                pinned[key]["url"] = source.as_uri()
            manifest_path.write_text(_to_toml(pinned))
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
                code = db.main(
                    [
                        "fetch",
                        "--target",
                        TARGET,
                        "--manifest",
                        str(manifest_path),
                        "--cache",
                        str(cache),
                    ]
                )

        assert code == 1
        assert "but bundle.toml pins " + "0" * 64 in stderr.getvalue()
        assert "but bundle.toml pins " + "1" * 64 in stderr.getvalue()


def _to_toml(pinned: dict) -> str:
    pg = pinned["postgresql"]
    borg = pinned["borg"]
    asset = borg["assets"][TARGET]
    libxml2 = pinned.get("libxml2")
    libxml2_lines = (
        [
            "[libxml2]",
            f'version = "{libxml2["version"]}"',
            f'url = "{libxml2["url"]}"',
            f'sha256 = "{libxml2["sha256"]}"',
            "targets = [" + ", ".join(f'"{target}"' for target in libxml2["targets"]) + "]",
        ]
        if libxml2 is not None
        else []
    )
    return "\n".join(
        [
            *libxml2_lines,
            "[postgresql]",
            f'version = "{pg["version"]}"',
            f"attested = {str(pg['attested']).lower()}",
            f'url = "{pg["url"]}"',
            "[postgresql.sha256]",
            f'{TARGET} = "{pg["sha256"][TARGET]}"',
            "[borg]",
            f'version = "{borg["version"]}"',
            f"attested = {str(borg['attested']).lower()}",
            f'url = "{borg["url"]}"',
            f"[borg.assets.{TARGET}]",
            f'name = "{asset["name"]}"',
            f'sha256 = "{asset["sha256"]}"',
        ]
    )


if __name__ == "__main__":
    unittest.main()
