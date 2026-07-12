#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""opencode-harness: a deterministic supervisor around opencode's full-auto mode.

Priority order, checked every poll cycle:

1. Work the oldest open pull request that currently has something fixable
   (`ci failing`, `merge conflict`, `precheck failed`, or `changes requested`
   - see gh.py). Fetch the concrete failure content (CI logs, review
   comments, coverage/duplicate-code bot comments) in plain Python, hand it
   to opencode as a fix prompt, then run this repo's own validation commands
   (pre-commit, and the exact skills/rust and skills/frontend checklists)
   before committing and pushing. Never touch the repo's own status labels -
   .github/workflows/pr-status-labels.yml owns those end to end and
   re-evaluates them automatically on every push.
2. If a PR keeps hitting the same problem after several push attempts, stop
   touching it (`opencode-harness-stuck` label + a comment) rather than
   burning cycles or pushing something worse.
3. Only once there are zero open PRs at all, pick the newest open issue and
   implement it on a new branch, then open a PR - which flows back into
   step 1 on the next cycle.

See README.md for setup, required env vars, and the safety notes around
opencode's `--auto` flag.
"""

from __future__ import annotations

import argparse
import hashlib
import logging
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gh
import git_ops
import opencode_runner
import prompts
import validate
from config import Config
from gh import PrDetail, PrSummary
from state import HarnessState

log = logging.getLogger("harness")


def setup_logging(log_file: Path | None) -> None:
    handlers: list[logging.Handler] = [logging.StreamHandler(sys.stdout)]
    if log_file is not None:
        log_file.parent.mkdir(parents=True, exist_ok=True)
        handlers.append(logging.FileHandler(log_file))
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)-7s %(name)s: %(message)s",
        handlers=handlers,
    )


def _sanitize_subject(text: str, max_len: int = 60) -> str:
    text = " ".join((text or "").split())
    text = text.replace(":", " -")
    if len(text) > max_len:
        text = text[: max_len - 1].rstrip() + "..."
    return text or "automated change"


def _fingerprint(
    pr: PrDetail, ci_logs: str | None, review_comments: str | None, precheck_notes: str | None
) -> str:
    parts = [
        str(pr.ci_failing),
        str(pr.merge_conflict),
        str(pr.coverage_failed),
        str(pr.duplicate_code),
        str(pr.changes_requested),
        ci_logs or "",
        review_comments or "",
        precheck_notes or "",
    ]
    return hashlib.sha256("\x00".join(parts).encode()).hexdigest()


def run_fix_and_validate(cfg: Config, prompt: str) -> tuple[bool, str]:
    """Runs opencode, then this repo's own validation commands, retrying with
    the concrete failure fed back in - never trusting the model's say-so."""
    current_prompt = prompt
    for attempt in range(1, cfg.max_local_validation_attempts + 1):
        result = opencode_runner.run_opencode(
            current_prompt, cfg.repo_dir, cfg.opencode_model, cfg.opencode_timeout_seconds
        )
        if not result.ok:
            log.info(
                "opencode run failed (attempt %d/%d): %s",
                attempt,
                cfg.max_local_validation_attempts,
                result.output[:500],
            )
            current_prompt = prompts.build_retry_prompt(
                current_prompt, "opencode run", result.output
            )
            continue
        if not git_ops.has_uncommitted_changes(cfg.repo_dir):
            return False, "opencode made no changes"
        changed = git_ops.changed_files(cfg.repo_dir)
        validation = validate.run_all(cfg.repo_dir, changed)
        if validation.ok:
            return True, "validated"
        log.info(
            "validation step '%s' failed (attempt %d/%d)",
            validation.step,
            attempt,
            cfg.max_local_validation_attempts,
        )
        current_prompt = prompts.build_retry_prompt(
            current_prompt, validation.step, validation.output
        )
    return False, f"validation still failing after {cfg.max_local_validation_attempts} attempts"


def _resolve_conflicts(cfg: Config) -> bool:
    ok, status = git_ops.rebase_onto(cfg.repo_dir, cfg.base_branch)
    if ok:
        return True
    prompt = (
        f"Resolve the git rebase conflicts in this repository (rebasing onto "
        f"{cfg.base_branch}).\n\n`git status` output:\n\n{status}\n\n" + prompts.COMMON_RULES
    )
    result = opencode_runner.run_opencode(
        prompt, cfg.repo_dir, cfg.opencode_model, cfg.opencode_timeout_seconds
    )
    if not result.ok:
        git_ops.abort_rebase(cfg.repo_dir)
        return False
    ok, _ = git_ops.continue_rebase(cfg.repo_dir)
    if not ok:
        git_ops.abort_rebase(cfg.repo_dir)
        return False
    return True


