# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Stages everything the desktop installers ship for one target.

`cargo tauri build` with desktop/tauri.bundle.conf.json packs what this
script puts under desktop/: the server and agent next to the app
(desktop/binaries/assimilate-{server,agent}-<target>, Tauri's externalBin
naming) and PostgreSQL,
borg, their license texts, the web UI and the docs as resources
(desktop/resources/). PostgreSQL, borg and the license texts are downloaded
from the URLs pinned in desktop/bundle.toml, and a download whose SHA-256
differs from the pin is refused.

    python3 scripts/desktop_bundle.py fetch --target aarch64-apple-darwin
    python3 scripts/desktop_bundle.py stage --target x86_64-unknown-linux-gnu \\
        --server target/release/server --agent target/release/agent \\
        --web frontend/dist --docs docs_html

`fetch` only downloads and verifies, printing each file and the GitHub
repository it came from, so CI can check the files' build attestations.
"""

import argparse
import hashlib
import shutil
import sys
import tarfile
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

import tomllib

REPO_ROOT = Path(__file__).resolve().parents[1]
DESKTOP_DIR = REPO_ROOT / "desktop"
MANIFEST = DESKTOP_DIR / "bundle.toml"
DEFAULT_CACHE = REPO_ROOT / "target" / "desktop-bundle-cache"

# The borg binary inside its staged directory. BorgBackup's PyInstaller spec
# names it borg.exe on every platform; desktop/src/layout.rs looks for it here.
BORG_EXECUTABLE = "borg.exe"


class BundleError(Exception):
    """A problem that stops staging, explained for the person running it."""


@dataclass(frozen=True)
class Download:
    """One pinned file to download."""

    name: str
    url: str
    sha256: str

    @property
    def github_repo(self) -> str:
        """The `owner/repo` a github.com release URL points into."""
        parsed = urllib.parse.urlparse(self.url)
        parts = PurePosixPath(parsed.path).parts
        if parsed.hostname != "github.com" or len(parts) < 3:
            raise BundleError(f"{self.url} is not a GitHub release download")
        return f"{parts[1]}/{parts[2]}"


@dataclass(frozen=True)
class TargetDownloads:
    """What one target needs: a PostgreSQL archive, a borg archive and the
    license texts that ship with them."""

    postgresql: Download
    borg: Download
    notices: tuple[Download, ...] = ()


def load_manifest(path: Path) -> dict:
    with path.open("rb") as file:
        return tomllib.load(file)


def downloads_for(manifest: dict, target: str) -> TargetDownloads:
    """The pinned PostgreSQL and borg downloads for `target`."""
    postgresql = manifest["postgresql"]
    borg = manifest["borg"]
    pg_sha = postgresql["sha256"].get(target)
    borg_asset = borg["assets"].get(target)
    if pg_sha is None or borg_asset is None:
        known = sorted(set(postgresql["sha256"]) & set(borg["assets"]))
        raise BundleError(
            f"no pinned downloads for {target}; bundle.toml covers {', '.join(known)}"
        )
    pg_url = postgresql["url"].format(version=postgresql["version"], target=target)
    borg_url = borg["url"].format(version=borg["version"], asset=borg_asset["name"])
    return TargetDownloads(
        postgresql=Download(PurePosixPath(urllib.parse.urlparse(pg_url).path).name, pg_url, pg_sha),
        borg=Download(borg_asset["name"], borg_url, borg_asset["sha256"]),
        notices=tuple(
            Download(notice["name"], notice["url"], notice["sha256"])
            for notice in manifest.get("notices", ())
        ),
    )


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(path: Path, expected: str) -> None:
    """Raises unless `path`'s SHA-256 is `expected`."""
    actual = sha256_of(path)
    if actual != expected.lower():
        raise BundleError(f"{path.name}: SHA-256 is {actual}, but bundle.toml pins {expected}")


def fetch(download: Download, cache: Path) -> Path:
    """Downloads `download` into `cache` unless a verified copy is there."""
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / download.name
    if path.exists():
        try:
            verify(path, download.sha256)
            return path
        except BundleError:
            path.unlink()
    partial = path.with_name(path.name + ".part")
    with urllib.request.urlopen(download.url) as response, partial.open("wb") as out:
        shutil.copyfileobj(response, out)
    try:
        verify(partial, download.sha256)
    except BundleError:
        partial.unlink()
        raise
    partial.rename(path)
    return path


def _parts(name: str) -> tuple[str, ...]:
    """`name`'s path components, without a leading `./`."""
    parts = PurePosixPath(name).parts
    return parts[1:] if parts[:1] == (".",) else parts


def _strip_top(name: str) -> str | None:
    """`name` without its first path component, or None for the top directory."""
    parts = _parts(name)
    return str(PurePosixPath(*parts[1:])) if len(parts) > 1 else None


