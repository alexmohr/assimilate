# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Deterministic validation gate run before every push.

This is the harness's actual answer to "the cheap model keeps forgetting to
run pre-commit": it does not ask opencode to remember anything. It runs
AGENTS.md workflow step 4 (pre-commit) and the exact validation-checklist
commands from skills/rust/SKILL.md and skills/frontend/SKILL.md itself, in
Python, every single time, regardless of what the model did or didn't do.

Deliberately NOT run here: cargo dylint, the e2e/frontend-coverage CI jobs.
Those need Docker and are CI's job, not a per-cycle local gate's - see the
harness's own CI-failure-log-driven retry loop for that tier instead.

The DB-backed test suite (crates/server/tests/db_queries.rs and
integration.rs, both `#[sqlx::test]`-based) is different: it's run here too,
opportunistically, whenever a Postgres is reachable at DATABASE_URL (see
_db_reachable). Skipping it unconditionally used to mean opencode's local
retry loop could never actually see a regression there - it would validate
clean (since `cargo test --workspace --lib --bins` never touches those
files), push, and only find out several minutes later via a full CI
round-trip that its "fix" broke an integration test, burning through
HARNESS_MAX_STUCK_CYCLES on slow, unverifiable guesses instead of fast local
iteration. When no DB is reachable, this falls back to `--lib --bins` only
and defers to CI for that tier, same as before.

