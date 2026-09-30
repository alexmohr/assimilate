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

// The Rust reports (cargo-llvm-cov, and the e2e backend report ci.yml
// rewrites to the workspace) carry absolute paths under the CI checkout,
// while GitHub names a PR's files relative to the repository root.
const WORKSPACE = "/home/runner/work/assimilate/assimilate";

function lcovAt(sourcePath, hitsByLine) {
  const lines = [`SF:${sourcePath}`];
  for (const [lineNo, hits] of hitsByLine) lines.push(`DA:${lineNo},${hits}`);
  lines.push("end_of_record");
  return lines.join("\n");
}

async function analyzeReports(baseReport, prReport, github = noChangedFiles) {
  caseCount += 1;
  const dir = path.join(tmpRoot, `case-${caseCount}`);
  fs.mkdirSync(dir);
  const baseLcovPath = path.join(dir, "base.info");
  const prLcovPath = path.join(dir, "pr.info");
  fs.writeFileSync(baseLcovPath, `${baseReport}\n`);
  fs.writeFileSync(prLcovPath, `${prReport}\n`);
  return analyzeDiff({
    github,
    owner: "o",
    repo: "r",
    prNumber: 1,
    prLcovPath,
    baseLcovPath,
    workspaceRoot: WORKSPACE,
  });
}

function changedFiles(...files) {
  return { paginate: async () => files, rest: { pulls: { listFiles: {} } } };
}

test("new_rust_line_without_coverage_is_reported_despite_absolute_lcov_paths", async () => {
  const report = lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [
    [1, 1],
    [2, 0],
  ]);
  const github = changedFiles({
    filename: "crates/server/src/lib.rs",
    patch: "@@ -1,1 +1,2 @@\n fn covered() {}\n+fn uncovered() {}",
  });

  const result = await analyzeReports(report, report, github);

  assert.equal(result.ok, false, "a Rust line no test reaches must be flagged like any other");
  assert.match(result.findings.join("\n"), /crates\/server\/src\/lib\.rs:2 is new\/changed but has no test coverage/);
});

test("sub_precision_decrease_is_still_a_regression_with_absolute_paths", async () => {
  const hits = (covered) => Array.from({ length: 40000 }, (_, i) => [i + 1, i < covered ? 1 : 0]);
  const result = await analyzeReports(
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, hits(32416)),
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, hits(32415)),
  );

  assert.equal(result.ok, false, "normalizing paths must not open a tolerance");
  assert.match(result.findings.join("\n"), /Aggregate line coverage decreased/);
});

test("unit_and_e2e_records_of_one_file_are_one_file", async () => {
  // The same Rust file reported once by the unit run and once by the e2e run:
  // line 2 is reached only by e2e, and still counts as covered.
  const report = [
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [
      [1, 1],
      [2, 0],
    ]),
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [
      [1, 0],
      [2, 3],
    ]),
  ].join("\n");
  const github = changedFiles({
    filename: "crates/server/src/lib.rs",
    patch: "@@ -1,1 +1,2 @@\n fn covered() {}\n+fn covered_by_e2e() {}",
  });

  const result = await analyzeReports(report, report, github);

  assert.equal(result.ok, true);
  assert.deepEqual(result.findings, []);
});

test("toolchain_sources_outside_the_workspace_do_not_count", async () => {
  // The toolchain's own thread-local shims pick up whichever dependency
  // statics a run happened to initialise. No change in this repository can
  // cover or uncover them.
  const own = lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [[1, 1]]);
  const std = (hits) => lcovAt("/rustc/0123abcd/library/std/src/sys/thread_local/native/mod.rs", [[105, hits]]);

  const result = await analyzeReports(`${own}\n${std(1)}`, `${own}\n${std(0)}`);

  assert.equal(result.ok, true);
  assert.deepEqual(result.findings, []);
});

test("a_workspace_root_that_matches_nothing_fails_instead_of_ungating_rust", async () => {
  const report = lcovAt("/somewhere/else/crates/server/src/lib.rs", [
    [1, 1],
    [2, 1],
  ]);

  const result = await analyzeReports(report, report);

  assert.equal(result.ok, false, "dropping every Rust line must not read as a pass");
  assert.match(result.findings.join("\n"), /lies outside the workspace/);
});

test("an_aggregate_drop_names_the_lines_that_lost_coverage_outside_the_diff", async () => {
  const untouched = (hits) => lcovAt(`${WORKSPACE}/crates/server/src/tunnel.rs`, [[7, hits]]);
  const touched = lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [[1, 1]]);
  const github = changedFiles({
    filename: "crates/server/src/lib.rs",
    patch: "@@ -1,1 +1,1 @@\n-fn old() {}\n+fn covered() {}",
  });

  const result = await analyzeReports(`${touched}\n${untouched(2)}`, `${touched}\n${untouched(0)}`, github);

  assert.equal(result.ok, false, "a lost line fails the gate whether or not the PR touched it");
  assert.match(
    result.findings.join("\n"),
    /1 line\(s\) in files this PR does not change lost coverage: `crates\/server\/src\/tunnel\.rs:7`/,
  );
});

test("an_aggregate_drop_inside_the_diff_says_nothing_else_lost_coverage", async () => {
  const github = changedFiles({
    filename: "crates/server/src/lib.rs",
    patch: "@@ -1,2 +1,1 @@\n fn stays() {}\n-fn removed_and_covered() {}",
  });

  const result = await analyzeReports(
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [
      [1, 1],
      [2, 1],
      [3, 0],
    ]),
    lcovAt(`${WORKSPACE}/crates/server/src/lib.rs`, [
      [1, 1],
      [3, 0],
    ]),
    github,
  );

  assert.equal(result.ok, false);
  assert.match(result.findings.join("\n"), /No line in a file this PR leaves alone lost coverage/);
});
