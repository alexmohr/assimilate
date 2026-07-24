# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Per-task model routing: a single fixed `--model` for every job the harness
does (fixing CI, implementing a feature, resolving a merge conflict, ...) was
never a good fit - a cheap/fast model that's fine for a mechanical CI fix is a
poor choice for a large refactor or a security-sensitive review, and a model
strong enough for those is wasteful for a one-line boilerplate fix.

Instead, before doing any real work, the harness hands a short description of
the task to a cheap, fast classifier model (see `Config.router_model`) and
asks it to pick the right model for the *actual* job from `ROUTING_TABLE`
below. The classifier's own JSON answer is validated against that table
before ever being handed to `opencode --model` - a hallucinated or malformed
model string would otherwise surface as an opaque `UnknownError` from
opencode itself several minutes into a run (see tools/opencode-harness/
README.md's provider-prefix note), not as a clear routing bug.

`Config.opencode_model` (the CLI-only `--model` flag) is still an escape
hatch: passing it pins every task to that one model and skips classification
entirely, the same as before this module existed - useful for testing a
specific model or working around a routing table that doesn't fit your setup.
"""

from __future__ import annotations

import json
import logging
import re
from dataclasses import dataclass

import git_ops
import opencode_runner
from config import Config

log = logging.getLogger("harness.router")

# Absolute fallback whenever classification can't produce a usable answer
# (the classifier run itself failed, timed out, or its output couldn't be
# parsed into a model this table actually recognizes) - "Fix failing PRs /
# CI failures" and "Mass automated PR repair bot" both land here anyway,
# and that's what this harness spends most of its time doing.
DEFAULT_FALLBACK_MODEL = "kimi-k2.7-code"

_ROUTER_TIMEOUT_INSTRUCTION = (
    "\n\nDo not edit, create, or delete any files - only read what you need to "
    "judge scope and complexity, then answer. A separate process does the "
    "actual fix once you've classified it."
)


@dataclass(frozen=True)
class ModelRoute:
    label: str
    primary: str
    alternative: str
    notes: str


# Mirrors the task/model table maintained alongside this harness - keep the
# two in sync if either changes. Keys are the `task_type` values the
# classifier is asked to choose from; order matches the source table so a
# human diffing the two can follow along.
ROUTING_TABLE: dict[str, ModelRoute] = {
    "bug_fix": ModelRoute(
        "Fix failing PRs / CI failures",
        "kimi-k2.7-code",
        "glm-5.2",
        "Kimi is a good default for code repair. Use GLM when the failure requires "
        "deeper architecture reasoning.",
    ),
    "feature": ModelRoute(
        "Implement new features",
        "kimi-k2.7-code",
        "glm-5.2",
        "Kimi for most coding; GLM for large cross-module features.",
    ),
    "refactor": ModelRoute(
        "Large refactors",
        "glm-5.2",
        "kimi-k2.7-code",
        "Better when many files and dependencies are involved.",
    ),
    "code_review": ModelRoute(
        "Code review",
        "glm-5.2",
        "kimi-k2.7-code",
        "GLM as reviewer, Kimi as implementer.",
    ),
    "debug": ModelRoute(
        "Debug mysterious bugs",
        "glm-5.2",
        "kimi-k2.7-code",
        "Use the stronger reasoning model first.",
    ),
    "write_tests": ModelRoute(
        "Write tests",
        "kimi-k2.7-code",
        "qwen3.7-plus",
        "Good balance of speed and correctness.",
    ),
    "unit_test_fix": ModelRoute(
        "Unit test fixes",
        "kimi-k2.7-code",
        "deepseek-v4-pro",
        "Usually straightforward.",
    ),
    "documentation": ModelRoute(
        "Documentation generation",
        "qwen3.7-plus",
        "kimi-k2.7-code",
        "Saves your stronger models for harder tasks.",
    ),
    "boilerplate": ModelRoute(
        "Simple boilerplate code",
        "qwen3.7-plus",
        "deepseek-v4-flash",
        "High quota, lower importance.",
    ),
    "dependency_upgrade": ModelRoute(
        "Dependency upgrades",
        "glm-5.2",
        "kimi-k2.7-code",
        "Needs awareness of ecosystem changes.",
    ),
    "security_review": ModelRoute(
        "Security review",
        "glm-5.2",
        "kimi-k2.7-code",
        "Prefer deeper reasoning.",
    ),
    "architecture": ModelRoute(
        "Architecture design",
        "glm-5.2",
        "grok-4.5",
        "Planning > raw coding speed.",
    ),
    "repo_exploration": ModelRoute(
        "Repo exploration / onboarding",
        "glm-5.2",
        "kimi-k2.7-code",
        "Long context and reasoning matter.",
    ),
    "small_bug_fix": ModelRoute(
        "Small bug fixes",
        "kimi-k2.7-code",
        "qwen3.7-plus",
        "Fast turnaround.",
    ),
    "mass_pr_repair": ModelRoute(
        "Mass automated PR repair bot",
        "kimi-k2.7-code",
        "qwen3.7-plus",
        "Best quota/capability ratio.",
    ),
    "cheap_background": ModelRoute(
        "Cheap background agent tasks",
        "deepseek-v4-flash",
        "mimo-v2.5",
        "Use only for low-risk work.",
    ),
}

# Every model id this harness is allowed to hand to `opencode --model` as a
# *routed* choice - anything the classifier returns outside this set is
# treated as a hallucination, never passed through as-is (see _resolve_model).
_VALID_MODELS: frozenset[str] = frozenset(
    {route.primary for route in ROUTING_TABLE.values()}
    | {route.alternative for route in ROUTING_TABLE.values()}
    | {DEFAULT_FALLBACK_MODEL}
)


@dataclass(frozen=True)
class ModelDecision:
    model: str
    # None when routing was skipped (an explicit --model override) or the
    # classifier run itself didn't produce a usable answer - present whenever
    # a real classification happened, for logging/diagnostics.
    classification: dict | None


def _routing_table_markdown() -> str:
    lines = ["| task_type | recommended_model | alternative_model | notes |", "|---|---|---|---|"]
    for key, route in ROUTING_TABLE.items():
        lines.append(
            f"| {key} ({route.label}) | {route.primary} | {route.alternative} | {route.notes} |"
        )
    return "\n".join(lines)


def _build_classifier_prompt(task_context: str) -> str:
    return (
        "You are a fast, cheap task classifier for an automated coding pipeline "
        "that delegates the actual work to one of several models with different "
        "cost/capability tradeoffs.\n\n"
        f"Model routing table:\n{_routing_table_markdown()}\n\n"
        f"Task to classify:\n\n{task_context}\n\n"
        "Respond with ONLY a single JSON object - no prose, no markdown code "
        "fences - in exactly this shape:\n"
        "{\n"
        '  "task_type": "<one of the task_type values from the table above>",\n'
        '  "complexity": "low|medium|high",\n'
        '  "files_affected": "single|few|multiple",\n'
        '  "recommended_model": "<the recommended_model or alternative_model of '
        "the chosen task_type row - pick the alternative when its note applies "
        'to this task>",\n'
        '  "reason": "<one sentence>"\n'
        "}" + _ROUTER_TIMEOUT_INSTRUCTION
    )


def _extract_assistant_text(raw_output: str) -> str:
    """Pulls the model's own text out of `opencode run --format json`'s
    newline-delimited event stream - see opencode_runner._format_event for the
    same event shapes handled there for logging. Falls back to the raw output
    untouched if nothing parses (e.g. a completely different output shape),
    so `_find_json_object` still gets a chance to find something.
    """
    parts = []
    for line in raw_output.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and event.get("type") == "text":
            text = (event.get("part") or {}).get("text") or ""
            if text:
                parts.append(text)
    return "\n".join(parts) if parts else raw_output


def _find_json_object(text: str) -> str | None:
    """First balanced `{...}` object in `text` - a simple depth counter is
    more robust than a greedy regex here, since the classifier's answer can
    legitimately contain nested braces (e.g. inside `reason`).
    """
    start = text.find("{")
    if start == -1:
        return None
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
    return None


_TASK_TYPE_NORMALIZE_RE = re.compile(r"[\s-]+")


def _normalize_task_type(raw: str) -> str:
    return _TASK_TYPE_NORMALIZE_RE.sub("_", raw.strip().lower())


def _resolve_model(classification: dict) -> str:
    """The classifier is asked to name a model directly (see the prompt) so it
    can pick the alternative when its own note applies - but that string is
    never trusted blindly: only a model id that actually appears in
    ROUTING_TABLE is passed through. Falls back to the chosen task_type's own
    primary model, then to DEFAULT_FALLBACK_MODEL, if the model string itself
    doesn't check out (a hallucinated id, a provider-prefixed variant this
    table doesn't know about, or a missing/malformed field).
    """
    candidate = classification.get("recommended_model")
    if isinstance(candidate, str) and candidate.strip() in _VALID_MODELS:
        return candidate.strip()

    task_type = classification.get("task_type")
    if isinstance(task_type, str):
        route = ROUTING_TABLE.get(_normalize_task_type(task_type))
        if route is not None:
            return route.primary

    return DEFAULT_FALLBACK_MODEL


def _parse_classification(raw_output: str) -> dict | None:
    text = _extract_assistant_text(raw_output)
    blob = _find_json_object(text)
    if blob is None:
        return None
    try:
        data = json.loads(blob)
    except json.JSONDecodeError:
        return None
    return data if isinstance(data, dict) else None


def route(cfg: Config, task_context: str, task_label: str) -> ModelDecision:
    """Classifies `task_context` with `cfg.router_model` and returns the model
    to actually use for it. `task_label` is only for logging (e.g. "PR #123").

    Skips classification entirely - returning `cfg.opencode_model` unchanged -
    when a human pinned a single model via `--model`. Never raises: any
    failure in the classifier run itself (opencode error, timeout, unparsable
    output) is logged and falls back to DEFAULT_FALLBACK_MODEL rather than
    blocking the actual fix over a routing decision.
    """
    if cfg.opencode_model:
        return ModelDecision(model=cfg.opencode_model, classification=None)

    prompt = _build_classifier_prompt(task_context)
    result = opencode_runner.run_opencode(
        prompt, cfg.repo_dir, cfg.router_model, cfg.router_timeout_seconds
    )
    # Best-effort safety net: the classifier is told not to touch files, but
    # nothing stops a cheap/small model from ignoring that - discard whatever
    # it may have left behind so the actual fix that follows starts clean,
    # the same way a failed opencode run is cleaned up elsewhere in this
    # codebase (see handle_pr_fix's own discard_uncommitted_changes calls).
    git_ops.discard_uncommitted_changes(cfg.repo_dir)

    if not result.ok:
        log.warning(
            "model router: classifier run failed for %s, falling back to %s: %s",
            task_label,
            DEFAULT_FALLBACK_MODEL,
            result.output[:300],
        )
        return ModelDecision(model=DEFAULT_FALLBACK_MODEL, classification=None)

    classification = _parse_classification(result.output)
    if classification is None:
        log.warning(
            "model router: could not parse a classification for %s, falling back to %s",
            task_label,
            DEFAULT_FALLBACK_MODEL,
        )
        return ModelDecision(model=DEFAULT_FALLBACK_MODEL, classification=None)

    model = _resolve_model(classification)
    log.info(
        "model router: %s -> %s (task_type=%s complexity=%s files_affected=%s reason=%s)",
        task_label,
        model,
        classification.get("task_type"),
        classification.get("complexity"),
        classification.get("files_affected"),
        classification.get("reason"),
    )
    return ModelDecision(model=model, classification=classification)