Each command is streamed via procstream.run_streaming rather than captured
silently until it exits - pre-commit (installing hook environments on a
first run) and cargo test/clippy in particular can run for minutes, and
without this a working-but-slow validation pass looks identical to a hang.
"""

from __future__ import annotations

import hashlib
import logging
import os
from dataclasses import dataclass
from pathlib import Path

import procstream

log = logging.getLogger("harness.validate")

_LOCKFILE_HASH_MARKER = ".harness-lockfile-hash"
# Matches this repo's own CI services (see .github/workflows/ci.yml) - not a
# credential, just the fixed local dev/CI Postgres this repo's tests assume
# (see crates/server/tests/db_queries.rs's own module docs).
_DEFAULT_DATABASE_URL = "postgres://borg:borg_secret@localhost:5432/borg"

RUST_FMT_ARGS = [
    "cargo",
    "+nightly",
    "fmt",
    "--",
    "--config",
    "error_on_unformatted=true,error_on_line_overflow=true,format_strings=true,"
    "group_imports=StdExternalCrate,imports_granularity=Crate",
]


@dataclass
class ValidationResult:
    ok: bool
    step: str
    output: str


def _run(
    cwd: Path, args: list[str], timeout: int, env: dict[str, str] | None = None
) -> ValidationResult:
    step = " ".join(args)
    result = procstream.run_streaming(args, cwd, timeout, log, step, env=env)
    if result.timed_out:
        output = f"timed out after {timeout}s:\n{result.output}"
        return ValidationResult(ok=False, step=step, output=output)
    return ValidationResult(ok=result.returncode == 0, step=step, output=result.output)


def run_precommit(cwd: Path, timeout: int = 900) -> ValidationResult:
    return _run(
        cwd, ["uv", "run", "pre-commit", "run", "--all-files", "--show-diff-on-failure"], timeout
    )


def _db_env() -> dict[str, str]:
    env = dict(os.environ)
    env.setdefault("DATABASE_URL", _DEFAULT_DATABASE_URL)
    return env


def _db_reachable(cwd: Path) -> bool:
    """Best-effort probe: true if a Postgres is reachable at DATABASE_URL and
    this repo's migrations apply cleanly against it - the same DB this
    repo's CI spins up for its Database Integration Tests / Nightly Tests
    jobs. `cargo sqlx migrate run` is idempotent (a no-op against a DB
    that's already current), so this is safe and cheap to call every cycle.
    """
    result = _run(
        cwd,
        ["cargo", "sqlx", "migrate", "run", "--source", "crates/server/migrations"],
        timeout=60,
        env=_db_env(),
    )
    return result.ok


def run_rust_checks(cwd: Path, timeout: int = 1800) -> list[ValidationResult]:
    steps = [
        RUST_FMT_ARGS,
        ["cargo", "+nightly", "clippy", "--workspace", "--", "-D", "warnings"],
        ["cargo", "test", "--workspace", "--lib", "--bins"],
        ["cargo", "deny", "check"],
    ]
    results = []
    for step in steps:
        result = _run(cwd, step, timeout)
        results.append(result)
        if not result.ok:
            return results

    if _db_reachable(cwd):
        # Mirrors CI's own "Nightly Tests" job (cargo test --workspace --
        # --test-threads=1 with DATABASE_URL set): the #[sqlx::test] suite
        # in db_queries.rs/integration.rs isolates each test in its own DB,
        # but running them single-threaded avoids incidental cross-test
        # contention on the same Postgres instance.
        results.append(
            _run(
                cwd,
                ["cargo", "+nightly", "test", "--workspace", "--", "--test-threads=1"],
                timeout,
                env=_db_env(),
            )
        )
    else:
        log.info(
            "no Postgres reachable at DATABASE_URL (or migrations failed); skipping the "
            "DB-backed test suite locally - only CI will catch a regression there this cycle"
        )
    return results


def _lockfile_hash(frontend_dir: Path) -> str | None:
    lockfile = frontend_dir / "package-lock.json"
    return hashlib.sha256(lockfile.read_bytes()).hexdigest() if lockfile.exists() else None


def _npm_ci_needed(frontend_dir: Path) -> bool:
    """node_modules/ is preserved across cycles (see git_ops._CLEAN_EXCLUDES)
    to avoid a full reinstall every single time - but the harness works many
    different PRs/branches in the same HARNESS_REPO_DIR checkout in
    sequence, and "does node_modules exist" doesn't mean "matches the
    package-lock.json of whichever branch is checked out right now". Unlike
    cargo, `npm run lint`/`build`/`test` never reconcile node_modules
    against the lockfile themselves - only `npm ci`/`install` do - so a
    stale install from a previous PR's dependencies would otherwise produce
    spurious failures with nothing to do with the PR actually being fixed.
    Reinstall whenever the lockfile's content differs from the one last
    installed, tracked via a hash marker inside node_modules/ itself (so it
    is wiped together with node_modules/ if that's ever cleaned out).
    """
    node_modules = frontend_dir / "node_modules"
    if not node_modules.exists():
        return True
    marker = node_modules / _LOCKFILE_HASH_MARKER
    if not marker.exists():
        return True
    return marker.read_text().strip() != (_lockfile_hash(frontend_dir) or "")


def run_frontend_checks(cwd: Path, timeout: int = 1800) -> list[ValidationResult]:
    frontend_dir = cwd / "frontend"
    npm_ci_needed = _npm_ci_needed(frontend_dir)
    steps = [["npm", "ci"]] if npm_ci_needed else []
    steps += [
        ["npm", "run", "format:check"],
        ["npm", "run", "lint"],
        ["npm", "run", "build"],
        ["npm", "run", "test"],
    ]
    results = []
    for step in steps:
        result = _run(frontend_dir, step, timeout)
        results.append(result)
        if not result.ok:
            break
    if npm_ci_needed and results and results[0].ok:
        lockfile_hash = _lockfile_hash(frontend_dir)
        if lockfile_hash is not None:
            (frontend_dir / "node_modules" / _LOCKFILE_HASH_MARKER).write_text(lockfile_hash)
    return results


def touches(
    paths: list[str], *, suffixes: tuple[str, ...] = (), prefixes: tuple[str, ...] = ()
) -> bool:
    return any(p.endswith(suffixes) or p.startswith(prefixes) for p in paths)


def run_all(cwd: Path, changed_paths: list[str]) -> ValidationResult:
    """Runs pre-commit, then whichever of the rust/frontend checklists apply.

    Stops at the first failing step and returns it directly so the caller can
    feed exactly that failure back to opencode.
    """
    precommit = run_precommit(cwd)
    if not precommit.ok:
        return precommit

    if touches(changed_paths, suffixes=(".rs",), prefixes=("Cargo.toml", "Cargo.lock", "crates/")):
        for result in run_rust_checks(cwd):
            if not result.ok:
                return result

    if touches(changed_paths, prefixes=("frontend/",)):
        for result in run_frontend_checks(cwd):
            if not result.ok:
                return result

    return ValidationResult(ok=True, step="all", output="")
