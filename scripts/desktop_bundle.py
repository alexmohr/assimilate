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

`fetch` only downloads and verifies, printing each file bundle.toml marks
`attested` and the GitHub repository it came from, so CI can check those
files' build attestations.

On Linux the installers also ship a libxml2.so.2 for PostgreSQL, built by
scripts/build-libxml2.sh from the source tarball pinned in bundle.toml:
`libxml2-source` fetches and verifies that tarball and prints its path, and
`stage --libxml2` puts the built library in PostgreSQL's lib/ along with
libxml2's license text.

    src=$(python3 scripts/desktop_bundle.py libxml2-source --target x86_64-unknown-linux-gnu)
    scripts/build-libxml2.sh "$src" build/libxml2.so.2

`stage-agent` writes the standalone agent's release archive for a target
instead: the agent with the same pinned borg and its license texts, which
the Windows agent ships with since Windows has no borg package to install.

    python3 scripts/desktop_bundle.py stage-agent --target x86_64-pc-windows-msvc \\
        --agent target/release/agent.exe --out dist/assimilate-agent.zip
"""

import argparse
import hashlib
import shutil
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass
from enum import Enum
from pathlib import Path, PurePosixPath, PureWindowsPath

import tomllib

REPO_ROOT = Path(__file__).resolve().parents[1]
DESKTOP_DIR = REPO_ROOT / "desktop"
MANIFEST = DESKTOP_DIR / "bundle.toml"
DEFAULT_CACHE = REPO_ROOT / "target" / "desktop-bundle-cache"

# The borg binary inside its staged directory. BorgBackup's PyInstaller spec
# names it borg.exe on every platform, and so does the Windows build's;
# desktop/src/layout.rs looks for it here.
BORG_EXECUTABLE = "borg.exe"


class BundleError(Exception):
    """A problem that stops staging, explained for the person running it."""


class Component(Enum):
    """A third-party component the installers ship, which a license notice
    belongs to."""

    POSTGRESQL = "postgresql"
    BORG = "borg"
    LIBXML2 = "libxml2"


@dataclass(frozen=True)
class Download:
    """One pinned file to download."""

    name: str
    url: str
    sha256: str
    # Whether the upstream release publishes a GitHub build attestation for it.
    attested: bool = False
    # For a license notice, the component it covers.
    component: Component | None = None

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
    """What one target needs: a PostgreSQL archive, a borg archive, the
    license texts that ship with them and, on Linux, the libxml2 source the
    library PostgreSQL links is built from."""

    postgresql: Download
    borg: Download
    notices: tuple[Download, ...] = ()
    libxml2: Download | None = None


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
    # An asset may name its own URL and attestation: BorgBackup publishes no
    # Windows build, so that one target comes from elsewhere (see bundle.toml).
    borg_url = borg_asset.get("url") or borg["url"].format(
        version=borg["version"], asset=borg_asset["name"]
    )
    libxml2 = manifest.get("libxml2")
    libxml2_download = None
    if libxml2 is not None and target in libxml2["targets"]:
        libxml2_url = libxml2["url"].format(version=libxml2["version"])
        libxml2_download = Download(
            PurePosixPath(urllib.parse.urlparse(libxml2_url).path).name,
            libxml2_url,
            libxml2["sha256"],
        )
    return TargetDownloads(
        libxml2=libxml2_download,
        postgresql=Download(
            PurePosixPath(urllib.parse.urlparse(pg_url).path).name,
            pg_url,
            pg_sha,
            attested=postgresql["attested"],
        ),
        borg=Download(
            borg_asset["name"],
            borg_url,
            borg_asset["sha256"],
            attested=borg_asset.get("attested", borg["attested"]),
        ),
        notices=tuple(
            Download(
                notice["name"],
                notice["url"],
                notice["sha256"],
                component=Component(notice["component"]),
            )
            for notice in manifest.get("notices", ())
        ),
    )


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(path: Path, expected: str, name: str) -> None:
    """Raises unless `path`'s SHA-256 is `expected`; `name` is what the error
    calls the file."""
    actual = sha256_of(path)
    if actual != expected.lower():
        raise BundleError(f"{name}: SHA-256 is {actual}, but bundle.toml pins {expected}")


def fetch(download: Download, cache: Path) -> Path:
    """Downloads `download` into `cache` unless a verified copy is there."""
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / download.name
    if path.exists():
        try:
            verify(path, download.sha256, download.name)
            return path
        except BundleError:
            path.unlink()
    partial = path.with_name(path.name + ".part")
    with urllib.request.urlopen(download.url) as response, partial.open("wb") as out:
        shutil.copyfileobj(response, out)
    try:
        verify(partial, download.sha256, download.name)
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


def _zip_member_path(name: str) -> PurePosixPath:
    """A zip entry's path, whichever separator the archive was written with.
    Refuses absolute paths, drive letters and `..`, which would land outside
    the destination."""
    path = PureWindowsPath(name)
    if path.drive or path.root or ".." in path.parts:
        raise BundleError(f"unsafe path in archive: {name}")
    return PurePosixPath(*path.parts)


def extract_zip(archive: Path, dest: Path) -> None:
    """Extracts a .zip into `dest` as it is laid out, without a top-level
    directory to drop: the Windows borg build has its files at the root."""
    dest.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        for member in zipped.infolist():
            target = dest / _zip_member_path(member.filename)
            if member.is_dir() or member.filename.endswith("\\"):
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with zipped.open(member) as source, target.open("wb") as out:
                shutil.copyfileobj(source, out)


def extract(archive: Path, dest: Path) -> None:
    """Extracts a pinned archive: a .zip as laid out, a .tar.gz without its
    top-level directory."""
    if archive.suffix == ".zip":
        extract_zip(archive, dest)
    else:
        extract_without_top_dir(archive, dest)


# The name the bundled PostgreSQL links libxml2 by, and the license file in
# libxml2's source tarball.
LIBXML2_SONAME = "libxml2.so.2"
LIBXML2_LICENSE = "Copyright"
LIBXML2_NOTICE = "libxml2.txt"


def libxml2_license(source: Path) -> bytes:
    """The license text in libxml2's source tarball."""
    with tarfile.open(source) as tar:
        for member in tar.getmembers():
            if member.isfile() and _strip_top(member.name) == LIBXML2_LICENSE:
                extracted = tar.extractfile(member)
                if extracted is not None:
                    return extracted.read()
    raise BundleError(f"{source.name} has no {LIBXML2_LICENSE}")


