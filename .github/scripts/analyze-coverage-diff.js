// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic "new/changed code must be covered" + "coverage must not
// regress" checks, so the review workflow doesn't have to spend a Claude
// turn re-deriving facts an lcov report already answers.
//
// Runs solely in coverage-diff-check.yml, which owns the PR comment, the
// `coverage failed` label, AND a "Coverage Diff Check" GitHub check run on
// the commit. claude-review.yml never runs this analysis itself - it
// can't, since it triggers on the same "CI completed" event this workflow
// does with no ordering guarantee, so re-running the check inline would
// either race coverage-diff-check.yml's own writes (the bug that used to
// duplicate the PR comment) or still be gating on a guess. Instead
// pre-review-checks.js waits for every check run on the commit
// (lib/wait-for-check.js, not specific to this one by name) to reach an
// authoritative, already-finished conclusion. See skills/review/SKILL.md
// for the full reasoning.

const fs = require("fs");
const { parseLcov, totals } = require("./lib/lcov");
const syncLabels = require("./sync-pr-labels");
const { upsertMarkedComment } = require("./lib/pr-comment");

const MARKER = "<!-- coverage-diff-check -->";
const CHECK_NAME = "Coverage Diff Check";

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

async function publishCheckRun(github, owner, repo, headSha, ok, findings) {
  await github.rest.checks.create({
    owner,
    repo,
    name: CHECK_NAME,
    head_sha: headSha,
    status: "completed",
    conclusion: ok ? "success" : "failure",
    output: {
      title: ok ? "Coverage-diff check passed" : "Coverage-diff check failed",
      summary: ok
        ? "No new/changed lines are uncovered, and aggregate coverage did not regress."
        : findings.join("\n"),
    },
  });
}

module.exports = async ({ github, context, core, prNumber, headSha, prLcovPath, baseLcovPath }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const { ok, findings } = await analyzeDiff({ github, owner, repo, prNumber, prLcovPath, baseLcovPath });

  // Published first and unconditionally: this is the signal
  // pre-review-checks.js polls for via lib/wait-for-check.js, so it must
  // land regardless of what the comment/label steps below do.
  await publishCheckRun(github, owner, repo, headSha, ok, findings);

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

  // Recompute status now that "coverage failed" may have changed, so the
  // overall status label reflects it immediately rather than waiting for the
  // next unrelated trigger. selfCheckNames excludes this exact job (still
  // "in progress" - it's the one calling this) from sync-pr-labels.js's own
  // ready-to-merge completeness check, so a genuinely-finished PR isn't kept
  // waiting just because this job hasn't technically completed the instant
  // it asks.
  await syncLabels({ github, context, core, prNumber, selfCheckNames: ["Check coverage diff"] });

  return { ok, findings };
};

module.exports.analyzeDiff = analyzeDiff;
