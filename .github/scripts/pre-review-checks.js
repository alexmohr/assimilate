// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Orchestrates the review workflow's deterministic pre-flight: force a
// fresh, code-only recompute of "is CI green / does this conflict" (never
// re-derived by an LLM - see skills/review/SKILL.md), then run the
// coverage-diff hard gate before anything spends a Claude turn. The
// duplicate-code hard gate is a separate stage that runs independently in
// .github/workflows/duplicate-code-check.yml (see analyze-duplication.js);
// this script checks whether *any* pre-flight stage - coverage-diff here or
// duplicate-code there - has already failed on this commit before invoking
// Claude. Sets the run_claude output the workflow uses to decide whether to
// invoke claude-code-action.

const syncLabels = require("./sync-pr-labels");
const analyzeCoverageDiff = require("./analyze-coverage-diff");

const MARKER = "<!-- pre-review-checks:coverage -->";

async function upsertPrecheckComment(github, owner, repo, prNumber, findings) {
  const comments = await github.paginate(github.rest.issues.listComments, {
    owner,
    repo,
    issue_number: prNumber,
    per_page: 100,
  });
  const existing = comments.find((c) => c.body.startsWith(MARKER));

  if (findings.length === 0) {
    // Nothing to say - don't spam an "all good" comment on every clean run.
    // If a prior failing comment exists, replace it so it doesn't read stale.
    if (existing) {
      await github.rest.issues.updateComment({
        owner,
        repo,
        comment_id: existing.id,
        body: `${MARKER}\nCoverage-diff check passed.`,
      });
    }
    return;
  }

  const body =
    `${MARKER}\n**Coverage-diff check failed** - this is a deterministic finding ` +
    "(not from Claude); fix it before a review is worth spending on:\n\n" +
    findings.map((f) => `- ${f}`).join("\n");

  if (existing) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: prNumber, body });
  }
}

module.exports = async ({
  github,
  context,
  core,
  prNumber,
  prLcovPath,
  baseLcovPath,
  force,
}) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  // Step 1: force a fresh, code-only label sync before deciding anything -
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
  // Two independent pre-flight stages exist: this script runs coverage-diff
  // below, and .github/workflows/duplicate-code-check.yml runs the
  // duplicate-code scan on its own trigger (push, not CI/label-driven), so
  // it may already have failed on this exact commit by the time we get here.
  // If EITHER stage has already failed, don't bother running (or re-running)
  // the other - a forced /claude-review retrigger still must not bypass
  // this, only "already reviewed this exact commit" is force-bypassable.
  if (
    labels.includes(syncLabels.STATUS_LABELS.PRECHECK_FAILED.name) ||
    labels.includes(syncLabels.DUPLICATE_CODE_LABEL.name)
  ) {
    core.info(`PR #${prNumber}: a pre-flight stage already failed - not running Claude.`);
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

  const coverage = await analyzeCoverageDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath });

  await upsertPrecheckComment(github, owner, repo, prNumber, coverage.findings);

  if (!coverage.ok) {
    const label = syncLabels.STATUS_LABELS.PRECHECK_FAILED;
    await syncLabels.ensureLabelExists(github, owner, repo, label);
    await github.rest.issues.addLabels({
      owner,
      repo,
      issue_number: prNumber,
      labels: [label.name],
    });
    // Recompute status now that "precheck failed" is set, so the PR Merge
    // Gate and overall status label reflect it immediately.
    await syncLabels({ github, context, core, prNumber });
    core.info(
      `PR #${prNumber}: coverage-diff check failed (${coverage.findings.length} finding(s)) - not running Claude.`,
    );
    core.setOutput("run_claude", "false");
    return;
  }

  core.info(`PR #${prNumber}: pre-flight checks passed - running Claude.`);
  core.setOutput("run_claude", "true");
};