def exe_suffix(target: str) -> str:
    """What executables are named with on `target`."""
    return ".exe" if target.endswith("-windows-msvc") else ""


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
    libxml2: Path | None = None,
) -> None:
    """Puts everything the installer for `target` ships under `desktop_dir`.
    `libxml2` is the libxml2.so.2 built from `downloads.libxml2`, which a
    target that pins one needs and any other refuses."""
    if downloads.libxml2 is not None and libxml2 is None:
        raise BundleError(f"{target} ships libxml2: pass the built {LIBXML2_SONAME}")
    if downloads.libxml2 is None and libxml2 is not None:
        raise BundleError(f"{target} ships no libxml2")
    resources = desktop_dir / "resources"
    binaries = desktop_dir / "binaries"
    shutil.rmtree(resources, ignore_errors=True)
    shutil.rmtree(binaries, ignore_errors=True)

    exe = exe_suffix(target)
    postgres_dir = resources / "postgres"
    extract(fetch(downloads.postgresql, cache), postgres_dir)
    if not (postgres_dir / "bin" / f"initdb{exe}").is_file():
        raise BundleError(f"{downloads.postgresql.name} has no bin/initdb{exe}")

    borg_dir = resources / "borg"
    extract(fetch(downloads.borg, cache), borg_dir)
    borg = borg_dir / BORG_EXECUTABLE
    if not borg.is_file():
        raise BundleError(f"{downloads.borg.name} has no {BORG_EXECUTABLE}")
    borg.chmod(0o755)

    licenses = resources / "licenses"
    licenses.mkdir(parents=True)
    for notice in downloads.notices:
        shutil.copyfile(fetch(notice, cache), licenses / notice.name)

    if downloads.libxml2 is not None and libxml2 is not None:
        _install_binary(libxml2, postgres_dir / "lib" / LIBXML2_SONAME)
        license_text = libxml2_license(fetch(downloads.libxml2, cache))
        (licenses / LIBXML2_NOTICE).write_bytes(license_text)

    _replace_dir(web, resources / "web")
    if docs is not None:
        _replace_dir(docs, resources / "docs")

    # Tauri's externalBin: <name>-<target triple>, installed as <name> next to
    # the app; desktop/src/layout.rs looks for these names.
    _install_binary(server, binaries / f"assimilate-server-{target}{exe}")
    _install_binary(agent, binaries / f"assimilate-agent-{target}{exe}")


