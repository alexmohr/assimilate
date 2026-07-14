# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Runs a subprocess with output streamed line-by-line and a heartbeat log
during quiet stretches, instead of the silent capture-until-exit that makes
a slow-but-working command indistinguishable from a hung one.

Shared by opencode_runner.py (the opencode call itself) and validate.py
(pre-commit, cargo fmt/clippy/test/deny, npm format/lint/build/test) - all of
these can run for minutes with a plain `subprocess.run(capture_output=True)`,
which is exactly the silence this exists to remove.
"""

from __future__ import annotations

import contextlib
import logging
import os
import select
import signal
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

_HEARTBEAT_INTERVAL_SECONDS = 20
_READ_CHUNK_SIZE = 65536


@dataclass
class StreamResult:
    returncode: int
    output: str
    timed_out: bool


def run_streaming(
    cmd: list[str],
    cwd: Path,
    timeout_seconds: int,
    log: logging.Logger,
    label: str,
    format_line: Callable[[str], str | None] | None = None,
    env: dict[str, str] | None = None,
) -> StreamResult:
    """Runs `cmd`, logging each output line (via `format_line` if given, else
    verbatim) as it arrives, plus a heartbeat every ~20s of silence. `label`
    identifies the command in heartbeat/log lines. Combines stdout+stderr -
    only suitable for callers that treat output as human-readable log text,
    not ones that parse stdout programmatically. `env` replaces the child's
    environment entirely if given (None inherits this process's environment,
    same as subprocess's own default) - callers that only need to add/override
    one variable should pass `{**os.environ, "VAR": value}`.
    """
    # start_new_session puts the child in its own process group so a timeout
    # kill can take out every process it spawned (e.g. opencode's bash tool
    # calling pre-commit/git), not just the top-level one - killing only the
    # parent leaves grandchildren running orphaned, still holding locks or
    # writing files, which then breaks every subsequent command in this repo.
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        env=env,
    )
    assert proc.stdout is not None
    fd = proc.stdout.fileno()

    deadline = time.monotonic() + timeout_seconds
    last_activity = time.monotonic()
    buf = b""
    chunks: list[bytes] = []
    timed_out = False

    def flush_lines(data: bytes) -> bytes:
        while b"\n" in data:
            raw_line, data = data.split(b"\n", 1)
            text = raw_line.decode("utf-8", errors="replace").strip()
            if not text:
                continue
            formatted = format_line(text) if format_line else text
            if formatted:
                log.info("%s: %s", label, formatted)
        return data

    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            timed_out = True
            with contextlib.suppress(ProcessLookupError):
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            break
        ready, _, _ = select.select([fd], [], [], min(_HEARTBEAT_INTERVAL_SECONDS, remaining))
        if ready:
            chunk = os.read(fd, _READ_CHUNK_SIZE)
            if not chunk:
                break  # EOF: process closed its stdout, nothing more will arrive
            chunks.append(chunk)
            buf = flush_lines(buf + chunk)
            last_activity = time.monotonic()
        else:
            log.info(
                "%s: still running (%ds since last output)",
                label,
                int(time.monotonic() - last_activity),
            )

    proc.wait()
    if buf:
        # buf's bytes are already in chunks (each os.read() result is
        # appended there in full, unflushed tail included) - it's only
        # tracked separately to know where the last incomplete line starts.
        # Re-appending it here would duplicate those trailing bytes in
        # `output` whenever the process's last write didn't end in "\n".
        text = buf.decode("utf-8", errors="replace").strip()
        if text:
            formatted = format_line(text) if format_line else text
            if formatted:
                log.info("%s: %s", label, formatted)

    output = b"".join(chunks).decode("utf-8", errors="replace")
    return StreamResult(
        returncode=-1 if timed_out else proc.returncode,
        output=output,
        timed_out=timed_out,
    )
