# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Invokes the `opencode` CLI non-interactively to make code edits.

opencode is asked only to edit files - see git_ops.py's module docstring for
why commit/push are never delegated to it. `--auto` auto-approves whatever
permissions opencode's config doesn't explicitly deny, which is what makes
unattended operation possible at all; it also means opencode can run
arbitrary shell commands on this machine without a human in the loop. See
README.md's Safety section before pointing this at anything but a disposable
checkout.
"""

from __future__ import annotations

import logging
import subprocess
from dataclasses import dataclass
from pathlib import Path

log = logging.getLogger("harness.opencode")

NEVER_COMMIT_INSTRUCTION = (
    "\n\nImportant: do not run `git commit`, `git push`, or stage/commit changes "
    "in any way. Leave your edits as uncommitted working-tree changes. A "
    "separate deterministic process will run the project's validation "
    "commands, commit, and push on your behalf."
)


@dataclass
class OpencodeResult:
    ok: bool
    output: str


def run_opencode(prompt: str, cwd: Path, model: str | None, timeout_seconds: int) -> OpencodeResult:
    cmd = ["opencode", "run", "--dir", str(cwd), "--format", "json", "--auto"]
    if model:
        cmd += ["--model", model]
    cmd.append(prompt + NEVER_COMMIT_INSTRUCTION)

    log.info("invoking opencode (model=%s, timeout=%ss)", model or "default", timeout_seconds)
    try:
        proc = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout_seconds)
    except subprocess.TimeoutExpired as exc:
        return OpencodeResult(
            ok=False, output=f"opencode timed out after {timeout_seconds}s: {exc}"
        )

    output = proc.stdout + ("\n" + proc.stderr if proc.stderr else "")
    if proc.returncode != 0:
        return OpencodeResult(ok=False, output=f"opencode exited {proc.returncode}:\n{output}")
    return OpencodeResult(ok=True, output=output)
