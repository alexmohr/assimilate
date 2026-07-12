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

Output is streamed and logged line by line as opencode runs (rather than
captured silently until it exits) - a 30-minute call with no visibility into
whether it's stuck or working is not an acceptable operator experience.
"""

from __future__ import annotations

import json
import logging
import subprocess
import threading
from dataclasses import dataclass
from pathlib import Path

log = logging.getLogger("harness.opencode")

NEVER_COMMIT_INSTRUCTION = (
    "\n\nImportant: do not run `git commit`, `git push`, or stage/commit changes "
    "in any way. Leave your edits as uncommitted working-tree changes. A "
    "separate deterministic process will run the project's validation "
    "commands, commit, and push on your behalf."
)

_SNIPPET_KEYS = ("text", "message", "content", "summary")


@dataclass
class OpencodeResult:
    ok: bool
    output: str


def _summarize_event(line: str) -> str:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return line[:500]
    if not isinstance(event, dict):
        return str(event)[:500]
    kind = event.get("type") or event.get("event") or ""
    for key in _SNIPPET_KEYS:
        value = event.get(key)
        if isinstance(value, str) and value.strip():
            snippet = " ".join(value.split())
            if len(snippet) > 400:
                snippet = snippet[:400] + "..."
            return f"{kind}: {snippet}" if kind else snippet
    tool = event.get("tool") or event.get("name")
    if tool:
        return f"{kind or 'tool'}: {tool}"
    return json.dumps(event)[:400]


def run_opencode(prompt: str, cwd: Path, model: str | None, timeout_seconds: int) -> OpencodeResult:
    cmd = ["opencode", "run", "--dir", str(cwd), "--format", "json", "--auto"]
    if model:
        cmd += ["--model", model]
    cmd.append(prompt + NEVER_COMMIT_INSTRUCTION)

    log.info("invoking opencode (model=%s, timeout=%ss)", model or "default", timeout_seconds)
    proc = subprocess.Popen(
        cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1
    )

    timed_out = threading.Event()
    timer = threading.Timer(timeout_seconds, lambda: (timed_out.set(), proc.kill()))
    timer.start()

    output_lines: list[str] = []
    try:
        assert proc.stdout is not None
        for raw_line in proc.stdout:
            output_lines.append(raw_line)
            line = raw_line.rstrip("\n")
            if line:
                log.info("opencode: %s", _summarize_event(line))
    finally:
        timer.cancel()
    proc.wait()

    output = "".join(output_lines)
    if timed_out.is_set():
        message = f"opencode timed out after {timeout_seconds}s and was killed:\n{output}"
        return OpencodeResult(ok=False, output=message)
    if proc.returncode != 0:
        return OpencodeResult(ok=False, output=f"opencode exited {proc.returncode}:\n{output}")
    log.info("opencode run finished (exit 0)")
    return OpencodeResult(ok=True, output=output)
