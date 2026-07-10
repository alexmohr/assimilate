// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic gate for whether a Claude review turn is worth spending.
//
// duplicate-code-check.yml triggers on push, well before CI's multi-stage
// run finishes - by the time this script runs (triggered by CI completion),
// it has always already had time to land its `duplicate code` label, so
// that label is trusted as-is here, no need to re-run jscpd.
//
// coverage-diff-check.yml is different: it triggers on the *same* "CI
// completed" event this script does, with no ordering guarantee between the
// two workflows. Trusting its `coverage failed` label here would risk
// inviting Claude to review (and even approve) a commit whose coverage
// result simply hasn't landed yet - denying an automatic review on any
// pipeline failure only holds if the failure is checked fresh, not read
// from a label that might still be in flight. So this script calls
// analyze-coverage-diff.js's pure analyzeDiff() itself, synchronously,
// right before deciding - same deterministic check coverage-diff-check.yml
// runs, just guaranteed current at decision time instead of racing it. It
// deliberately does NOT call analyze-coverage-diff.js's default export
// (which posts the PR comment and sets/clears `coverage failed`) - that
// export runs on the same "CI completed" trigger this script does, and
// calling it from both places would race two non-atomic
// read-then-write comment upserts into creating duplicate comments each
// round. Only coverage-diff-check.yml owns those side effects.
//
// Sets the run_claude output the workflow uses to decide whether to invoke
// claude-code-action.

const syncLabels = require("./sync-pr-labels");
const { analyzeDiff } = require("./analyze-coverage-diff");

module.exports = async ({ github, context, core, prNumber, prLcovPath, baseLcovPath, force }) => {
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
  if (labels.includes(syncLabels.DUPLICATE_CODE_LABEL.name)) {
    core.info(`PR #${prNumber}: duplicate-code check already failed - not running Claude.`);
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

  const coverage = await analyzeDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath });
  if (!coverage.ok) {
    core.info(
      `PR #${prNumber}: coverage-diff check failed (${coverage.findings.length} finding(s)) - not running Claude.`,
    );
    core.setOutput("run_claude", "false");
    return;
  }

  core.info(`PR #${prNumber}: pre-flight checks passed - running Claude.`);
  core.setOutput("run_claude", "true");
};
