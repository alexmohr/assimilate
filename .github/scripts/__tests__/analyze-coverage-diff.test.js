// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Run with `node --test .github/scripts/__tests__/` from the repo root (CI's
// "CI Scripts (node --test)" job). Uses node's built-in test runner so the
// `.github/scripts` helpers are testable without adding a dependency or a
// package.json outside frontend/.

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { analyzeDiff, aggregateRegressed } = require("../analyze-coverage-diff.js");

// Builds an lcov report from [path, [[lineNo, hits], ...]] entries.
function lcov(records) {
  return records
    .map(([file, lines]) => [`SF:${file}`, ...lines.map(([no, hits]) => `DA:${no},${hits}`), "end_of_record"].join("\n"))
    .join("\n");
}

function writeLcov(dir, name, records) {
  const file = path.join(dir, name);
  fs.writeFileSync(file, lcov(records));
  return file;
}

// Only `percent` is read by aggregateRegressed; the counts ride along for
// the finding message.
function totals(percent) {
  return { percent, coveredLines: 0, totalLines: 0 };
}

test("aggregateRegressed ignores a drop smaller than the reported precision", () => {
  // The exact shape that blocked PR #485: both sides print as "81.04%", so a
  // finding here could only ever read "decreased from 81.04% to 81.04%".
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(81.0375)), false);
});

test("aggregateRegressed catches a drop the message can actually show", () => {
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(81.03)), true);
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(75.0)), true);
});

test("aggregateRegressed treats equal and improved coverage as no regression", () => {
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(81.04)), false);
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(81.05)), false);
  assert.strictEqual(aggregateRegressed(totals(81.04), totals(99.9)), false);
});

// A file of `total` executable lines, the first `covered` of them hit. Big
// enough that one line is worth less than the reported precision, which is
// the only way to reproduce the jitter this check used to fail on.
function report(covered, total) {
  const lines = [];
  for (let no = 1; no <= total; no += 1) lines.push([no, no <= covered ? 1 : 0]);
  return [["crates/server/src/a.rs", lines]];
}

test("analyzeDiff reports no aggregate finding for sub-precision jitter", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-diff-"));
  // 81.04% vs 81.0375% - one line out of 40k. Both print as "81.04%", so the
  // old raw-float comparison produced the "decreased from 81.04% to 81.04%"
  // finding that blocked PR #485.
  const baseLcovPath = writeLcov(dir, "base.info", report(32416, 40000));
  const prLcovPath = writeLcov(dir, "pr.info", report(32415, 40000));

  const github = { paginate: async () => [], rest: { pulls: { listFiles: () => {} } } };
  const result = await analyzeDiff({ github, owner: "o", repo: "r", prNumber: 1, prLcovPath, baseLcovPath });

  assert.deepStrictEqual(result.findings, []);
  assert.strictEqual(result.ok, true);
});

test("analyzeDiff reports a visible aggregate regression, with line counts", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-diff-"));
  // 81.04% -> 80.00%, a full percentage point: a real regression the message
  // can show, so it must still fail.
  const baseLcovPath = writeLcov(dir, "base.info", report(32416, 40000));
  const prLcovPath = writeLcov(dir, "pr.info", report(32000, 40000));

  const github = { paginate: async () => [], rest: { pulls: { listFiles: () => {} } } };
  const result = await analyzeDiff({ github, owner: "o", repo: "r", prNumber: 1, prLcovPath, baseLcovPath });

  assert.strictEqual(result.ok, false);
  assert.strictEqual(result.findings.length, 1);
  assert.match(result.findings[0], /decreased from 81\.04% \(main, 32416\/40000 lines\)/);
  assert.match(result.findings[0], /to 80\.00% \(this PR, 32000\/40000 lines\)/);
});

test("analyzeDiff still flags a new line with no coverage", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-diff-"));
  const records = [
    [
      "crates/server/src/a.rs",
      [
        [1, 1],
        [2, 0],
      ],
    ],
  ];
  const baseLcovPath = writeLcov(dir, "base.info", records);
  const prLcovPath = writeLcov(dir, "pr.info", records);

  const github = {
    paginate: async () => [
      {
        filename: "crates/server/src/a.rs",
        patch: "@@ -1,1 +1,2 @@\n line one\n+line two\n",
      },
    ],
    rest: { pulls: { listFiles: () => {} } },
  };
  const result = await analyzeDiff({ github, owner: "o", repo: "r", prNumber: 1, prLcovPath, baseLcovPath });

  assert.strictEqual(result.ok, false);
  assert.deepStrictEqual(result.findings, ["crates/server/src/a.rs:2 is new/changed but has no test coverage."]);
});

test("analyzeDiff passes when either lcov artifact is missing", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-diff-"));
  const github = { paginate: async () => [], rest: { pulls: { listFiles: () => {} } } };
  const result = await analyzeDiff({
    github,
    owner: "o",
    repo: "r",
    prNumber: 1,
    prLcovPath: path.join(dir, "missing-pr.info"),
    baseLcovPath: path.join(dir, "missing-base.info"),
  });

  assert.strictEqual(result.ok, true);
  assert.deepStrictEqual(result.findings, []);
});
