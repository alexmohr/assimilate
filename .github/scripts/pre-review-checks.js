// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Orchestrates the review workflow's deterministic pre-flight: force a
// fresh, code-only recompute of "is CI green / does this conflict" (never
// re-derived by an LLM - see skills/review/SKILL.md), then run the
// coverage-diff and duplication hard gates before anything spends a Claude
// turn. Sets the run_claude output the workflow uses to decide whether to
// invoke claude-code-action.

const syncLabels = require("./sync-pr-labels");
const analyzeCoverageDiff = require("./analyze-coverage-diff");
const analyzeDuplication = require("./analyze-duplication");

const MARKER = "<!-- pre-review-checks -->";

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
        body: `${MARKER}\nAutomated pre-flight checks passed.`,
      });
    }
    return;
  }

  const body =
    `${MARKER}\n**Automated pre-flight checks failed** - these are deterministic findings ` +
    "(not from Claude); fix them before a review is worth spending on:\n\n" +
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
  jscpdReportPath,
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

  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  const [coverage, duplication] = await Promise.all([
    analyzeCoverageDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath }),
    analyzeDuplication({
      reportPath: jscpdReportPath,
      changedFiles: files.map((f) => f.filename),
    }),
  ]);

  const findings = [...coverage.findings, ...duplication.findings];
  await upsertPrecheckComment(github, owner, repo, prNumber, findings);

  if (!coverage.ok || !duplication.ok) {
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
    core.info(`PR #${prNumber}: pre-flight checks failed (${findings.length} finding(s)) - not running Claude.`);
    core.setOutput("run_claude", "false");
    return;
  }

  core.info(`PR #${prNumber}: pre-flight checks passed - running Claude.`);
  core.setOutput("run_claude", "true");
};