def _commit_message_for(pr: PrDetail, cfg: Config) -> str:
    if pr.merge_conflict:
        return f"fix: rebase onto {cfg.base_branch}"
    if pr.changes_requested:
        return "fix: address review feedback"
    if pr.ci_failing:
        return "fix: resolve CI failures"
    if pr.coverage_failed or pr.duplicate_code:
        return "fix: address pre-flight check findings"
    return "fix: address outstanding PR feedback"


def _mark_stuck(cfg: Config, pr: PrDetail, reason: str) -> None:
    if cfg.dry_run:
        log.info("[dry-run] would mark PR #%d stuck: %s", pr.number, reason)
        return
    gh.add_label(cfg.repo, pr.number, cfg.stuck_label)
    gh.comment(
        cfg.repo,
        pr.number,
        f"opencode-harness: giving up on this PR for now - {reason}. "
        f"Marked `{cfg.stuck_label}`; push a new commit or remove the label "
        "to have the harness retry.",
    )
    log.warning("PR #%d marked stuck: %s", pr.number, reason)


def handle_pr_fix(cfg: Config, state: HarnessState, pr: PrDetail) -> bool:
    """Attempts to fix `pr`. Returns True only if a fix was actually pushed -
    this is what "solved problems" counts for --max-solved."""
    log.info(
        "PR #%d needs a fix: ci_failing=%s merge_conflict=%s coverage_failed=%s "
        "duplicate_code=%s changes_requested=%s",
        pr.number,
        pr.ci_failing,
        pr.merge_conflict,
        pr.coverage_failed,
        pr.duplicate_code,
        pr.changes_requested,
    )

    ci_logs = gh.get_failing_check_logs(cfg.repo, pr.number) if pr.ci_failing else None
    review_comments = gh.get_review_comments(cfg.repo, pr.number) if pr.changes_requested else None
    precheck_notes = (
        gh.get_bot_comments(cfg.repo, pr.number)
        if (pr.coverage_failed or pr.duplicate_code)
        else None
    )

    fingerprint = _fingerprint(pr, ci_logs, review_comments, precheck_notes)
    head_sha = gh.get_pr_head_sha(cfg.repo, pr.number)
    attempts = state.record_attempt(pr.number, fingerprint, head_sha)
    if attempts > cfg.max_stuck_cycles:
        _mark_stuck(cfg, pr, f"the same problem has persisted across {attempts - 1} attempts")
        return False

    if cfg.dry_run:
        log.info(
            "[dry-run] would fix PR #%d now (attempt %d/%d)",
            pr.number,
            attempts,
            cfg.max_stuck_cycles,
        )
        return False

    git_ops.checkout_branch_at_remote(cfg.repo_dir, pr.head_ref_name)

    if pr.merge_conflict and not _resolve_conflicts(cfg):
        _mark_stuck(cfg, pr, "could not resolve merge conflicts")
        return False

    prompt = prompts.build_pr_fix_prompt(pr, ci_logs, review_comments, precheck_notes)
    ok, message = run_fix_and_validate(cfg, prompt)
    if not ok:
        log.warning("PR #%d: did not converge this cycle (%s)", pr.number, message)
        return False

    committed = git_ops.commit(cfg.repo_dir, _commit_message_for(pr, cfg))
    if not committed:
        log.warning("PR #%d: opencode made no net changes; nothing to push", pr.number)
        return False
    git_ops.push(cfg.repo_dir, pr.head_ref_name, force_with_lease=pr.merge_conflict)
    log.info("PR #%d: pushed a fix, letting CI/review automation re-evaluate", pr.number)
    return True


def process_prs(cfg: Config, state: HarnessState, prs: list[PrSummary]) -> bool:
    """Handles at most one actionable PR. Returns True if a fix was pushed."""
    for summary in prs:
        if cfg.ignore_label in summary.labels:
            continue

        detail = gh.get_pr(cfg.repo, summary.number)

        if detail.state == "MERGED":
            state.clear_pr(summary.number)
            log.info("PR #%d merged", summary.number)
            continue
        if detail.state == "CLOSED":
            state.clear_pr(summary.number)
            log.info("PR #%d closed without merging, skipping", summary.number)
            continue

        if cfg.stuck_label in detail.labels:
            head_sha = gh.get_pr_head_sha(cfg.repo, summary.number)
            recorded = state.pr_attempts.get(str(summary.number))
            if recorded is not None and recorded.last_head_sha == head_sha:
                log.info("PR #%d still stuck (no new commits), skipping", summary.number)
                continue
            log.info(
                "PR #%d has new commits since being marked stuck; clearing and retrying",
                summary.number,
            )
            gh.remove_label(cfg.repo, summary.number, cfg.stuck_label)

        if not detail.needs_fix:
            log.info("PR #%d: nothing actionable (labels=%s)", summary.number, detail.labels)
            continue

        return handle_pr_fix(cfg, state, detail)

    return False


