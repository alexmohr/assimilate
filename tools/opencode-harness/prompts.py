# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Builds the fix/implementation prompts handed to opencode.

All of the actual diagnosis (what's failing, why) happens in gh.py before
this is called - these functions only format already-gathered, deterministic
facts into an instruction. opencode is never asked to go figure out why CI is
red on its own; it's handed the failing job's log text directly.
"""

from __future__ import annotations

from gh import PrDetail

COMMON_RULES = (
    "Follow AGENTS.md and the relevant skills/*/SKILL.md files in this repo "
    "exactly. Do not weaken, delete, or skip any test to make it pass. Do not "
    "add any #[allow(...)] or deny.toml ignore entry. Do not add or remove any "
    "GitHub label yourself."
)


def build_pr_fix_prompt(
    pr: PrDetail, ci_logs: str | None, review_comments: str | None, precheck_notes: str | None
) -> str:
    sections = [
        f"You are fixing pull request #{pr.number} ('{pr.title}') in this "
        "repository so it becomes mergeable.",
        COMMON_RULES,
    ]
    if pr.merge_conflict:
        sections.append(
            "The branch has already been rebased onto the base branch by a separate "
            "process; if you see leftover conflict markers, resolve them so the code "
            "is correct and coherent, preserving the intent of both sides."
        )
    if ci_logs:
        sections.append(
            f"CI is failing. Here is the log output from the failing job(s):\n\n{ci_logs}"
        )
    if review_comments:
        sections.append(
            "A reviewer requested changes. Here are the review comments to "
            f"address:\n\n{review_comments}"
        )
    if precheck_notes:
        sections.append(
            "An automated pre-flight check (coverage or duplicate-code) failed. "
            f"Here is its report:\n\n{precheck_notes}"
        )
    sections.append(
        "Make the minimal correct changes needed to resolve everything above. "
        "Do not refactor unrelated code."
    )
    return "\n\n".join(sections)


def build_retry_prompt(
    previous_prompt: str, failure_step: str, failure_output: str, max_output_chars: int = 6000
) -> str:
    output = failure_output
    if len(output) > max_output_chars:
        output = output[-max_output_chars:]
    return (
        f"{previous_prompt}\n\n---\n\nYour previous attempt did not pass validation. "
        f"The failing step was `{failure_step}`. Its output:\n\n{output}\n\n"
        "Fix the underlying issue - do not disable or skip whatever check failed."
    )


def build_issue_prompt(number: int, title: str, body: str) -> str:
    return "\n\n".join(
        [
            f"Implement a fix for issue #{number} in this repository: '{title}'.",
            f"Issue description:\n\n{body or '(no description provided)'}",
            COMMON_RULES,
            "Include tests for any new behavior, and update documentation if this "
            "is a user-facing change, per skills/testing/SKILL.md and "
            "skills/documentation/SKILL.md.",
        ]
    )
