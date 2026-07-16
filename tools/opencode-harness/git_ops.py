# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""git plumbing. The harness owns every commit and push - opencode is only
ever asked to edit files, never to run `git commit`/`git push` itself. That
is deliberate: a cheap model forgetting to run pre-commit, or writing a
non-conventional-commit message, is exactly the failure mode this harness
exists to remove, and the only reliable fix is to not let it hold the
commit/push button at all.
"""

from __future__ import annotations

import logging
import subprocess
from pathlib import Path

log = logging.getLogger("harness.git")


class GitError(RuntimeError):
    pass


def _run(
    cwd: Path, args: list[str], timeout: int = 120, check: bool = True
) -> subprocess.CompletedProcess:
    proc = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if check and proc.returncode != 0:
        raise GitError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc


def fetch(cwd: Path, ref: str) -> None:
    _run(cwd, ["fetch", "origin", ref])


_STALE_LOCK_NAMES = ("index.lock", "HEAD.lock", "MERGE_HEAD.lock", "shallow.lock")


def _clear_stale_locks(cwd: Path) -> None:
    """Removes leftover git lock files from a subprocess killed mid-operation.

    opencode runs under a hard timeout and gets SIGKILL'd if it overruns -
    if that happens while it (or a tool it invoked, e.g. a pre-commit hook)
    was mid-write to the index, the lock file it held never gets released.
    Every subsequent git command in this checkout then fails with "Unable to
    create '.git/index.lock': File exists" forever, since nothing is left
    alive to remove it. This is the harness's own recovery point for that -
    called before the first git command of a fresh checkout, where it's safe
    to assume no concurrent git process of ours is actually running.
    """
    git_dir = cwd / ".git"
    for name in _STALE_LOCK_NAMES:
        lock = git_dir / name
        if lock.exists():
            log.warning("removing stale git lock file: %s", lock)
            lock.unlink()
    for lock in git_dir.glob("refs/**/*.lock"):
        log.warning("removing stale git lock file: %s", lock)
        lock.unlink()


# -x makes `git clean` remove gitignored files too (needed to wipe stale
# generated/build files a previous branch might have left behind) - but
# target/, node_modules/, and Vite's cache are also gitignored, and wiping
# them on every single checkout forces a full cargo rebuild and npm reinstall
# from scratch every cycle. Excluding them preserves incremental compilation
# and installed packages across cycles; they're safe to keep since cargo and
# npm are both content-addressed against the current lockfiles/sources, not
# branch-specific state that could go stale in a way a rebuild wouldn't catch.
_CLEAN_EXCLUDES = [
    "--exclude=.state.json",
    "--exclude=target/",
    "--exclude=node_modules/",
    "--exclude=frontend/node_modules/",
    "--exclude=.vite/",
]


def checkout_branch_at_remote(cwd: Path, branch: str) -> None:
    """Get a clean local checkout of `branch` matching origin exactly.

    Hard-resets rather than merging so a prior crashed/aborted harness run
    never leaves stale local edits in the way of a fresh attempt.
    """
    _clear_stale_locks(cwd)
    fetch(cwd, branch)
    _run(cwd, ["checkout", "-B", branch, f"origin/{branch}"])
    _run(cwd, ["reset", "--hard", f"origin/{branch}"])
    _run(cwd, ["clean", "-fdx", *_CLEAN_EXCLUDES])


def checkout_new_branch_from_base(cwd: Path, branch: str, base: str) -> None:
    _clear_stale_locks(cwd)
    fetch(cwd, base)
    _run(cwd, ["checkout", "-B", branch, f"origin/{base}"])
    _run(cwd, ["clean", "-fdx", *_CLEAN_EXCLUDES])


def discard_uncommitted_changes(cwd: Path) -> None:
    """Wipes any uncommitted edits left behind by a fix attempt that didn't
    converge (opencode never commits its own changes, so a validation
    failure otherwise leaves them sitting in the working tree until some
    unrelated future checkout happens to clean them up). Resets whatever
    branch is currently checked out in place - doesn't fetch or switch
    branches.
    """
    _run(cwd, ["reset", "--hard", "HEAD"])
    _run(cwd, ["clean", "-fdx", *_CLEAN_EXCLUDES])


def rebase_onto(cwd: Path, base: str) -> tuple[bool, str]:
    """Attempt a rebase onto origin/<base>. Returns (clean, conflict_status_text)."""
    fetch(cwd, base)
    proc = _run(cwd, ["rebase", f"origin/{base}"], check=False)
    if proc.returncode == 0:
        return True, ""
    status = _run(cwd, ["status"], check=False).stdout
    return False, status


def abort_rebase(cwd: Path) -> None:
    _run(cwd, ["rebase", "--abort"], check=False)


def continue_rebase(cwd: Path) -> tuple[bool, str]:
    _run(cwd, ["add", "-A"], check=False)
    proc = _run(cwd, ["rebase", "--continue"], check=False)
    if proc.returncode == 0:
        return True, ""
    return False, _run(cwd, ["status"], check=False).stdout


def has_uncommitted_changes(cwd: Path) -> bool:
    return bool(_run(cwd, ["status", "--porcelain"]).stdout.strip())


def changed_files(cwd: Path, against: str = "HEAD") -> list[str]:
    staged = _run(cwd, ["diff", "--name-only", "--cached"]).stdout.splitlines()
    unstaged = _run(cwd, ["diff", "--name-only"]).stdout.splitlines()
    untracked = _run(cwd, ["ls-files", "--others", "--exclude-standard"]).stdout.splitlines()
    return sorted(set(staged) | set(unstaged) | set(untracked))


def changed_files_between(cwd: Path, ref_a: str, ref_b: str) -> list[str]:
    """Files that differ between two *committed* refs - unlike `changed_files`
    (uncommitted staged/unstaged/untracked only), this sees content that's
    already been committed, e.g. a conflict resolution landed via
    `continue_rebase` before any validation has run over it.
    """
    return _run(cwd, ["diff", "--name-only", ref_a, ref_b]).stdout.splitlines()


def commit(cwd: Path, message: str) -> bool:
    """Stage everything and commit. Returns False if there was nothing to commit."""
    _run(cwd, ["add", "-A"])
    if not _run(cwd, ["diff", "--cached", "--name-only"]).stdout.strip():
        return False
    _run(cwd, ["commit", "-m", message])
    return True


def push(cwd: Path, branch: str, force_with_lease: bool = False) -> None:
    args = ["push", "-u", "origin", branch]
    if force_with_lease:
        args.insert(1, "--force-with-lease")
    _run(cwd, args)


def head_sha(cwd: Path) -> str:
    return _run(cwd, ["rev-parse", "HEAD"]).stdout.strip()


def remote_head_sha(cwd: Path, branch: str) -> str:
    """The remote-tracking ref's SHA as of the last fetch of `branch` - not a
    live query. Used to tell whether a local rebase actually moved HEAD
    since the last checkout/fetch, not whether origin has since changed.
    """
    return _run(cwd, ["rev-parse", f"origin/{branch}"]).stdout.strip()
