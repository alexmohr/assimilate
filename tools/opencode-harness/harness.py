#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""opencode-harness: a deterministic supervisor around opencode's full-auto mode.

Priority order, checked every poll cycle:

1. Work the oldest open pull request that currently has something fixable
   (`ci failing`, `merge conflict`, `precheck failed`, or `changes requested`
   - see gh.py). Always rebase onto the base branch first, whether or not
   `merge conflict` is set - a PR can be plainly behind base with no
   conflict yet still be carrying a problem base has already fixed (e.g. a
   since-patched dependency), which only an actual rebase picks up; asks
   opencode to resolve real conflicts if the rebase doesn't apply cleanly
   (see `_resolve_conflicts`). CI results are always discovered and reacted
   to by this harness's own Python, never by opencode - opencode only ever
   sees already-gathered log text handed to it in a prompt, it never queries
   CI itself. If CI is failing on nothing but the deterministic `pre-commit`
   check, fix it directly (re-run pre-commit locally, which autofixes, then
   commit/push) without spending an opencode call at all - see
   `_try_mechanical_ci_fix`. Otherwise fetch the concrete failure content (CI
   logs, review comments, coverage/duplicate-code bot comments) in plain
   Python, hand it to opencode as a fix prompt, then run this repo's own
   validation commands (pre-commit, and the exact skills/rust and
   skills/frontend checklists) before committing and pushing. Never touch
   the repo's own status labels - .github/workflows/pr-status-labels.yml
   owns those end to end and re-evaluates them automatically on every push.
2. If a PR keeps hitting the same problem after several push attempts, stop
   touching it (`opencode-harness-stuck` label + a comment) rather than
   burning cycles or pushing something worse.
3. Once there are zero open PRs, or every open PR is momentarily unactionable
   (e.g. all still mid-CI) and `HARNESS_FALLBACK_TO_ISSUES` hasn't been
   turned off, pick the newest open issue and implement it on a new branch,
   then open a PR - which flows back into step 1 on the next cycle.