def extract_without_top_dir(archive: Path, dest: Path) -> None:
    """Extracts a .tar.gz whose entries all sit under one top-level directory
    into `dest`, dropping that directory. tarfile's `data` filter refuses
    absolute paths, `..` and links leading out of `dest`."""
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:gz") as tar:
        tops = {parts[0] for m in tar.getmembers() if (parts := _parts(m.name))}
        if len(tops) != 1:
            raise BundleError(
                f"{archive.name}: expected one top-level directory, found {sorted(tops)}"
            )
        for member in tar.getmembers():
            stripped = _strip_top(member.name)
            if stripped is None:
                continue
            member.name = stripped
            if member.islnk():
                linked = _strip_top(member.linkname)
                if linked is None:
                    raise BundleError(f"{archive.name}: hard link to the top directory")
                member.linkname = linked
            tar.extract(member, dest, filter="data")


def _replace_dir(source: Path, dest: Path) -> None:
    if not source.is_dir():
        raise BundleError(f"{source} is not a directory")
    shutil.rmtree(dest, ignore_errors=True)
    shutil.copytree(source, dest, symlinks=True)


def _install_binary(source: Path, dest: Path) -> None:
    if not source.is_file():
        raise BundleError(f"{source} is not a file")
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, dest)
    dest.chmod(0o755)


def stage(
    *,
    target: str,
    downloads: TargetDownloads,
    server: Path,
    agent: Path,
    web: Path,
    docs: Path | None,
    cache: Path,
    desktop_dir: Path,
) -> None:
    """Puts everything the installer for `target` ships under `desktop_dir`."""
    resources = desktop_dir / "resources"
    binaries = desktop_dir / "binaries"
    shutil.rmtree(resources, ignore_errors=True)
    shutil.rmtree(binaries, ignore_errors=True)

    postgres_dir = resources / "postgres"
    extract_without_top_dir(fetch(downloads.postgresql, cache), postgres_dir)
    if not (postgres_dir / "bin" / "initdb").is_file():
        raise BundleError(f"{downloads.postgresql.name} has no bin/initdb")

    borg_dir = resources / "borg"
    extract_without_top_dir(fetch(downloads.borg, cache), borg_dir)
    borg = borg_dir / BORG_EXECUTABLE
    if not borg.is_file():
        raise BundleError(f"{downloads.borg.name} has no {BORG_EXECUTABLE}")
    borg.chmod(0o755)

    licenses = resources / "licenses"
    licenses.mkdir(parents=True)
    for notice in downloads.notices:
        shutil.copyfile(fetch(notice, cache), licenses / notice.name)

    _replace_dir(web, resources / "web")
    if docs is not None:
        _replace_dir(docs, resources / "docs")

    # Tauri's externalBin: <name>-<target triple>, installed as <name> next to
    # the app; desktop/src/layout.rs looks for these names.
    _install_binary(server, binaries / f"assimilate-server-{target}")
    _install_binary(agent, binaries / f"assimilate-agent-{target}")


def _fetch_command(args: argparse.Namespace, downloads: TargetDownloads) -> None:
    # Only the release binaries carry build attestations; the license texts
    # are pinned by digest alone. Every file is tried, so one run reports
    # each digest that differs from its pin.
    problems = []
    for download in (downloads.postgresql, downloads.borg):
        try:
            print(f"{fetch(download, args.cache)} {download.github_repo}")
        except BundleError as error:
            problems.append(str(error))
    if problems:
        raise BundleError("\n".join(problems))


def _stage_command(args: argparse.Namespace, downloads: TargetDownloads) -> None:
    stage(
        target=args.target,
        downloads=downloads,
        server=args.server,
        agent=args.agent,
        web=args.web,
        docs=args.docs,
        cache=args.cache,
        desktop_dir=args.desktop_dir,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(required=True)

    fetch_cmd = commands.add_parser("fetch", help="download and verify the pinned files")
    fetch_cmd.set_defaults(run=_fetch_command)
    stage_cmd = commands.add_parser("stage", help="fetch, then stage everything for one target")
    stage_cmd.set_defaults(run=_stage_command)
    for command in (fetch_cmd, stage_cmd):
        command.add_argument("--target", required=True, help="Rust target triple")
        command.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
        command.add_argument("--manifest", type=Path, default=MANIFEST)
    stage_cmd.add_argument("--server", type=Path, required=True)
    stage_cmd.add_argument("--agent", type=Path, required=True)
    stage_cmd.add_argument("--web", type=Path, required=True, help="the built web UI")
    stage_cmd.add_argument("--docs", type=Path, help="the built documentation site")
    stage_cmd.add_argument("--desktop-dir", type=Path, default=DESKTOP_DIR)
    args = parser.parse_args(argv)

    try:
        args.run(args, downloads_for(load_manifest(args.manifest), args.target))
    except BundleError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