def process_issues(cfg: Config, state: HarnessState) -> bool:
    """Implements the newest actionable open issue. Returns True if a PR was opened."""
    issues = gh.list_open_issues(cfg.repo)
    candidates = [
        i
        for i in issues
        if cfg.ignore_label not in i["labels"] and cfg.stuck_label not in i["labels"]
    ]
    if not candidates:
        log.info("no open PRs and no actionable open issues; idle this cycle")
        return False

    issue = candidates[0]
    number = issue["number"]
    log.info("no open PRs; picking up newest open issue #%d: %s", number, issue["title"])

    if cfg.dry_run:
        log.info("[dry-run] would implement issue #%d now", number)
        return False

    branch = f"opencode/issue-{number}"
    git_ops.checkout_new_branch_from_base(cfg.repo_dir, branch, cfg.base_branch)

    prompt = prompts.build_issue_prompt(number, issue["title"], issue.get("body") or "")
    ok, message = run_fix_and_validate(cfg, prompt)
    state.mark_issue_started(number)
    if not ok:
        gh.comment(
            cfg.repo,
            number,
            f"opencode-harness: attempted this issue but could not produce a change that passes "
            f"local validation ({message}). Leaving it for a human or a future run.",
            issue=True,
        )
        log.warning("issue #%d: did not converge (%s)", number, message)
        return False

    committed = git_ops.commit(cfg.repo_dir, f"fix: {_sanitize_subject(issue['title'])}")
    if not committed:
        log.warning("issue #%d: opencode made no changes", number)
        return False
    git_ops.push(cfg.repo_dir, branch, force_with_lease=True)
    pr_url = gh.create_pr(
        cfg.repo,
        branch,
        cfg.base_branch,
        f"Fix #{number}: {issue['title']}",
        f"Closes #{number}\n\nImplemented automatically by opencode-harness.",
    )
    log.info("issue #%d: opened %s", number, pr_url)
    return True


def run_once(cfg: Config, state: HarnessState) -> bool:
    """Runs a single cycle. Returns True if it solved a problem (pushed a PR
    fix, or implemented an issue into a new PR) - see --max-solved."""
    prs = gh.list_open_prs(cfg.repo)
    if prs:
        did_work = process_prs(cfg, state, prs)
        if not did_work:
            log.info("%d open PR(s), none actionable right now", len(prs))
        return did_work
    return process_issues(cfg, state)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--once",
        action="store_true",
        help="run a single cycle and exit (also settable via HARNESS_ONCE=1)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="log what would happen without invoking opencode or pushing",
    )
    parser.add_argument(
        "--model",
        default=None,
        help="opencode model, e.g. deepseek/deepseek-v4-flash (defaults to opencode's own default)",
    )
    parser.add_argument(
        "--max-solved",
        type=int,
        default=None,
        metavar="N",
        help=(
            "stop after successfully solving N problems (a PR fix pushed, or an "
            "issue implemented into a new PR) - also settable via HARNESS_MAX_SOLVED"
        ),
    )
    args = parser.parse_args()

    cfg = Config.from_env()
    overrides: dict[str, object] = {}
    if args.once:
        overrides["once"] = True
    if args.dry_run:
        overrides["dry_run"] = True
    if args.model:
        overrides["opencode_model"] = args.model
    if args.max_solved is not None:
        overrides["max_solved"] = args.max_solved
    if overrides:
        cfg = Config(**{**cfg.__dict__, **overrides})

    setup_logging(cfg.log_file)
    log.info("opencode-harness starting: %s", cfg.summary())

    state = HarnessState.load(cfg.state_file)
    solved_count = 0

    while True:
        try:
            if run_once(cfg, state):
                solved_count += 1
                if cfg.max_solved is not None and solved_count >= cfg.max_solved:
                    log.info("solved %d problem(s), reached --max-solved; stopping", solved_count)
                    return 0
        except Exception:
            log.exception("unhandled error during cycle; will retry next cycle")

        if cfg.once:
            return 0

        time.sleep(cfg.poll_interval_seconds)


if __name__ == "__main__":
    raise SystemExit(main())