See README.md for setup, required env vars, and the safety notes around
opencode's `--auto` flag.
"""

from __future__ import annotations

import argparse
import hashlib
import logging
import sys
import time
from enum import Enum
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


def _failure_signature(step: str, output: str) -> str:
    return hashlib.sha256(f"{step}\x00{output}".encode()).hexdigest()


# Absolute ceiling on local attempts regardless of whether each one is making
# progress - without this, a chain of distinct-but-never-converging failures
# would retry forever, burning unbounded opencode calls/time on one cycle.
_MAX_LOCAL_ATTEMPTS_HARD_CAP_MULTIPLIER = 3


def run_fix_and_validate(cfg: Config, prompt: str) -> tuple[bool, str]:
    """Runs opencode, then this repo's own validation commands, retrying with
    the concrete failure fed back in - never trusting the model's say-so.

    An attempt whose failure differs from the previous one counts as
    progress (fix bug A, reveal distinct bug B) and doesn't count against
    `max_local_validation_attempts` - only a failure that repeats *identically*
    does. Without this, a chain of real, distinct bugs revealed one at a time
    would exhaust the attempt budget and push nothing, and the next cycle's
    cross-cycle circuit breaker (see harness.py's stuck-cycle tracking) would
    then see "the same problem" persisting externally (nothing was pushed, so
    CI's own failure never changed) even though opencode kept finding and
    fixing genuinely new issues locally, just never enough of them in one go
    to land a pushable state. A hard cap (independent of whether progress is
    being made) still bounds worst-case cost/time.
    """
    current_prompt = prompt
    attempt = 0
    no_progress_streak = 0
    last_failure_sig: str | None = None
    hard_cap = cfg.max_local_validation_attempts * _MAX_LOCAL_ATTEMPTS_HARD_CAP_MULTIPLIER
    while True:
        attempt += 1
        result = opencode_runner.run_opencode(
            current_prompt, cfg.repo_dir, cfg.opencode_model, cfg.opencode_timeout_seconds
        )
        if not result.ok:
            no_progress_streak += 1
            log.info(
                "opencode run failed (attempt %d, %d/%d with no progress): %s",
                attempt,
                no_progress_streak,
                cfg.max_local_validation_attempts,
                result.output[:500],
            )
            if no_progress_streak >= cfg.max_local_validation_attempts or attempt >= hard_cap:
                return False, f"opencode run failing, no progress after {attempt} attempts"
            current_prompt = prompts.build_retry_prompt(
                current_prompt, "opencode run", result.output
            )
            continue
        if not git_ops.has_uncommitted_changes(cfg.repo_dir):
            return False, "opencode made no changes"
        changed = git_ops.changed_files(cfg.repo_dir)
        validation = validate.run_all(cfg.repo_dir, changed)
        if not validation.ok:
            # pre-commit's hooks and cargo fmt rewrite files in place as their
            # actual fix, even on the run that reports failure (that's the
            # point of an auto-fixing hook) - so a first failure here has
            # often already fixed itself on disk. Re-run once, deterministically,
            # before spending a whole opencode call on something a formatter
            # already solved.
            log.info(
                "validation step '%s' failed; retrying validation once before involving "
                "opencode, in case an auto-fixing hook just fixed it on disk",
                validation.step,
            )
            validation = validate.run_all(cfg.repo_dir, git_ops.changed_files(cfg.repo_dir))
        if validation.ok:
            return True, "validated"

        failure_sig = _failure_signature(validation.step, validation.output)
        made_progress = failure_sig != last_failure_sig
        last_failure_sig = failure_sig
        no_progress_streak = 0 if made_progress else no_progress_streak + 1
        log.info(
            "validation step '%s' failed (attempt %d, %s, %d/%d with no progress)",
            validation.step,
            attempt,
            "new failure" if made_progress else "same failure repeated",
            no_progress_streak,
            cfg.max_local_validation_attempts,
        )
        if no_progress_streak >= cfg.max_local_validation_attempts or attempt >= hard_cap:
            return False, f"validation still failing after {attempt} attempts"
        current_prompt = prompts.build_retry_prompt(
            current_prompt, validation.step, validation.output
        )


# CI checks the harness can resolve deterministically, without opencode:
# pre-commit's own hooks (ruff --fix, cargo +nightly fmt, trailing-whitespace,
# end-of-file-fixer, ...) rewrite files in place as their actual fix. A
# failing `pre-commit` check on a harness-authored commit almost always means
# the harness's local gate and CI's pre-commit environment drifted (a hook
# cache difference, `uv run` bootstrapping hook envs fresh, etc.), not a
# logic problem that needs judgment - so it's handled here directly, keeping
# opencode's cost/time/attempt budget reserved for problems that actually
# need it.
_MECHANICAL_CI_CHECKS = {"pre-commit"}


def _try_mechanical_ci_fix(cfg: Config, pr: PrDetail) -> bool:
    """Runs the repo's own pre-commit locally and pushes the result if that's
    enough to fix it. Returns False (without pushing anything) if pre-commit
    still fails after autofixing, or if it already passes locally with
    nothing to fix (a stale/flaky CI result) - both fall back to the normal
    opencode-driven flow.
    """
    if cfg.dry_run:
        log.info("[dry-run] would attempt a mechanical pre-commit fix for PR #%d", pr.number)
        return False
    result = validate.run_precommit(cfg.repo_dir)
    if not result.ok:
        # pre-commit's own autofixing hooks (ruff --fix, cargo +nightly fmt,
        # trailing-whitespace, ...) rewrite files in place as their actual
        # fix, even on the run that reports failure - that's simply how an
        # auto-fixing hook works, the same reason run_fix_and_validate
        # retries once below before ever involving opencode. Without this
        # retry, the very first run always looks like "still fails" even
        # when it just fixed everything, so this function could never
        # actually push anything.
        result = validate.run_precommit(cfg.repo_dir)
    if not result.ok:
        log.info("PR #%d: pre-commit still fails locally after autofixing", pr.number)
        git_ops.discard_uncommitted_changes(cfg.repo_dir)
        return False
    if not git_ops.has_uncommitted_changes(cfg.repo_dir):
        log.info("PR #%d: pre-commit already passes locally; CI failure looks stale", pr.number)
        return False
    if not git_ops.commit(cfg.repo_dir, "fix: apply pre-commit auto-fixes"):
        return False
    # force_with_lease unconditionally: handle_pr_fix always rebases onto
    # base before reaching here (see _resolve_conflicts), which rewrites
    # history the instant the branch was actually behind - a plain push
    # would be rejected as non-fast-forward in that case. Harmless when the
    # branch was already current, since nothing was rewritten.
    git_ops.push(cfg.repo_dir, pr.head_ref_name, force_with_lease=True)
    log.info("PR #%d: pushed a pre-commit autofix without invoking opencode", pr.number)
    return True


class RebaseOutcome(Enum):
    """_resolve_conflicts' three possible outcomes.

    CLEAN and RESOLVED_BY_OPENCODE are deliberately distinct, not just two
    flavors of "succeeded": a clean rebase replays already-CI-tested
    commits onto a new base with no new content at all, safe to push
    immediately - but opencode resolving a real conflict produces new,
    unvalidated file edits that need the same local validation gate any
    other opencode-authored change in this file goes through before ever
    reaching origin. Conflating the two let a conflict resolution get
    force-pushed with zero validation - see handle_pr_fix.
    """

    CLEAN = "clean"
    RESOLVED_BY_OPENCODE = "resolved_by_opencode"
    FAILED = "failed"


def _resolve_conflicts(cfg: Config) -> RebaseOutcome:
    """Rebases the current checkout onto `cfg.base_branch`, asking opencode to
    resolve real conflicts if the rebase doesn't apply cleanly.

    Called unconditionally on every PR the harness works, not just ones with
    the `merge conflict` label - a PR can be plainly behind base (no
    conflict, GitHub reports it as mergeable) yet still be carrying a
    problem base has already fixed (e.g. a since-patched dependency), which
    only actually rebasing picks up. When the branch is already current,
    `git rebase` is a no-op, so this is always safe to call.
    """
    ok, status = git_ops.rebase_onto(cfg.repo_dir, cfg.base_branch)
    if ok:
        return RebaseOutcome.CLEAN
    prompt = (
        f"Resolve the git rebase conflicts in this repository (rebasing onto "
        f"{cfg.base_branch}).\n\n`git status` output:\n\n{status}\n\n" + prompts.COMMON_RULES
    )
    result = opencode_runner.run_opencode(
        prompt, cfg.repo_dir, cfg.opencode_model, cfg.opencode_timeout_seconds
    )
    if not result.ok:
        git_ops.abort_rebase(cfg.repo_dir)
        return RebaseOutcome.FAILED
    ok, _ = git_ops.continue_rebase(cfg.repo_dir, cfg.base_branch)
    if not ok:
        git_ops.abort_rebase(cfg.repo_dir)
        return RebaseOutcome.FAILED
    return RebaseOutcome.RESOLVED_BY_OPENCODE


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


_DECISION_NEEDED_PHRASES = (
    "needs a decision",
    "needs a human decision",
    "needs a maintainer",
    "needs your decision",
    "human decision",
    "human sign-off",
    "explicit sign-off",
    "requires a human",
    "policy-level",
    "policy call",
    "policy decision",
)


def _looks_like_policy_question(review_comments: str | None) -> bool:
    """Best-effort: true if the review text itself is asking for a human
    judgment call, rather than just reporting an ordinary fixable bug.

    "changes_requested is the only thing outstanding" is the *normal* state
    partway through any multi-round review - most of the time it just means
    there's real, actionable feedback opencode hasn't landed a fix for yet
    in cfg.max_stuck_cycles tries, not that there's nothing left a code
    change could resolve. Treating every such case as "needs a maintainer's
    decision" produces false positives (observed on PR #323: a quota-parse
    error that aborts an import inconsistently with the rest of the
    codebase, and a UI filter/display mismatch - both concrete, fixable
    bugs, both mislabeled this way). Only the review text itself can
    actually distinguish "this is a values/policy call" from "this is a
    bug report" - the language a reviewer uses when explicitly asking for a
    decision (see the passphrase-export precedent this PR itself hit
    earlier: "policy-level security concern that needs a human decision")
    is the closest cheap signal available without full comprehension of the
    review's content.
    """
    if not review_comments:
        return False
    lowered = review_comments.lower()
    return any(phrase in lowered for phrase in _DECISION_NEEDED_PHRASES)


def _problem_summary(
    pr: PrDetail,
    ci_logs: str | None,
    review_comments: str | None,
    precheck_notes: str | None,
    max_chars: int = 600,
) -> str:
    """Renders what's actually blocking `pr` for the stuck-PR comment.

    "the same problem has persisted across 3 attempts" says nothing about
    what that problem actually was - a human reading the comment has to go
    dig through CI/review UI themselves to find out. This puts the same
    diagnostic content the harness fed to opencode directly in the comment.
    """
    parts = []
    if pr.ci_failing and ci_logs:
        parts.append(f"**CI failing** - end of the failing log:\n```\n{ci_logs[-max_chars:]}\n```")
    if pr.merge_conflict:
        parts.append("**Merge conflict** with the base branch.")
    if (pr.coverage_failed or pr.duplicate_code) and precheck_notes:
        parts.append(f"**Pre-flight check failed:**\n{precheck_notes[:max_chars]}")
    if pr.changes_requested and review_comments:
        parts.append(f"**Review comments requesting changes:**\n{review_comments[:max_chars]}")
    return "\n\n".join(parts) if parts else "(no diagnostic content was available)"


def _mark_stuck(
    cfg: Config, pr: PrDetail, reason: str, details: str | None = None, question: bool = False
) -> None:
    """Stops the harness retrying `pr` and posts why.

    `question` marks the harness's other signal, separate from the plain
    circuit breaker: a review thread keeps requesting changes across every
    retry with no CI/merge-conflict failure alongside it usually means the
    reviewer raised something opencode has no way to resolve by editing
    code - a product/policy call only a human can make (e.g. "is storing
    this value in plaintext acceptable at all"), not a bug to keep
    hammering at. `stuck_label` still applies either way so the harness
    stops burning cycles on it; `question_label` is the extra, more
    specific flag so a human scanning labels can tell "needs a decision"
    apart from "needs a better fix" at a glance.
    """
    if cfg.dry_run:
        log.info("[dry-run] would mark PR #%d stuck: %s", pr.number, reason)
        return
    gh.add_label(cfg.repo, pr.number, cfg.stuck_label)
    if question:
        gh.add_label(cfg.repo, pr.number, cfg.question_label)
        body = (
            f"opencode-harness: pausing on this PR - {reason}, and it looks like this needs a "
            f"decision from a maintainer rather than another code fix. Marked "
            f"`{cfg.stuck_label}` and `{cfg.question_label}`; please reply with a decision on "
            "the open review thread, then push a new commit or remove the labels to have the "
            "harness retry."
        )
    else:
        body = (
            f"opencode-harness: giving up on this PR for now - {reason}. "
            f"Marked `{cfg.stuck_label}`; push a new commit or remove the label "
            "to have the harness retry."
        )
    if details:
        body += f"\n\n---\n\n{details}"
    gh.comment(cfg.repo, pr.number, body)
    log.warning("PR #%d marked stuck: %s (question=%s)", pr.number, reason, question)


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

    failing_checks = gh.get_failing_check_names(cfg.repo, pr.number) if pr.ci_failing else []
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
        details = _problem_summary(pr, ci_logs, review_comments, precheck_notes)
        # Only review feedback recurring, with CI/merge/pre-flight all clean,
        # AND the review itself reading like an explicit request for a human
        # judgment call (see _looks_like_policy_question) - otherwise this is
        # just ordinary, actionable review feedback opencode failed to land
        # a fix for, which is the normal state mid-review, not a sign
        # there's nothing left a code change could resolve.
        is_question = (
            pr.changes_requested
            and not (pr.ci_failing or pr.merge_conflict or pr.coverage_failed or pr.duplicate_code)
            and _looks_like_policy_question(review_comments)
        )
        _mark_stuck(
            cfg,
            pr,
            f"the same problem has persisted across {attempts - 1} attempts",
            details,
            question=is_question,
        )
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
    pre_rebase_head = git_ops.head_sha(cfg.repo_dir)

    rebase_outcome = _resolve_conflicts(cfg)
    if rebase_outcome is RebaseOutcome.FAILED:
        _mark_stuck(cfg, pr, "could not resolve merge conflicts")
        return False

    if rebase_outcome is RebaseOutcome.CLEAN and git_ops.head_sha(
        cfg.repo_dir
    ) != git_ops.remote_head_sha(cfg.repo_dir, pr.head_ref_name):
        # rebase-onto-base (always run - see _resolve_conflicts' own
        # docstring) actually moved HEAD with no opencode involvement: base
        # had commits this branch didn't, and rebasing onto them picked up
        # a real fix (e.g. PR #323's cargo-deny failure on a since-patched
        # dependency). `git rebase` already committed that result locally -
        # nothing further needs to happen for the fix to exist, and no new
        # content means no new validation risk either. Push it now rather
        # than waiting on opencode/the mechanical shortcut to separately
        # produce a change: if the rebase alone was enough, a capable
        # opencode run correctly finds nothing left to fix and makes no
        # edits, run_fix_and_validate below reports "opencode made no
        # changes", and the rebase would otherwise be discarded via
        # discard_uncommitted_changes and never reach origin at all - reset
        # away again by the very next cycle's checkout_branch_at_remote.
        git_ops.push(cfg.repo_dir, pr.head_ref_name, force_with_lease=True)
        log.info("PR #%d: pushed a rebase-onto-base fix on its own", pr.number)
        return True

    if rebase_outcome is RebaseOutcome.RESOLVED_BY_OPENCODE:
        # Unlike a clean rebase, opencode edited files to resolve a real
        # conflict here - already committed via continue_rebase, but never
        # validated. That's new, unvetted content, so it needs the same
        # local gate any other opencode-authored change in this function
        # goes through before it can reach origin - a force-push straight
        # from conflict resolution with zero validation is exactly how a
        # bad resolution (formatting, a broken test) would reach origin
        # undetected until a full CI round-trip, if at all.
        changed = git_ops.changed_files_between(cfg.repo_dir, pre_rebase_head, "HEAD")
        validation = validate.run_all(cfg.repo_dir, changed)
        if not validation.ok:
            # pre-commit/cargo fmt autofix in place even on a failing run -
            # same reasoning as run_fix_and_validate's identical retry.
            validation = validate.run_all(cfg.repo_dir, changed)
        if not validation.ok:
            # A conflict that applies cleanly at the text level can still be
            # semantically broken - e.g. a struct gained a field in this PR's
            # own history while base independently added new call sites that
            # never learned about it; git's line-based merge has no way to
            # catch that, only an actual build does. That's exactly the kind
            # of concrete, fixable failure opencode can act on given another
            # turn, so retry through the same opencode+validate loop every
            # other fix path uses instead of giving up after a single
            # attempt - confirmed live on PR #323, where a real compile error
            # (a missing struct field two unrelated test call sites needed)
            # got surfaced clearly here and then simply discarded unfixed.
            retry_prompt = prompts.build_retry_prompt(
                f"You just resolved a merge conflict while rebasing this repository onto "
                f"{cfg.base_branch}. " + prompts.COMMON_RULES,
                validation.step,
                validation.output,
            )
            ok, _ = run_fix_and_validate(cfg, retry_prompt)
            if not ok:
                log.warning(
                    "PR #%d: opencode's conflict resolution failed local validation (%s)",
                    pr.number,
                    validation.step,
                )
                details = (
                    f"**Failed step:** `{validation.step}`\n\n```\n{validation.output[-4000:]}\n```"
                )
                _mark_stuck(
                    cfg,
                    pr,
                    "opencode's merge-conflict resolution failed local validation",
                    details,
                )
                git_ops.discard_uncommitted_changes(cfg.repo_dir)
                return False
        if git_ops.has_uncommitted_changes(cfg.repo_dir):
            git_ops.commit(cfg.repo_dir, "fix: apply pre-commit auto-fixes")
        git_ops.push(cfg.repo_dir, pr.head_ref_name, force_with_lease=True)
        log.info("PR #%d: pushed opencode's validated conflict resolution", pr.number)
        return True

    # Only take the mechanical shortcut when CI is the *only* outstanding
    # problem - if review feedback or a coverage/duplicate-code precheck is
    # also unresolved, a trivial "fix: apply pre-commit auto-fixes" push
    # would get counted as "solved" (see handle_pr_fix's return contract)
    # while the actual review feedback never reaches opencode this cycle.
    only_ci_outstanding = not (pr.changes_requested or pr.coverage_failed or pr.duplicate_code)
    if only_ci_outstanding and failing_checks and set(failing_checks) <= _MECHANICAL_CI_CHECKS:
        if _try_mechanical_ci_fix(cfg, pr):
            return True
        log.info(
            "PR #%d: mechanical fix for %s didn't resolve it, falling back to opencode",
            pr.number,
            failing_checks,
        )

    prompt = prompts.build_pr_fix_prompt(pr, ci_logs, review_comments, precheck_notes)
    ok, message = run_fix_and_validate(cfg, prompt)
    if not ok:
        log.warning("PR #%d: did not converge this cycle (%s)", pr.number, message)
        git_ops.discard_uncommitted_changes(cfg.repo_dir)
        return False

    committed = git_ops.commit(cfg.repo_dir, _commit_message_for(pr, cfg))
    if not committed:
        log.warning("PR #%d: opencode made no net changes; nothing to push", pr.number)
        return False
    # force_with_lease unconditionally, not just pr.merge_conflict: the
    # rebase-onto-base above now always runs, so history may have been
    # rewritten even when GitHub never flagged a conflict (a plain "behind
    # base" branch rebases cleanly with no label at all). A plain push
    # would be rejected as non-fast-forward in that case.
    git_ops.push(cfg.repo_dir, pr.head_ref_name, force_with_lease=True)
    log.info("PR #%d: pushed a fix, letting CI/review automation re-evaluate", pr.number)
    return True


def _check_and_fix_pr(cfg: Config, state: HarnessState, number: int) -> bool | None:
    """Checks one PR and fixes it if actionable.

    Returns None if there was nothing to attempt - merged, closed, ignored,
    still stuck with no new commits, or simply not actionable right now -
    all of which mean "keep scanning for a different PR." Otherwise returns
    handle_pr_fix's own True/False: an attempt was actually made (checkout,
    opencode, validate, push), so the caller must stop here regardless of
    whether it succeeded, rather than moving on to try another PR in the
    same cycle.
    """
    detail = gh.get_pr(cfg.repo, number)

    if cfg.ignore_label in detail.labels:
        return None
    if detail.state == "MERGED":
        state.clear_pr(number)
        log.info("PR #%d merged", number)
        return None
    if detail.state == "CLOSED":
        state.clear_pr(number)
        log.info("PR #%d closed without merging, skipping", number)
        return None

    if detail.checks_in_progress:
        # Judging this PR right now - fingerprinting it, counting a stuck
        # attempt, deciding it needs a fix - would mean doing so against a
        # commit whose CI/review hasn't had a chance to actually finish.
        # Concretely: push a fix, poll again a few minutes later while CI is
        # still running, see the *same* stale review/CI content because the
        # automated re-review this repo runs can't have landed yet either,
        # and count that as "no progress" toward max_stuck_cycles - even
        # though the fix was never actually evaluated. Skip entirely and
        # let a later cycle re-check once everything has settled.
        log.info("PR #%d: checks still in progress, waiting for them to settle", number)
        return None

    if detail.needs_human_review and not (
        detail.ci_failing
        or detail.merge_conflict
        or detail.coverage_failed
        or detail.duplicate_code
    ):
        # `needs human review` is the repo's own sticky sign-off gate (see
        # HUMAN_LABEL/humanSignOffStillStands in sync-pr-labels.js) - only a
        # human removing the *label* counts as clearing it; dismissing the
        # review that triggered it does not touch this label at all, and
        # neither does a fresh approval that leaves the label in place.
        # Deliberately keyed on needs_human_review alone, not also
        # detail.changes_requested: if this required both, changes_requested
        # flipping back to False on its own (e.g. that same or another
        # reviewer approves without separately removing the sticky label)
        # would fall through this branch entirely with needs_human_review
        # still set - straight into the un-stick block below, which assumes
        # reaching it means needs_human_review is already false and would
        # incorrectly clear this PR's stuck bookkeeping. CI/merge/coverage/
        # duplicate-code problems are ordinary and still worth fixing
        # regardless of this label, so those still fall through instead of
        # being caught here. Checked before the stuck-label-clearing logic
        # below so a harness-authored commit (which changes head_sha) can't
        # quietly un-stick a PR that's actually still waiting on a human -
        # without this it would look "stuck" again a few attempts later,
        # chasing the exact same already-addressed review content.
        if cfg.stuck_label not in detail.labels:
            _mark_stuck(
                cfg,
                detail,
                "this PR carries the repo's own `needs human review` label with no other "
                "fixable problem outstanding",
                "The `needs human review` label only ever clears when a human removes it "
                "themselves - pushing more commits, or even a fresh approval, can't do that. "
                "If a reviewer's changes-requested verdict is also still in effect, only that "
                "reviewer's own new review or dismissal refreshes it. Please resolve the "
                "outstanding review situation and remove `needs human review` yourself to have "
                "the harness retry.",
                question=True,
            )
            if not cfg.dry_run:
                state.set_stuck_reason(number, "needs_human_review")
        else:
            log.info("PR #%d: still needs human review, skipping", number)
        return None

    if cfg.stuck_label in detail.labels:
        recorded = state.pr_attempts.get(str(number))
        if recorded is not None and recorded.stuck_reason == "needs_human_review":
            # This PR was marked stuck by the needs_human_review branch
            # above, not the ordinary fingerprint circuit breaker - the only
            # thing that ever resolves that is the label clearing (a new
            # commit is neither necessary nor sufficient for it), and
            # detail.needs_human_review is already false here or the branch
            # above would have caught this PR instead. Un-stick regardless
            # of head_sha - without this, a human doing exactly what the
            # stuck comment asked (removing the label, no new commit) would
            # otherwise deadlock forever against the ordinary "no new
            # commits, still stuck" check below, which the comment never
            # warns about since it only tells them to remove one label.
            log.info("PR #%d: needs human review cleared; clearing and retrying", number)
            gh.remove_label(cfg.repo, number, cfg.stuck_label)
            if cfg.question_label in detail.labels:
                gh.remove_label(cfg.repo, number, cfg.question_label)
            state.clear_pr(number)
        else:
            head_sha = gh.get_pr_head_sha(cfg.repo, number)
            if recorded is not None and recorded.last_head_sha == head_sha:
                log.info("PR #%d still stuck (no new commits), skipping", number)
                return None
            log.info(
                "PR #%d has new commits since being marked stuck; clearing and retrying", number
            )
            gh.remove_label(cfg.repo, number, cfg.stuck_label)
            if cfg.question_label in detail.labels:
                gh.remove_label(cfg.repo, number, cfg.question_label)

    if not detail.needs_fix:
        log.info("PR #%d: nothing actionable (labels=%s)", number, detail.labels)
        return None

    return handle_pr_fix(cfg, state, detail)


def process_prs(cfg: Config, state: HarnessState, prs: list[PrSummary]) -> tuple[bool, bool]:
    """Handles at most one actionable PR. Returns (attempted, solved).

    `attempted` is True the moment any PR turns out actionable and
    `handle_pr_fix` actually runs on it, regardless of whether that attempt
    converged - that's this cycle's one unit of work already spent. It's
    False only when every PR was skipped (checks still in progress, stuck
    with no new commits, ignored, ...), which leaves the cycle free for
    something else - see run_once's issue fallback. `solved` is True only if
    a fix was actually pushed - this is what "solved problems" counts for
    --max-solved. Logs here, not in the caller, since `_check_and_fix_pr`'s
    own return can't tell "nothing was actionable" apart from "something was
    actionable, attempted, and didn't converge" - only this loop still has
    that distinction.
    """
    for summary in prs:
        result = _check_and_fix_pr(cfg, state, summary.number)
        if result is None:
            continue  # nothing to attempt here, keep scanning
        if not result:
            log.info(
                "%d open PR(s); attempted PR #%d, did not converge this cycle",
                len(prs),
                summary.number,
            )
        return True, result  # an attempt was made - stop here regardless of outcome
    log.info("%d open PR(s), none actionable right now", len(prs))
    return False, False


def process_single_pr(cfg: Config, state: HarnessState, number: int) -> bool:
    """Handles a specific PR (--pr N) regardless of auto-selection order.

    The one-time override of the stuck-cycle backoff for an explicitly
    targeted PR happens once per number, the first time `_override_stuck_pr`
    sees it - not on every cycle. This function may run for the same number
    on every poll cycle for as long as the process keeps running, so if it
    re-cleared the stuck label on every call, a long-running `--pr N` process
    would never let the circuit breaker hold: mark stuck, immediately
    clear+retry next cycle, mark stuck again, forever - repeatedly burning
    opencode attempts and reposting the same give-up comment. Delegating to
    the same `_check_and_fix_pr` the auto-scan path uses means a still-stuck
    PR with no new commits is correctly skipped after that first override.
    """
    return bool(_check_and_fix_pr(cfg, state, number))


def _override_stuck_pr(cfg: Config, state: HarnessState, overridden: set[int], number: int) -> None:
    """One-time-per-number override of the stuck-cycle backoff for a PR
    reached via `--pr` (an explicit list, or "all open PRs"): a human
    pointing the harness at a PR by number is choosing to retry it right
    now, which isn't the same as "retry forever on every poll cycle this
    number happens to come up again" - see process_single_pr's docstring for
    what goes wrong without the one-time guard. `overridden` is owned by
    main()'s loop and lives only for this process's lifetime; a restart
    re-overrides every targeted number once more, which is harmless (the
    same "explicitly targeting this PR means retry now" reasoning applies
    just as much right after a restart).
    """
    if number in overridden:
        return
    overridden.add(number)
    if cfg.dry_run:
        return
    if str(number) in state.pr_attempts:
        log.info(
            "--pr %d: clearing %d prior attempt(s) from a previous run before starting",
            number,
            state.pr_attempts[str(number)].attempts,
        )
        state.clear_pr(number)
    detail = gh.get_pr(cfg.repo, number)
    if cfg.stuck_label in detail.labels:
        log.info("--pr %d: clearing stuck label before starting", number)
        gh.remove_label(cfg.repo, number, cfg.stuck_label)
    if cfg.question_label in detail.labels:
        log.info("--pr %d: clearing question label before starting", number)
        gh.remove_label(cfg.repo, number, cfg.question_label)


def _implement_issue(cfg: Config, state: HarnessState, issue: dict) -> bool:
    """Implements `issue` on a new branch and opens a PR. Returns True on success."""
    number = issue["number"]
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
        git_ops.discard_uncommitted_changes(cfg.repo_dir)
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


def process_issues(cfg: Config, state: HarnessState) -> bool:
    """Implements the newest actionable open issue. Returns True if a PR was opened.

    Called both when there are zero open PRs at all, and - when
    `cfg.fallback_to_issues` - when open PRs exist but none of them are
    actionable this cycle (e.g. all mid-CI); the log lines below stay
    generic rather than assuming "no open PRs" for that reason.
    """
    issues = gh.list_open_issues(cfg.repo)
    candidates = [
        i
        for i in issues
        if cfg.ignore_label not in i["labels"] and cfg.stuck_label not in i["labels"]
    ]
    if not candidates:
        log.info("no actionable open issues either; idle this cycle")
        return False

    issue = candidates[0]
    log.info("picking up newest open issue #%d: %s", issue["number"], issue["title"])
    return _implement_issue(cfg, state, issue)


def process_single_issue(cfg: Config, state: HarnessState, number: int) -> bool:
    """Implements a specific issue (--issue N) regardless of auto-selection."""
    issue = gh.get_issue(cfg.repo, number)
    if issue.get("state") == "CLOSED":
        log.info("issue #%d is already closed, nothing to do", number)
        return False
    log.info("targeting issue #%d: %s", number, issue["title"])
    return _implement_issue(cfg, state, issue)


def _round_robin(numbers: list[int], cycle: int) -> int | None:
    """Picks one target per poll cycle out of a (possibly multi-item, possibly
    per-cycle-refreshed) list - `cycle` is the poll loop's own iteration
    count, so successive calls advance through `numbers` in order and wrap
    around, rather than the harness hammering the same one every time or
    needing to persist a cursor across restarts.
    """
    if not numbers:
        return None
    return numbers[cycle % len(numbers)]


def _clear_stuck_after_base_merge(cfg: Config, state: HarnessState) -> None:
    """Gives every stuck PR a fresh look the moment `cfg.base_branch` itself
    moves - a merge landing on base is real, external progress the ordinary
    per-PR circuit breaker can't see on its own (it only ever compares a
    PR's *own* head_sha across cycles, never base's), even though it's
    exactly what can unstick one: a merge conflict that looked unresolvable,
    or a CI failure caused by something already broken on base, can both
    simply disappear once base advances past whatever caused them - rather
    than staying parked until a human notices and clears the label by hand.

    A no-op on the very first cycle a state file has ever seen (nothing
    recorded yet to compare against, so nothing is swept) and whenever base
    hasn't actually moved. Skips any PR whose stuck marking came from the
    needs_human_review branch specifically (see PrAttempt.stuck_reason) -
    that one is gated on the repo's own `needs human review` label, which a
    base-branch merge has no bearing on at all; clearing it here would just
    have `_check_and_fix_pr` re-mark it (and repost the same give-up
    comment) on the very next cycle regardless.
    """
    if cfg.dry_run:
        return
    current = gh.get_branch_head_sha(cfg.repo, cfg.base_branch)
    if not current:
        return
    previous = state.last_base_sha
    state.set_last_base_sha(current)
    if previous is None or previous == current:
        return
    for pr in gh.list_open_prs(cfg.repo):
        if cfg.stuck_label not in pr.labels:
            continue
        recorded = state.pr_attempts.get(str(pr.number))
        if recorded is not None and recorded.stuck_reason == "needs_human_review":
            continue
        log.info(
            "PR #%d: %s advanced (%s -> %s) since last cycle; clearing stuck state for a "
            "fresh look",
            pr.number,
            cfg.base_branch,
            previous[:8],
            current[:8],
        )
        gh.remove_label(cfg.repo, pr.number, cfg.stuck_label)
        if cfg.question_label in pr.labels:
            gh.remove_label(cfg.repo, pr.number, cfg.question_label)
        state.clear_pr(pr.number)


def run_once(cfg: Config, state: HarnessState, cycle: int, overridden_prs: set[int]) -> bool:
    """Runs a single cycle. Returns True if it solved a problem (pushed a PR
    fix, or implemented an issue into a new PR) - see --max-solved."""
    _clear_stuck_after_base_merge(cfg, state)
    if cfg.target_prs is not None or cfg.target_all_prs:
        # "all open PRs" is re-resolved fresh every cycle (new PRs may have
        # opened, others merged/closed since the last one) - an explicit
        # list from --pr N [N ...] stays fixed for the process's lifetime.
        numbers = (
            list(cfg.target_prs)
            if cfg.target_prs is not None
            else [p.number for p in gh.list_open_prs(cfg.repo)]
        )
        number = _round_robin(numbers, cycle)
        if number is None:
            log.info("--pr: no open PRs to target this cycle")
            return False
        _override_stuck_pr(cfg, state, overridden_prs, number)
        return process_single_pr(cfg, state, number)
    if cfg.target_issues is not None:
        number = _round_robin(list(cfg.target_issues), cycle)
        if number is None:
            return False
        return process_single_issue(cfg, state, number)

    prs = gh.list_open_prs(cfg.repo)
    if prs:
        attempted, solved = process_prs(cfg, state, prs)
        if attempted or not cfg.fallback_to_issues:
            return solved
        # Nothing among the open PRs was actionable this cycle (e.g. all mid-
        # CI) - rather than idling until the next poll, pick up an open issue
        # instead. Only reached when process_prs made no attempt at all; if
        # it attempted a PR and just didn't converge, that's real work
        # already done this cycle, not idle time to fill.
        return process_issues(cfg, state)
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
    parser.add_argument(
        "--pr",
        type=int,
        nargs="*",
        default=None,
        metavar="N",
        help=(
            "work only on PR(s) N instead of auto-selecting - one or more numbers "
            "(--pr 12 34), round-robining one per poll cycle if more than one; bare "
            "--pr with no numbers targets every currently open PR, re-resolved fresh "
            "each cycle. Mutually exclusive with --issue"
        ),
    )
    parser.add_argument(
        "--issue",
        type=int,
        nargs="*",
        default=None,
        metavar="N",
        help=(
            "implement only issue(s) N instead of auto-selecting - one or more numbers "
            "(--issue 5 6), round-robining one per poll cycle if more than one. "
            "Mutually exclusive with --pr"
        ),
    )
    args = parser.parse_args()
    if args.pr is not None and args.issue is not None:
        parser.error("--pr and --issue are mutually exclusive")
    if args.issue is not None and len(args.issue) == 0:
        parser.error(
            "--issue requires at least one issue number "
            "(use --pr with no number for 'all open PRs')"
        )

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
    if args.pr is not None:
        if len(args.pr) == 0:
            overrides["target_all_prs"] = True
        else:
            overrides["target_prs"] = tuple(args.pr)
    if args.issue is not None:
        overrides["target_issues"] = tuple(args.issue)
    if overrides:
        cfg = Config(**{**cfg.__dict__, **overrides})

    setup_logging(cfg.log_file)
    log.info("opencode-harness starting: %s", cfg.summary())

    state = HarnessState.load(cfg.state_file)
    solved_count = 0
    cycle = 0
    # Every PR number the harness has already applied _override_stuck_pr's
    # one-time stuck-cycle-backoff override to, this process's lifetime -
    # see that function's docstring for why it must happen once per number,
    # not on every cycle the number comes up again under --pr.
    overridden_prs: set[int] = set()

    while True:
        try:
            if run_once(cfg, state, cycle, overridden_prs):
                solved_count += 1
                if cfg.max_solved is not None and solved_count >= cfg.max_solved:
                    log.info("solved %d problem(s), reached --max-solved; stopping", solved_count)
                    return 0
        except Exception:
            log.exception("unhandled error during cycle; will retry next cycle")

        cycle += 1
        if cfg.once:
            return 0

        time.sleep(cfg.poll_interval_seconds)


if __name__ == "__main__":
    raise SystemExit(main())
