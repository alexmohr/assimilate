// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Standalone duplicate-code gate: reads a jscpd JSON report, keeps only
// clusters that touch this PR's changed files (repo-wide duplication
// unrelated to the diff isn't this PR's problem to fix), posts the actual
// duplicated source as a PR comment, sets/clears its own `duplicate code`
// label, and publishes a "Duplicate Code Check" GitHub check run on the
// commit - a separate stage from the coverage-diff `precheck failed` check,
// deliberately not folded into it (see DUPLICATE_CODE_LABEL in
// sync-pr-labels.js for why). Hard gate: "Logic is never to be duplicated
// instead of reused" (skills/review/SKILL.md) is a deterministic fact once
// jscpd has flagged a cluster, not something Claude needs to re-derive.
// What counts as "generated, ignore it" (`.sqlx/`, generated TS types,
// lockfiles, ...) is configured in `.jscpd.json` at the repo root, not here
// - editing that file is how maintainers extend the ignore list without
// touching this script or the workflow.
//
// pre-review-checks.js never re-runs jscpd itself - it polls the check run
// this module publishes via lib/wait-for-check.js and waits for an
// authoritative, already-finished conclusion, same as it does for
// coverage-diff. See skills/review/SKILL.md for the full reasoning.

const fs = require("fs");
const syncLabels = require("./sync-pr-labels");
const { upsertMarkedComment } = require("./lib/pr-comment");

const MARKER = "<!-- duplicate-code-check -->";
const CHECK_NAME = "Duplicate Code Check";

const normalize = (p) => p.replace(/^\.\//, "");

// Pure report analysis, kept separate from the GitHub orchestration below so
// it's independently reusable/testable.
function analyzeReport({ reportPath, changedFiles }) {
  if (!fs.existsSync(reportPath)) {
    return { ok: true, findings: [] };
  }

  const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
  const duplicates = report.duplicates || [];
  const changedSet = new Set(changedFiles.map(normalize));
  const findings = [];

  for (const dup of duplicates) {
    const first = dup.firstFile;
    const second = dup.secondFile;
    if (!first || !second) continue;

    const firstName = normalize(first.name);
    const secondName = normalize(second.name);
    if (!changedSet.has(firstName) && !changedSet.has(secondName)) continue;

    findings.push({
      firstFile: firstName,
      firstStart: first.start,
      firstEnd: first.end,
      secondFile: secondName,
      secondStart: second.start,
      secondEnd: second.end,
      lines: dup.lines,
      tokens: dup.tokens,
      format: dup.format || "",
      fragment: dup.fragment || "",
    });
  }

  return { ok: findings.length === 0, findings };
}

function formatFinding(finding, index) {
  const header =
    `### ${index + 1}. \`${finding.firstFile}:${finding.firstStart}-${finding.firstEnd}\` ` +
    `matches \`${finding.secondFile}:${finding.secondStart}-${finding.secondEnd}\` ` +
    `(${finding.lines} lines, ${finding.tokens} tokens)`;
  if (!finding.fragment) return header;
  return `${header}\n\n\`\`\`${finding.format}\n${finding.fragment}\n\`\`\``;
}

async function upsertComment(github, owner, repo, prNumber, findings) {
  if (findings.length === 0) {
    // Nothing to say - don't spam an "all good" comment on every clean run.
    // If a prior failing comment exists, replace it so it doesn't read stale.
    await upsertMarkedComment(
      github,
      owner,
      repo,
      prNumber,
      MARKER,
      `${MARKER}\nDuplicate-code check passed - no duplication found in changed files.`,
      { onlyIfExists: true },
    );
    return;
  }

  const body =
    `${MARKER}\n**Duplicate code detected** - this is a deterministic finding (not from Claude); ` +
    "these clusters touch files changed in this PR and must be resolved before a review is worth " +
    "spending on:\n\n" +
    findings.map(formatFinding).join("\n\n");

  await upsertMarkedComment(github, owner, repo, prNumber, MARKER, body);
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
      title: ok ? "No duplicate code found" : "Duplicate code found",
      summary: ok
        ? "No duplication touching this PR's changed files."
        : findings
            .map(
              (f) =>
                `${f.firstFile}:${f.firstStart}-${f.firstEnd} matches ${f.secondFile}:${f.secondStart}-${f.secondEnd}`,
            )
            .join("\n"),
    },
  });
}

module.exports = async ({ github, context, core, prNumber, headSha, reportPath }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  const { ok, findings } = analyzeReport({
    reportPath,
    changedFiles: files.map((f) => f.filename),
  });

  // Published first and unconditionally: this is the signal
  // pre-review-checks.js polls for via lib/wait-for-check.js, so it must
  // land regardless of what the comment/label steps below do.
  await publishCheckRun(github, owner, repo, headSha, ok, findings);

  await upsertComment(github, owner, repo, prNumber, findings);

  const label = syncLabels.DUPLICATE_CODE_LABEL;
  if (!ok) {
    await syncLabels.ensureLabelExists(github, owner, repo, label);
    await github.rest.issues.addLabels({
      owner,
      repo,
      issue_number: prNumber,
      labels: [label.name],
    });
    core.info(`PR #${prNumber}: duplicate-code check failed (${findings.length} finding(s)).`);
  } else {
    // This workflow owns the label's full lifecycle (see DUPLICATE_CODE_LABEL
    // in sync-pr-labels.js) - explicitly clear it here rather than relying on
    // sync-pr-labels.js's synchronize-triggered clear, since that reacts to
    // the same push event and could otherwise race a fresh failing result.
    await github.rest.issues
      .removeLabel({ owner, repo, issue_number: prNumber, name: label.name })
      .catch((err) => {
        if (err.status !== 404) throw err;
      });
    core.info(`PR #${prNumber}: duplicate-code check passed.`);
  }

  // Recompute status now that "duplicate code" may have changed, so the PR
  // Merge Gate and overall status label reflect it immediately rather than
  // waiting for the next unrelated trigger.
  await syncLabels({ github, context, core, prNumber });
};

module.exports.analyzeReport = analyzeReport;
module.exports.CHECK_NAME = CHECK_NAME;
