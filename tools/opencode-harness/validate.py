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
tier instead.
"""

from __future__ import annotations

import logging
import subprocess
from dataclasses import dataclass
from pathlib import Path

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
    try:
        proc = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        return ValidationResult(ok=False, step=step, output=f"timed out after {timeout}s: {exc}")
    output = proc.stdout + ("\n" + proc.stderr if proc.stderr else "")
    return ValidationResult(ok=proc.returncode == 0, step=step, output=output)


def run_precommit(cwd: Path, timeout: int = 900) -> ValidationResult:
    return _run(
        cwd, ["uv", "run", "pre-commit", "run", "--all-files", "--show-diff-on-failure"], timeout
    )


def run_rust_checks(cwd: Path, timeout: int = 1800) -> list[ValidationResult]:
    steps = [
        RUST_FMT_ARGS,
        ["cargo", "+nightly", "clippy", "--workspace", "--", "-D", "warnings"],
        ["cargo", "test", "--workspace"],
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
    steps = [
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
