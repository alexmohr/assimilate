// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Pins the aggregate coverage gate to ZERO TOLERANCE: any decrease at all is
// a regression, however small.
//
// This exists because the tempting "fix" for a noisy gate is an epsilon - let
// coverage drop by a hundredth, or compare at the two decimals the message
// prints - and that turns "aggregate coverage must not drop" into a
// suggestion. Coverage noise is a determinism bug in the tests (see #489,
// which removed the last known source of it by tracking fire-and-forget
// spawns); it is not a reason to widen the gate. If a future change adds a
// tolerance, `sub_precision_decrease_is_still_a_regression` below fails, so
// widening the gate requires visibly editing a test rather than quietly
// editing one comparison.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { after, test } = require("node:test");

const { analyzeDiff } = require("../analyze-coverage-diff.js");

// An lcov report with `total` instrumented lines in one file, the first
// `covered` of them hit. Enough for the aggregate percentage, which is all
// these cases turn on.
function lcov(covered, total) {
  const lines = ["SF:src/example.rs"];
  for (let i = 1; i <= total; i += 1) lines.push(`DA:${i},${i <= covered ? 1 : 0}`);
  lines.push("end_of_record", "");
  return lines.join("\n");
}

// One directory for the whole suite, removed when it finishes, so repeated
// local runs don't leave a trail of coverage-gate-* directories behind.
const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-gate-"));
after(() => fs.rmSync(tmpRoot, { recursive: true, force: true }));

let caseCount = 0;

function writeLcovPair(baseCounts, prCounts) {
  caseCount += 1;
  const dir = path.join(tmpRoot, `case-${caseCount}`);
  fs.mkdirSync(dir);
  const basePath = path.join(dir, "base.info");
  const prPath = path.join(dir, "pr.info");
  fs.writeFileSync(basePath, lcov(...baseCounts));
  fs.writeFileSync(prPath, lcov(...prCounts));
  return { basePath, prPath };
}

// `analyzeDiff` also walks the PR's changed files to flag uncovered new
// lines; these cases are about the aggregate check, so the PR touches
// nothing.
const noChangedFiles = { paginate: async () => [], rest: { pulls: { listFiles: {} } } };

async function analyze(baseCounts, prCounts, github = noChangedFiles) {
  const { basePath, prPath } = writeLcovPair(baseCounts, prCounts);
  return analyzeDiff({
    github,
    owner: "o",
    repo: "r",
    prNumber: 1,
    prLcovPath: prPath,
    baseLcovPath: basePath,
  });
}

test("sub_precision_decrease_is_still_a_regression", async () => {
  // 32416/40000 = 81.04%, 32415/40000 = 81.0375%. Both print as "81.04%", so
  // any comparison done at the reported precision - or with an epsilon of a
  // hundredth of a point - would call this equal and let it through.
  const result = await analyze([32416, 40000], [32415, 40000]);

  assert.equal(result.ok, false, "a one-line drop must fail the gate, however small");
  assert.match(result.findings.join("\n"), /Aggregate line coverage decreased/);
});

test("visible_decrease_is_a_regression", async () => {
  const result = await analyze([32416, 40000], [32000, 40000]);

  assert.equal(result.ok, false);
  assert.match(result.findings.join("\n"), /Aggregate line coverage decreased/);
});

test("unchanged_coverage_passes", async () => {
  const result = await analyze([32416, 40000], [32416, 40000]);

  assert.equal(result.ok, true, "equal coverage is not a decrease");
  assert.deepEqual(result.findings, []);
});

test("increased_coverage_passes", async () => {
  const result = await analyze([32416, 40000], [32417, 40000]);

  assert.equal(result.ok, true);
  assert.deepEqual(result.findings, []);
});

test("missing_artifact_passes_rather_than_blocking", async () => {
  const { basePath } = writeLcovPair([32416, 40000], [32416, 40000]);
  const result = await analyzeDiff({
    github: noChangedFiles,
    owner: "o",
    repo: "r",
    prNumber: 1,
    prLcovPath: path.join(path.dirname(basePath), "absent.info"),
    baseLcovPath: basePath,
  });

  assert.equal(result.ok, true, "a PR predating the coverage artifact is not a failure");
});

test("new_line_without_coverage_is_reported", async () => {
  const github = {
    paginate: async () => [
      {
        filename: "src/example.rs",
        patch: "@@ -1,0 +1,1 @@\n+let uncovered = true;",
      },
    ],
    rest: { pulls: { listFiles: {} } },
  };
  // Line 1 is covered in the base pair above, so make it uncovered here by
  // covering nothing at all.
  const result = await analyze([0, 10], [0, 10], github);

  assert.equal(result.ok, false);
  assert.match(result.findings.join("\n"), /src\/example\.rs:1 is new\/changed but has no test coverage/);
});
