// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic gate for whether a Claude review turn is worth spending.
// Runs no analysis of its own - every pre-flight stage (coverage-diff-
// check.yml, duplicate-code-check.yml, CI's own jobs, ...) owns its check
// entirely and reports a GitHub check run on the commit. This script only:
//
// 1. Forces a fresh, code-only label sync (never re-derived by an LLM - see
//    skills/review/SKILL.md) so "ci failing" / "merge conflict" reflect the
//    current commit, and exits immediately if either is set - a known-bad
//    signal that already exists needs no further waiting.
// 2. Otherwise, waits for every *other* check run on the commit to reach a
//    conclusion (lib/wait-for-check.js), up to two hours, and decides from
//    the actual results - not a fixed list of named checks, so any new
//    pre-flight stage added to the pipeline later is covered automatically
//    with no change needed here. Nothing is ever triggered from this
//    script, and no result is ever read from a label that might still be
//    in flight - only what GitHub itself reports as "completed" is
//    trusted. This is what fully closes the race a label-trusting or
//    inline-re-run approach would have: coverage-diff-check.yml and this
//    workflow trigger on the identical "CI completed" event with no
//    ordering guarantee, so anything short of waiting for an authoritative,
//    already-finished conclusion could gate Claude on stale or incomplete
//    data.
//
// Sets the run_claude output the workflow uses to decide whether to invoke
// claude-code-action.

const syncLabels = require("./sync-pr-labels");
const { waitForAllChecks } = require("./lib/wait-for-check");

// Excluded from the "wait for everything" gate below because waiting on
// them would either deadlock or be circular:
// - "Check if a review is actually needed" / "Review PR" are this exact
//   workflow's own jobs (the gate job that already ran, and the job this
//   script is running inside right now) - waiting on the latter would wait
//   on itself forever.
// - GATE_CHECK_NAME ("PR Merge Gate") is derived FROM the review outcome
//   (it's only "success" once the PR is reviewDecision APPROVED), so at
//   this point - before Claude has reviewed anything - it can never show
//   success yet. Waiting on it would make run_claude permanently false.
const SELF_CHECK_NAMES = ["Check if a review is actually needed", "Review PR", syncLabels.GATE_CHECK_NAME];

module.exports = async ({ github, context, core, prNumber, headSha, force }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  // Force a fresh, code-only label sync before deciding anything -
  // guarantees "ci failing" / "merge conflict" reflect the current commit
  // even if the general-purpose sync workflow hasn't run yet.
  await syncLabels({ github, context, core, prNumber });

  const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  const labels = pr.labels.map((l) => l.name);

  if (labels.includes(syncLabels.STATUS_LABELS.CI_FAILING.name)) {
    core.info(`PR #${prNumber}: CI is failing - not running Claude.`);
    core.setOutput("run_claude", "false");
    return;
  }
  if (labels.includes(syncLabels.STATUS_LABELS.MERGE_CONFLICT.name)) {
    core.info(`PR #${prNumber}: has merge conflicts - not running Claude.`);
    core.setOutput("run_claude", "false");
    return;
  }

  // claude-approved / claude-changes-requested only exist for the current
  // commit - sync-pr-labels.js clears them on every push - so their
  // presence alone means this exact commit already has a Claude verdict.
  const alreadyReviewed =
    labels.includes(syncLabels.REVIEW_VERDICT_LABELS.APPROVED.name) ||
    labels.includes(syncLabels.REVIEW_VERDICT_LABELS.CHANGES_REQUESTED.name);
  if (!force && alreadyReviewed) {
    core.info(`PR #${prNumber}: already has a Claude verdict for this commit - skipping.`);
    core.setOutput("run_claude", "false");
    return;
  }

  const result = await waitForAllChecks(github, core, {
    owner,
    repo,
    ref: headSha,
    excludeNames: SELF_CHECK_NAMES,
  });

  if (!result.completed) {
    core.warning(
      `PR #${prNumber}: not all checks completed within the wait window (still pending: ` +
        `${result.pending.join(", ")}) - not running Claude.`,
    );
    core.setOutput("run_claude", "false");
    return;
  }

  if (!result.ok) {
    core.info(`PR #${prNumber}: check(s) failed (${result.failed.join(", ")}) - not running Claude.`);
    core.setOutput("run_claude", "false");
    return;
  }

  core.info(`PR #${prNumber}: all checks passed - running Claude.`);
  core.setOutput("run_claude", "true");
};
