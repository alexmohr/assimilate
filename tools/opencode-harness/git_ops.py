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


def checkout_branch_at_remote(cwd: Path, branch: str) -> None:
    """Get a clean local checkout of `branch` matching origin exactly.

    Hard-resets rather than merging so a prior crashed/aborted harness run
    never leaves stale local edits in the way of a fresh attempt.
    """
    fetch(cwd, branch)
    _run(cwd, ["checkout", "-B", branch, f"origin/{branch}"])
    _run(cwd, ["reset", "--hard", f"origin/{branch}"])
    _run(cwd, ["clean", "-fdx", "--exclude=.state.json"])


def checkout_new_branch_from_base(cwd: Path, branch: str, base: str) -> None:
    fetch(cwd, base)
    _run(cwd, ["checkout", "-B", branch, f"origin/{base}"])
    _run(cwd, ["clean", "-fdx", "--exclude=.state.json"])


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
