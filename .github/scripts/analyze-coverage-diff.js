// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Deterministic "new/changed code must be covered" + "coverage must not
// regress" checks, so the review workflow doesn't have to spend a Claude
// turn re-deriving facts an lcov report already answers. See
// skills/review/SKILL.md.

const fs = require("fs");
const { parseLcov, totals } = require("./lib/lcov");

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

module.exports = async ({ github, owner, repo, prNumber, prLcovPath, baseLcovPath }) => {
  const findings = [];

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
};
