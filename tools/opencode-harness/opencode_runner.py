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

Output is logged as it arrives rather than captured silently until opencode
exits. A plain line-buffered read isn't enough on its own: opencode can go
quiet for a long stretch (thinking, running a slow tool call) with no line
to log, which looks identical to a hang. The read loop below polls with a
timeout instead of blocking indefinitely, so it can log a heartbeat during
those quiet stretches instead of going silent.
"""

from __future__ import annotations

import json
import logging
import os
import select
import subprocess
import time
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
_HEARTBEAT_INTERVAL_SECONDS = 20
_READ_CHUNK_SIZE = 65536


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


def _log_complete_lines(buf: bytes) -> bytes:
    """Logs every complete line in `buf`, returning the trailing partial line."""
    while b"\n" in buf:
        raw_line, buf = buf.split(b"\n", 1)
        text = raw_line.decode("utf-8", errors="replace").strip()
        if text:
            log.info("opencode: %s", _summarize_event(text))
    return buf


def run_opencode(prompt: str, cwd: Path, model: str | None, timeout_seconds: int) -> OpencodeResult:
    cmd = ["opencode", "run", "--dir", str(cwd), "--format", "json", "--auto"]
    if model:
        cmd += ["--model", model]
    cmd.append(prompt + NEVER_COMMIT_INSTRUCTION)

    log.info("invoking opencode (model=%s, timeout=%ss)", model or "default", timeout_seconds)
    proc = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert proc.stdout is not None
    fd = proc.stdout.fileno()

    deadline = time.monotonic() + timeout_seconds
    last_activity = time.monotonic()
    buf = b""
    chunks: list[bytes] = []
    timed_out = False

    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            timed_out = True
            proc.kill()
            break
        ready, _, _ = select.select([fd], [], [], min(_HEARTBEAT_INTERVAL_SECONDS, remaining))
        if ready:
            chunk = os.read(fd, _READ_CHUNK_SIZE)
            if not chunk:
                break  # EOF: opencode closed its stdout, nothing more will arrive
            chunks.append(chunk)
            buf = _log_complete_lines(buf + chunk)
            last_activity = time.monotonic()
        else:
            log.info(
                "opencode: still running (%ds since last output)",
                int(time.monotonic() - last_activity),
            )

    proc.wait()
    if buf:
        text = buf.decode("utf-8", errors="replace").strip()
        if text:
            log.info("opencode: %s", _summarize_event(text))
        chunks.append(buf)

    output = b"".join(chunks).decode("utf-8", errors="replace")
    if timed_out:
        message = f"opencode timed out after {timeout_seconds}s and was killed:\n{output}"
        return OpencodeResult(ok=False, output=message)
    if proc.returncode != 0:
        return OpencodeResult(ok=False, output=f"opencode exited {proc.returncode}:\n{output}")
    log.info("opencode run finished (exit 0)")
    return OpencodeResult(ok=True, output=output)
