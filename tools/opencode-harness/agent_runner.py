# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Invokes a coding-agent CLI non-interactively to make code edits.

Two backends are supported, selected per-call by the `cli` argument (see
`Config.agent_cli` / `HARNESS_AGENT_CLI` in config.py): `opencode` (the
original, default backend) and `claude` (the Claude Code CLI). Either way the
agent is asked only to edit files - see git_ops.py's module docstring for why
commit/push are never delegated to it.

Both backends are invoked in a fully unattended, auto-approving mode -
opencode's `--auto`, Claude Code's `--dangerously-skip-permissions` - which is
what makes running this harness unattended possible at all; it also means
either one can run arbitrary shell commands on this machine without a human
in the loop. See README.md's Safety section before pointing this at anything
but a disposable checkout.

Output is logged as it arrives via procstream.run_streaming rather than
captured silently until the process exits - see that module's docstring for
why.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from pathlib import Path

import procstream

log = logging.getLogger("harness.agent")

NEVER_COMMIT_INSTRUCTION = (
    "\n\nImportant: do not run `git commit`, `git push`, or stage/commit changes "
    "in any way. Leave your edits as uncommitted working-tree changes. A "
    "separate deterministic process will run the project's validation "
    "commands, commit, and push on your behalf."
)

_MAX_SNIPPET_CHARS = 500

# The agent CLI backends this module knows how to drive - see
# Config.agent_cli/HARNESS_AGENT_CLI.
SUPPORTED_CLIS = ("opencode", "claude")


@dataclass
class AgentResult:
    ok: bool
    output: str


def _truncate(text: str, limit: int = _MAX_SNIPPET_CHARS) -> str:
    text = " ".join(text.split())
    return text if len(text) <= limit else text[:limit] + "..."


def _build_opencode_cmd(prompt: str, cwd: Path, model: str | None) -> list[str]:
    cmd = ["opencode", "run", "--dir", str(cwd), "--format", "json", "--auto"]
    if model:
        cmd += ["--model", model]
    cmd.append(prompt)
    return cmd


def _build_claude_cmd(prompt: str, model: str | None) -> list[str]:
    # --dangerously-skip-permissions is Claude Code's equivalent of opencode's
    # --auto: it bypasses the CLI's permission system entirely instead of
    # prompting (there is no human to answer a prompt in this harness), which
    # is what makes unattended operation possible at all - see this module's
    # own docstring and README.md's Safety section. --verbose is required
    # alongside --output-format stream-json when combined with --print (-p);
    # without it the CLI refuses to stream intermediate events.
    cmd = [
        "claude",
        "--output-format",
        "stream-json",
        "--verbose",
        "--dangerously-skip-permissions",
    ]
    if model:
        cmd += ["--model", model]
    cmd += ["-p", prompt]
    return cmd


def _format_opencode_event(line: str) -> str | None:
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


def _format_claude_event(line: str) -> str | None:
    """Formats one `claude -p --output-format stream-json` event for logging.

    Mirrors _format_opencode_event's job for the other backend's own event
    schema: a `system`/`init` event is pure bookkeeping (suppressed); an
    `assistant` event carries a `message.content` list of text/tool_use
    blocks; a `user` event echoes back `tool_result` blocks; a terminal
    `result` event summarizes cost/turns/duration. Falls back to a truncated
    raw dump for any event shape not accounted for below, since this schema
    is not an officially documented, stability-guaranteed contract either.
    """
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return line[:_MAX_SNIPPET_CHARS]
    if not isinstance(event, dict):
        return str(event)[:_MAX_SNIPPET_CHARS]

    kind = event.get("type")

    if kind == "system":
        return None

    if kind == "assistant":
        message = event.get("message") or {}
        parts = []
        for block in message.get("content") or []:
            if not isinstance(block, dict):
                continue
            btype = block.get("type")
            if btype == "text":
                text = block.get("text") or ""
                if text.strip():
                    parts.append(_truncate(text))
            elif btype == "tool_use":
                name = block.get("name", "?")
                tool_input = block.get("input")
                if tool_input:
                    parts.append(f"tool: {name} - {_truncate(json.dumps(tool_input), 200)}")
                else:
                    parts.append(f"tool: {name}")
        return "; ".join(parts) if parts else None

    if kind == "user":
        # Claude routinely batches a whole round of parallel tool calls'
        # results into one user event as multiple tool_result blocks - collect
        # every one instead of returning after the first, or a failure that
        # isn't first in the batch silently never reaches the harness log.
        message = event.get("message") or {}
        parts = []
        for block in message.get("content") or []:
            if not isinstance(block, dict) or block.get("type") != "tool_result":
                continue
            content = block.get("content")
            text = content if isinstance(content, str) else json.dumps(content)
            label = "tool result FAILED" if block.get("is_error") else "tool result"
            parts.append(f"{label}: {_truncate(text, 300)}")
        return "; ".join(parts) if parts else None

    if kind == "result":
        cost = event.get("total_cost_usd")
        cost_str = f"${cost:.4f}" if isinstance(cost, (int, float)) else "?"
        duration = event.get("duration_ms")
        duration_str = f"{duration}ms" if isinstance(duration, (int, float)) else "?"
        return (
            f"run finished: subtype={event.get('subtype', '?')} cost={cost_str} "
            f"turns={event.get('num_turns', '?')} duration={duration_str}"
        )

    if kind == "error":
        return f"ERROR: {json.dumps(event)[:_MAX_SNIPPET_CHARS]}"

    return json.dumps(event)[:_MAX_SNIPPET_CHARS]


def run_agent(
    prompt: str, cwd: Path, model: str | None, timeout_seconds: int, cli: str = "opencode"
) -> AgentResult:
    """Runs `cli` (`"opencode"` or `"claude"`, see SUPPORTED_CLIS) non-interactively
    with `prompt`, appending NEVER_COMMIT_INSTRUCTION so it always leaves edits
    uncommitted. Raises ValueError for any other `cli` value - callers always
    pass `Config.agent_cli`, which is validated at load time (see config.py),
    so this is a defensive check against a future caller forgetting that.
    """
    full_prompt = prompt + NEVER_COMMIT_INSTRUCTION
    if cli == "opencode":
        cmd = _build_opencode_cmd(full_prompt, cwd, model)
        format_line = _format_opencode_event
    elif cli == "claude":
        cmd = _build_claude_cmd(full_prompt, model)
        format_line = _format_claude_event
    else:
        raise ValueError(f"unsupported agent cli {cli!r} - must be one of {SUPPORTED_CLIS}")

    log.info("invoking %s (model=%s, timeout=%ss)", cli, model or "default", timeout_seconds)
    result = procstream.run_streaming(cmd, cwd, timeout_seconds, log, cli, format_line=format_line)

    if result.timed_out:
        message = f"{cli} timed out after {timeout_seconds}s and was killed:\n{result.output}"
        return AgentResult(ok=False, output=message)
    if result.returncode != 0:
        message = f"{cli} exited {result.returncode}:\n{result.output}"
        return AgentResult(ok=False, output=message)
    log.info("%s run finished (exit 0)", cli)
    return AgentResult(ok=True, output=result.output)
