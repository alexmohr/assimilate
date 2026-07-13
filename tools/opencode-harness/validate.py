# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Deterministic validation gate run before every push.

This is the harness's actual answer to "the cheap model keeps forgetting to
run pre-commit": it does not ask opencode to remember anything. It runs
AGENTS.md workflow step 4 (pre-commit) and the exact validation-checklist
commands from skills/rust/SKILL.md and skills/frontend/SKILL.md itself, in
Python, every single time, regardless of what the model did or didn't do.

Deliberately NOT run here: cargo dylint, the db-integration/e2e/coverage CI
jobs. Those need Docker/Postgres and are CI's job, not a per-cycle local
gate's - see the harness's own CI-failure-log-driven retry loop for that
tier instead. This is why `cargo test` below is scoped to `--lib --bins`,
same as CI's own required Rust job: crates/server/tests/db_queries.rs and
integration.rs use `#[sqlx::test]` with no `#[ignore]` fallback, so a bare
`cargo test --workspace` fails almost every one of those tests outright in
this DB-less checkout, regardless of what changed.

Each command is streamed via procstream.run_streaming rather than captured
silently until it exits - pre-commit (installing hook environments on a
first run) and cargo test/clippy in particular can run for minutes, and
without this a working-but-slow validation pass looks identical to a hang.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path

import procstream

log = logging.getLogger("harness.validate")

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


def _run(cwd: Path, args: list[str], timeout: int) -> ValidationResult:
    step = " ".join(args)
    result = procstream.run_streaming(args, cwd, timeout, log, step)
    if result.timed_out:
        output = f"timed out after {timeout}s:\n{result.output}"
        return ValidationResult(ok=False, step=step, output=output)
    return ValidationResult(ok=result.returncode == 0, step=step, output=result.output)


def run_precommit(cwd: Path, timeout: int = 900) -> ValidationResult:
    return _run(
        cwd, ["uv", "run", "pre-commit", "run", "--all-files", "--show-diff-on-failure"], timeout
    )


def run_rust_checks(cwd: Path, timeout: int = 1800) -> list[ValidationResult]:
    steps = [
        RUST_FMT_ARGS,
        ["cargo", "+nightly", "clippy", "--workspace", "--", "-D", "warnings"],
        # --lib --bins only: crates/server/tests/{db_queries,integration}.rs
        # use #[sqlx::test], which unconditionally needs a live Postgres
        # with DATABASE_URL set - no #[ignore] gate gets applied to skip it
        # otherwise, so every one of the ~200 tests in there fails outright
        # in this disposable, DB-less checkout regardless of what changed.
        # CI's own required "Rust" job scopes its `cargo test` the same way
        # and runs the DB-backed suite as a separate job with its own
        # Postgres service instead - this local gate has no DB, so it
        # defers to that CI job for this tier rather than reporting a false
        # "still broken" no matter what opencode does.
        ["cargo", "test", "--workspace", "--lib", "--bins"],
        ["cargo", "deny", "check"],
    ]
    results = []
    for step in steps:
        result = _run(cwd, step, timeout)
        results.append(result)
        if not result.ok:
            break
    return results


def run_frontend_checks(cwd: Path, timeout: int = 1800) -> list[ValidationResult]:
    frontend_dir = cwd / "frontend"
    steps = []
    # node_modules/ is preserved across cycles (see git_ops._CLEAN_EXCLUDES)
    # so this only actually installs on the first cycle that touches
    # frontend/ - without it, every cycle would fail outright the moment
    # npm/eslint/vite don't exist yet.
    if not (frontend_dir / "node_modules").exists():
        steps.append(["npm", "ci"])
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