# The standalone agent archive's top-level directory.
AGENT_ARCHIVE_DIR = "assimilate-agent"


def stage_agent(
    *, target: str, downloads: TargetDownloads, agent: Path, cache: Path, out: Path
) -> None:
    """Writes `out`, a .zip holding the agent for `target` as
    assimilate-agent[.exe], borg under borg/ (where the agent looks for it
    next to itself) and the license texts under licenses/, all below one
    top-level directory."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / AGENT_ARCHIVE_DIR
        borg_dir = root / "borg"
        extract(fetch(downloads.borg, cache), borg_dir)
        if not (borg_dir / BORG_EXECUTABLE).is_file():
            raise BundleError(f"{downloads.borg.name} has no {BORG_EXECUTABLE}")
        licenses = root / "licenses"
        licenses.mkdir(parents=True)
        for notice in downloads.notices:
            if notice.component is Component.BORG:
                shutil.copyfile(fetch(notice, cache), licenses / notice.name)
        _install_binary(agent, root / f"assimilate-agent{exe_suffix(target)}")

        out.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zipped:
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    zipped.write(path, path.relative_to(root.parent).as_posix())


def _fetch_command(args: argparse.Namespace, downloads: TargetDownloads) -> None:
    # Prints each attested file with the repository whose attestation must
    # vouch for it. Every file is tried, so one run reports each digest that
    # differs from its pin.
    problems = []
    optional = (downloads.libxml2,) if downloads.libxml2 is not None else ()
    for download in (downloads.postgresql, downloads.borg, *optional):
        try:
            path = fetch(download, args.cache)
        except BundleError as error:
            problems.append(str(error))
            continue
        if download.attested:
            print(f"{path} {download.github_repo}")
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
        libxml2=args.libxml2,
    )


def _libxml2_source_command(args: argparse.Namespace, downloads: TargetDownloads) -> None:
    if downloads.libxml2 is None:
        raise BundleError(f"{args.target} ships no libxml2")
    print(fetch(downloads.libxml2, args.cache))


def _stage_agent_command(args: argparse.Namespace, downloads: TargetDownloads) -> None:
    stage_agent(
        target=args.target,
        downloads=downloads,
        agent=args.agent,
        cache=args.cache,
        out=args.out,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(required=True)

    fetch_cmd = commands.add_parser("fetch", help="download and verify the pinned files")
    fetch_cmd.set_defaults(run=_fetch_command)
    stage_cmd = commands.add_parser("stage", help="fetch, then stage everything for one target")
    stage_cmd.set_defaults(run=_stage_command)
    agent_cmd = commands.add_parser(
        "stage-agent", help="write the standalone agent's archive, with borg"
    )
    agent_cmd.set_defaults(run=_stage_agent_command)
    libxml2_cmd = commands.add_parser(
        "libxml2-source", help="fetch the pinned libxml2 source and print its path"
    )
    libxml2_cmd.set_defaults(run=_libxml2_source_command)
    for command in (fetch_cmd, stage_cmd, agent_cmd, libxml2_cmd):
        command.add_argument("--target", required=True, help="Rust target triple")
        command.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
        command.add_argument("--manifest", type=Path, default=MANIFEST)
    stage_cmd.add_argument("--server", type=Path, required=True)
    stage_cmd.add_argument("--agent", type=Path, required=True)
    stage_cmd.add_argument("--web", type=Path, required=True, help="the built web UI")
    stage_cmd.add_argument("--docs", type=Path, help="the built documentation site")
    stage_cmd.add_argument("--desktop-dir", type=Path, default=DESKTOP_DIR)
    stage_cmd.add_argument(
        "--libxml2", type=Path, help="the libxml2.so.2 built by scripts/build-libxml2.sh"
    )
    agent_cmd.add_argument("--agent", type=Path, required=True)
    agent_cmd.add_argument("--out", type=Path, required=True, help="the .zip to write")
    args = parser.parse_args(argv)

    try:
        args.run(args, downloads_for(load_manifest(args.manifest), args.target))
    except BundleError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
