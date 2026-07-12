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

_HEARTBEAT_INTERVAL_SECONDS = 20
_READ_CHUNK_SIZE = 65536
_MAX_SNIPPET_CHARS = 500


@dataclass
class OpencodeResult:
    ok: bool
    output: str


def _truncate(text: str, limit: int = _MAX_SNIPPET_CHARS) -> str:
    text = " ".join(text.split())
    return text if len(text) <= limit else text[:limit] + "..."


def _format_event(line: str) -> str | None:
    """Formats one `opencode run --format json` event for logging.

    Returns None to suppress an event entirely (e.g. step_start, or a
    step_finish that isn't the final one) - these are pure bookkeeping with
    no assistant-visible content, and printing them is exactly the raw-JSON
    noise this exists to avoid. Falls back to a truncated raw dump for any
    event shape not accounted for below, since this schema is not an
    officially documented, stability-guaranteed contract.
    """
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return line[:_MAX_SNIPPET_CHARS]
    if not isinstance(event, dict):
        return str(event)[:_MAX_SNIPPET_CHARS]

    kind = event.get("type")
    part = event.get("part") or {}

    if kind == "step_start":
        return None

    if kind == "text":
        text = part.get("text") or ""
        return _truncate(text) if text.strip() else None

    if kind == "tool_use":
        tool = part.get("tool", "?")
        state = part.get("state") or {}
        status = state.get("status")
        title = state.get("title") or ""
        if status == "completed":
            return f"tool: {tool}" + (f" - {_truncate(title, 200)}" if title else "")
        if status == "error":
            output = _truncate(str(state.get("output") or ""), 300)
            return f"tool: {tool} FAILED" + (f" - {output}" if output else "")
        return None  # still running/pending: nothing to report yet

    if kind == "step_finish":
        if part.get("reason") != "stop":
            return None  # just continuing to another step
        tokens = part.get("tokens") or {}
        cost = part.get("cost")
        cost_str = f"${cost:.4f}" if isinstance(cost, (int, float)) else "?"
        return (
            f"step finished: cost={cost_str} tokens(in={tokens.get('input', '?')}, "
            f"out={tokens.get('output', '?')}, reasoning={tokens.get('reasoning', '?')})"
        )

    if kind == "error":
        error = event.get("error") or {}
        message = (error.get("data") or {}).get("message", "")
        return f"ERROR: {error.get('name', 'unknown')}: {message}"

    return json.dumps(event)[:_MAX_SNIPPET_CHARS]


def _log_complete_lines(buf: bytes) -> bytes:
    """Logs every complete line in `buf`, returning the trailing partial line."""
    while b"\n" in buf:
        raw_line, buf = buf.split(b"\n", 1)
        text = raw_line.decode("utf-8", errors="replace").strip()
        if not text:
            continue
        formatted = _format_event(text)
        if formatted:
            log.info("opencode: %s", formatted)
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
            formatted = _format_event(text)
            if formatted:
                log.info("opencode: %s", formatted)
        chunks.append(buf)

    output = b"".join(chunks).decode("utf-8", errors="replace")
    if timed_out:
        message = f"opencode timed out after {timeout_seconds}s and was killed:\n{output}"
        return OpencodeResult(ok=False, output=message)
    if proc.returncode != 0:
        return OpencodeResult(ok=False, output=f"opencode exited {proc.returncode}:\n{output}")
    log.info("opencode run finished (exit 0)")
    return OpencodeResult(ok=True, output=output)
