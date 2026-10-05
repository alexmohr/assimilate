// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Pins which `main` run a PR's coverage is compared against: `main` at the
// exact commit the PR's CI merged onto whenever that measurement exists, and
// the old "latest successful `main` run" only as a labelled fallback.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { after, test } = require("node:test");

const { BaselineKind, readBaseSha, selectBaselineRun } = require("../lib/coverage-baseline.js");

const BASE_SHA = "a".repeat(40);
const NEWER_SHA = "b".repeat(40);

function run(id, headSha, { status = "completed", conclusion = "success", createdAt = "2026-09-30T10:00:00Z" } = {}) {
  return { id, head_sha: headSha, status, conclusion, created_at: createdAt };
}

// A fake of the two Actions endpoints the selection reads. `runs` is every
// `main` push run; `artifactsByRun` maps a run id to its artifacts.
function fakeGithub(runs, artifactsByRun = {}) {
  const calls = [];
  return {
    calls,
    rest: {
      actions: {
        listWorkflowRuns: async (params) => {
          calls.push(params);
          const matching = runs.filter(
            (r) =>
              (params.head_sha === undefined || r.head_sha === params.head_sha) &&
              (params.status === undefined || r.conclusion === params.status),
          );
          return { data: { workflow_runs: matching } };
        },
        listWorkflowRunArtifacts: async ({ run_id: runId, name }) => ({
          data: { artifacts: (artifactsByRun[runId] || []).filter((a) => a.name === name) },
        }),
      },
    },
  };
}

const coverage = [{ name: "coverage-final", expired: false }];

function select(github, baseSha) {
  return selectBaselineRun({ github, owner: "o", repo: "r", defaultBranch: "main", baseSha });
}

test("the_run_for_the_merge_base_is_preferred_over_a_newer_main_run", async () => {
  const github = fakeGithub(
    [run(1, BASE_SHA, { createdAt: "2026-09-30T08:00:00Z" }), run(2, NEWER_SHA, { createdAt: "2026-09-30T09:00:00Z" })],
    { 1: coverage, 2: coverage },
  );

  const selected = await select(github, BASE_SHA);

  assert.equal(selected.run.id, 1, "main moving on after the PR's CI must not move the baseline");
  assert.equal(selected.kind, BaselineKind.EXACT);
  assert.match(selected.note, /aaaaaaa/);
});

test("a_merge_base_run_with_an_unrelated_failure_still_counts", async () => {
  // A clippy failure fails the run, but the coverage jobs still produced the
  // artifact - and it is still the only measurement of that exact commit.
  const github = fakeGithub(
    [run(1, BASE_SHA, { conclusion: "failure" }), run(2, NEWER_SHA)],
    { 1: coverage, 2: coverage },
  );

  const selected = await select(github, BASE_SHA);

  assert.equal(selected.run.id, 1);
  assert.equal(selected.kind, BaselineKind.EXACT);
});

test("an_unfinished_merge_base_run_falls_back_to_latest_main", async () => {
  const github = fakeGithub(
    [run(1, BASE_SHA, { status: "in_progress", conclusion: null }), run(2, NEWER_SHA)],
    { 2: coverage },
  );

  const selected = await select(github, BASE_SHA);

  assert.equal(selected.run.id, 2);
  assert.equal(selected.kind, BaselineKind.LATEST_MAIN);
  assert.match(selected.note, /no finished CI run with coverage for aaaaaaa/);
});

test("a_merge_base_run_without_coverage_falls_back_to_latest_main", async () => {
  const github = fakeGithub(
    [run(1, BASE_SHA, { createdAt: "2026-09-30T08:00:00Z" }), run(2, NEWER_SHA, { createdAt: "2026-09-30T09:00:00Z" })],
    { 1: [{ name: "coverage-final", expired: true }], 2: coverage },
  );

  const selected = await select(github, BASE_SHA);

  assert.equal(selected.run.id, 2);
  assert.equal(selected.kind, BaselineKind.LATEST_MAIN);
});

test("an_artifact_without_a_recorded_base_falls_back_to_latest_main", async () => {
  const github = fakeGithub(
    [run(1, BASE_SHA, { createdAt: "2026-09-30T08:00:00Z" }), run(2, NEWER_SHA, { createdAt: "2026-09-30T09:00:00Z" })],
    { 1: coverage, 2: coverage },
  );

  const selected = await select(github, null);

  assert.equal(selected.run.id, 2, "the fallback is the newest successful main run, as before");
  assert.equal(selected.kind, BaselineKind.LATEST_MAIN);
  assert.match(selected.note, /does not record which commit/);
  assert.ok(
    github.calls.every((params) => params.head_sha === undefined),
    "no base commit means no lookup by one",
  );
});

test("no_successful_main_run_at_all_selects_nothing", async () => {
  const selected = await select(fakeGithub([]), null);

  assert.equal(selected.run, null);
  assert.equal(selected.kind, BaselineKind.LATEST_MAIN);
});

const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "coverage-baseline-"));
after(() => fs.rmSync(tmpRoot, { recursive: true, force: true }));

test("read_base_sha_accepts_only_a_full_commit_id", () => {
  const write = (name, content) => {
    const filePath = path.join(tmpRoot, name);
    fs.writeFileSync(filePath, content);
    return filePath;
  };

  assert.equal(readBaseSha(write("ok.sha", `${BASE_SHA.toUpperCase()}\n`)), BASE_SHA);
  assert.equal(readBaseSha(write("empty.sha", "")), null, "a failed `git cat-file` leaves an empty file");
  assert.equal(readBaseSha(write("short.sha", "abc1234")), null);
  assert.equal(readBaseSha(write("junk.sha", "main; rm -rf /")), null);
  assert.equal(readBaseSha(path.join(tmpRoot, "absent.sha")), null);
});
