// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic gate for whether a Claude review turn is worth spending.
// Runs no analysis of its own - coverage-diff-check.yml and
// duplicate-code-check.yml each own their check entirely (analysis,
// PR comment, status label, and a GitHub check run on the commit). This
// script only:
//
// 1. Forces a fresh, code-only label sync (never re-derived by an LLM - see
//    skills/review/SKILL.md) so "ci failing" / "merge conflict" reflect the
//    current commit, and exits immediately if either is set - a known-bad
//    signal that already exists needs no further waiting.
// 2. Otherwise, waits for both stages' check runs to reach a conclusion
//    (lib/wait-for-check.js), up to two hours, and decides from their
//    actual result. Neither stage's workflow is triggered from here, and
//    neither stage's result is ever read from a label that might still be
//    in flight - only a check run GitHub itself reports as "completed" is
//    trusted. This is what fully closes the race that the label-trusting
//    and inline-re-run approaches both had: coverage-diff-check.yml and
//    this workflow trigger on the identical "CI completed" event with no
//    ordering guarantee, so anything short of waiting for an authoritative,
//    already-finished conclusion could gate Claude on stale or incomplete
//    data.
//
// Sets the run_claude output the workflow uses to decide whether to invoke
// claude-code-action.

const syncLabels = require("./sync-pr-labels");
const analyzeCoverageDiff = require("./analyze-coverage-diff");
const analyzeDuplication = require("./analyze-duplication");
const { waitForCheckCompletion } = require("./lib/wait-for-check");

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

  const [coverage, duplication] = await Promise.all([
    waitForCheckCompletion(github, core, {
      owner,
      repo,
      ref: headSha,
      checkName: analyzeCoverageDiff.CHECK_NAME,
    }),
    waitForCheckCompletion(github, core, {
      owner,
      repo,
      ref: headSha,
      checkName: analyzeDuplication.CHECK_NAME,
    }),
  ]);

  if (!coverage.completed || !duplication.completed) {
    core.warning(
      `PR #${prNumber}: pre-flight checks did not complete within the wait window - not running Claude.`,
    );
    core.setOutput("run_claude", "false");
    return;
  }

  if (coverage.conclusion !== "success" || duplication.conclusion !== "success") {
    core.info(
      `PR #${prNumber}: a pre-flight stage failed (coverage=${coverage.conclusion}, ` +
        `duplication=${duplication.conclusion}) - not running Claude.`,
    );
    core.setOutput("run_claude", "false");
    return;
  }

  core.info(`PR #${prNumber}: pre-flight checks passed - running Claude.`);
  core.setOutput("run_claude", "true");
};
