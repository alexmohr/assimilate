// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Picks the `main` CI run whose coverage a PR is compared against.
//
// A PR's CI builds GitHub's test merge commit - the PR head merged into
// whatever `main` was at that moment - so the only baseline that measures
// "this PR's change and nothing else" is `main`'s own run for that exact
// commit, the merge commit's first parent. "The latest successful `main` run"
// is usually a different commit: `main` moved on after the PR's CI ran, or its
// newest run failed (on any job, coverage-unrelated or not) and an older one
// was picked instead. Every covered/uncovered line `main` gained or lost in
// between then reads as the PR's doing, which is how a PR touching nothing
// instrumented (#366) or covering every line it changed (#404) still showed
// an aggregate drop.
//
// ci.yml records that first parent next to the PR's lcov report
// (`coverage-base.sha`). When it is missing (an artifact older than that
// step) or `main` has no finished run with coverage for it yet, this falls
// back to the latest successful `main` run - the comparison this gate always
// made - and says so, so a result against an approximate baseline is never
// mistaken for an exact one.

const fs = require("fs");

const COVERAGE_ARTIFACT = "coverage-final";

const BaselineKind = Object.freeze({
  EXACT: "exact",
  LATEST_MAIN: "latest-main",
});

const SHA_PATTERN = /^[0-9a-f]{40}$/;

function readBaseSha(filePath) {
  if (!fs.existsSync(filePath)) return null;
  const sha = fs.readFileSync(filePath, "utf8").trim().toLowerCase();
  return SHA_PATTERN.test(sha) ? sha : null;
}

function newestFirst(runs) {
  return [...runs].sort((a, b) => new Date(b.created_at) - new Date(a.created_at));
}

async function hasCoverageArtifact(github, owner, repo, runId) {
  const { data } = await github.rest.actions.listWorkflowRunArtifacts({
    owner,
    repo,
    run_id: runId,
    name: COVERAGE_ARTIFACT,
  });
  return data.artifacts.some((artifact) => artifact.name === COVERAGE_ARTIFACT && !artifact.expired);
}

// A run for the exact base commit is usable once it has finished and produced
// the coverage artifact - whether or not some other job in it failed. The
// artifact only exists if the coverage and e2e jobs themselves succeeded
// (coveralls-finish needs both), so its presence is the real criterion;
// requiring a green run overall would throw away a valid measurement over,
// say, a clippy failure.
async function exactBaseRun(github, owner, repo, defaultBranch, baseSha) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    owner,
    repo,
    workflow_id: "ci.yml",
    branch: defaultBranch,
    event: "push",
    head_sha: baseSha,
  });
  const finished = newestFirst(data.workflow_runs).filter(
    (run) => run.head_sha === baseSha && run.status === "completed",
  );
  for (const run of finished) {
    if (await hasCoverageArtifact(github, owner, repo, run.id)) return run;
  }
  return null;
}

async function latestSuccessfulMainRun(github, owner, repo, defaultBranch) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    owner,
    repo,
    workflow_id: "ci.yml",
    branch: defaultBranch,
    event: "push",
    status: "success",
  });
  return newestFirst(data.workflow_runs)[0] || null;
}

// Returns `{ run, kind, note }`; `run` is null only when `main` has no
// successful run at all.
async function selectBaselineRun({ github, owner, repo, defaultBranch, baseSha }) {
  if (baseSha) {
    const run = await exactBaseRun(github, owner, repo, defaultBranch, baseSha);
    if (run) {
      return {
        run,
        kind: BaselineKind.EXACT,
        note: `Compared against \`${defaultBranch}\` at ${baseSha.slice(0, 7)}, the commit this PR's CI merged onto.`,
      };
    }
  }

  const run = await latestSuccessfulMainRun(github, owner, repo, defaultBranch);
  const why = baseSha
    ? `\`${defaultBranch}\` has no finished CI run with coverage for ${baseSha.slice(0, 7)}, the commit this PR's CI merged onto, yet`
    : "this PR's coverage artifact does not record which commit its CI merged onto";
  return {
    run,
    kind: BaselineKind.LATEST_MAIN,
    note:
      `${why}, so the baseline is the latest successful \`${defaultBranch}\` run` +
      (run ? ` (${run.head_sha.slice(0, 7)})` : "") +
      ". Coverage changes on `main` between the two commits show up here as this PR's.",
  };
}

module.exports = { BaselineKind, COVERAGE_ARTIFACT, readBaseSha, selectBaselineRun };
