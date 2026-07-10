// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic "new/changed code must be covered" + "coverage must not
// regress" checks, so the review workflow doesn't have to spend a Claude
// turn re-deriving facts an lcov report already answers.
//
// The pure analyzeDiff() below is called from two places: the default
// export here (comment + `coverage failed` label side effects, invoked
// solely by coverage-diff-check.yml) and, read-only, from
// pre-review-checks.js right before it decides whether to invoke Claude.
// pre-review-checks.js deliberately does NOT call the default export - both
// coverage-diff-check.yml and claude-review.yml trigger on the same "CI
// completed" event with no ordering guarantee, and upsertMarkedComment's
// read-then-write isn't atomic, so if both posted the comment/label
// side effects they could race each other into creating two comments for
// the same finding. Only coverage-diff-check.yml owns those side effects;
// pre-review-checks.js only needs a fresh ok/findings answer to gate on. See
// skills/review/SKILL.md for the full reasoning.

const fs = require("fs");
const { parseLcov, totals } = require("./lib/lcov");
const syncLabels = require("./sync-pr-labels");
const { upsertMarkedComment } = require("./lib/pr-comment");

const MARKER = "<!-- coverage-diff-check -->";

// Files where "this line has no coverage" isn't a meaningful finding: the
// tests themselves, generated code, and e2e specs (exercised through the
// browser, not instrumented the same way as unit-tested source).
const EXCLUDED_PATHS = [
  /^frontend\/src\/types\/generated\//,
  /(^|\/)tests?\//i,
  /\.(test|spec)\.[jt]sx?$/,
  /^frontend\/e2e\//,
  /_test\.rs$/,
];

function addedLineNumbers(patch) {
  const added = [];
  if (!patch) return added;
  let newLine = 0;
  for (const line of patch.split("\n")) {
    const hunk = line.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunk) {
      newLine = Number(hunk[1]);
      continue;
    }
    if (line.startsWith("+") && !line.startsWith("+++")) {
      added.push(newLine);
      newLine += 1;
    } else if (!line.startsWith("-") && !line.startsWith("\\")) {
      // context line - present in both old and new file, advances newLine.
      // Removed ('-') lines and the "no newline at eof" marker ('\') don't
      // exist in the new file, so they don't advance it.
      newLine += 1;
    }
  }
  return added;
}

// Pure report analysis, kept separate from the GitHub orchestration below so
// it's independently reusable/testable (also used directly by
// pre-review-checks.js's predecessor logic - see git history).
async function analyzeDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath }) {
  const findings = [];

  // Either artifact can be legitimately missing: the PR's own CI run may
  // predate the coverage-final upload (an already-open PR that hasn't been
  // pushed to since), or main may not have a run with it yet. Don't block
  // the review pipeline on historical data that can't exist yet - the check
  // becomes active again once a fresh push produces both artifacts.
  if (!fs.existsSync(prLcovPath) || !fs.existsSync(baseLcovPath)) {
    return { ok: true, findings: [] };
  }

  const prLcov = parseLcov(fs.readFileSync(prLcovPath, "utf8"));
  const baseLcov = parseLcov(fs.readFileSync(baseLcovPath, "utf8"));

  const prTotals = totals(prLcov);
  const baseTotals = totals(baseLcov);
  if (prTotals.percent < baseTotals.percent) {
    findings.push(
      `Aggregate line coverage decreased from ${baseTotals.percent.toFixed(2)}% (main) to ` +
        `${prTotals.percent.toFixed(2)}% (this PR) - check for removed or weakened tests, ` +
        "even if no specific uncovered line is flagged below.",
    );
  }

  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  for (const file of files) {
    if (EXCLUDED_PATHS.some((p) => p.test(file.filename))) continue;
    const lineHits = prLcov.get(file.filename);
    if (!lineHits) continue; // not an instrumented file (docs, config, ...)

    for (const lineNo of addedLineNumbers(file.patch)) {
      if (!lineHits.has(lineNo)) continue; // not an executable line
      if (lineHits.get(lineNo) === 0) {
        findings.push(`${file.filename}:${lineNo} is new/changed but has no test coverage.`);
      }
    }
  }

  return { ok: findings.length === 0, findings };
}

module.exports = async ({ github, context, core, prNumber, prLcovPath, baseLcovPath }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const { ok, findings } = await analyzeDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath });

  if (findings.length === 0) {
    // Nothing to say - don't spam an "all good" comment on every clean run.
    // If a prior failing comment exists, replace it so it doesn't read stale.
    await upsertMarkedComment(github, owner, repo, prNumber, MARKER, `${MARKER}\nCoverage-diff check passed.`, {
      onlyIfExists: true,
    });
  } else {
    const body =
      `${MARKER}\n**Coverage-diff check failed** - this is a deterministic finding ` +
      "(not from Claude); fix it before a review is worth spending on:\n\n" +
      findings.map((f) => `- ${f}`).join("\n");
    await upsertMarkedComment(github, owner, repo, prNumber, MARKER, body);
  }

  const label = syncLabels.COVERAGE_LABEL;
  if (!ok) {
    await syncLabels.ensureLabelExists(github, owner, repo, label);
    await github.rest.issues.addLabels({ owner, repo, issue_number: prNumber, labels: [label.name] });
    core.info(`PR #${prNumber}: coverage-diff check failed (${findings.length} finding(s)).`);
  } else {
    // This workflow owns the label's full lifecycle (see COVERAGE_LABEL in
    // sync-pr-labels.js) - explicitly clear it here rather than relying on a
    // generic synchronize-triggered clear, since coverage-diff-check.yml
    // reacts to the same "workflow_run: CI completed" event
    // pr-status-labels.yml does, and a blind clear there could otherwise
    // race a fresh failing result from this workflow.
    await github.rest.issues
      .removeLabel({ owner, repo, issue_number: prNumber, name: label.name })
      .catch((err) => {
        if (err.status !== 404) throw err;
      });
    core.info(`PR #${prNumber}: coverage-diff check passed.`);
  }

  // Recompute status now that "coverage failed" may have changed, so the PR
  // Merge Gate and overall status label reflect it immediately rather than
  // waiting for the next unrelated trigger.
  await syncLabels({ github, context, core, prNumber });

  return { ok, findings };
};

module.exports.analyzeDiff = analyzeDiff;
