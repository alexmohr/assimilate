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
// (`coverage-base.sha`). That commit is often one CI never runs on: the
// screenshot capture job follows every merge with a `[skip ci]` recapture
// commit, so `main`'s tip is usually one. Its coverage is its parent's - CI
// was skipped, nothing was measured - so the search walks down through
// `[skip ci]` commits to the nearest one CI ran on.
//
// When the base is missing (an artifact older than that step) or no such run
// has finished with coverage yet, this falls back to the latest successful
// `main` run - the comparison this gate always made - and says so, so a
// result against an approximate baseline is never mistaken for an exact one.

const fs = require("fs");

const COVERAGE_ARTIFACT = "coverage-final";

const BaselineKind = Object.freeze({
  EXACT: "exact",
  SKIP_CI_ANCESTOR: "skip-ci-ancestor",
  LATEST_MAIN: "latest-main",
});

// How far down a run of `[skip ci]` commits the search goes. One recapture
// per merge means one in practice; the cap only keeps a pathological history
// from costing a request per commit.
const MAX_SKIP_CI_HOPS = 10;

// The tokens GitHub Actions honours in a push's head commit message.
const SKIP_CI_PATTERN = /\[(skip ci|ci skip|no ci|skip actions|actions skip)\]/i;

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

// The parent of a `[skip ci]` commit, or null for any other commit: only a
// commit CI was told to skip is one whose measurement lives on its parent.
async function skipCiParent(github, owner, repo, sha) {
  const { data } = await github.rest.repos.getCommit({ owner, repo, ref: sha });
  if (!SKIP_CI_PATTERN.test(data.commit.message)) return null;
  return data.parents.length > 0 ? data.parents[0].sha : null;
}

// Filters on the conclusion here rather than with the API's `status` filter:
// that filter leaves out a recent run that is completed and green (seen with
// `main`'s newest run, which a list without it returns first), and then the
// fallback lands on a run a day or more old.
async function latestSuccessfulMainRun(github, owner, repo, defaultBranch) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    owner,
    repo,
    workflow_id: "ci.yml",
    branch: defaultBranch,
    event: "push",
  });
  const green = data.workflow_runs.filter(
    (run) => run.status === "completed" && run.conclusion === "success",
  );
  return newestFirst(green)[0] || null;
}

// Returns `{ run, kind, note }`; `run` is null only when `main` has no
// successful run at all.
async function selectBaselineRun({ github, owner, repo, defaultBranch, baseSha }) {
  if (baseSha) {
    let sha = baseSha;
    for (let hops = 0; sha && hops <= MAX_SKIP_CI_HOPS; hops += 1) {
      const run = await exactBaseRun(github, owner, repo, defaultBranch, sha);
      if (run && hops === 0) {
        return {
          run,
          kind: BaselineKind.EXACT,
          note: `Compared against \`${defaultBranch}\` at ${baseSha.slice(0, 7)}, the commit this PR's CI merged onto.`,
        };
      }
      if (run) {
        return {
          run,
          kind: BaselineKind.SKIP_CI_ANCESTOR,
          note:
            `Compared against \`${defaultBranch}\` at ${sha.slice(0, 7)}: this PR's CI merged onto ` +
            `${baseSha.slice(0, 7)}, which CI skipped, so its coverage is that of the nearest ` +
            `commit below it that CI ran on (${hops} \`[skip ci]\` commit${hops === 1 ? "" : "s"} down).`,
        };
      }
      sha = await skipCiParent(github, owner, repo, sha);
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

module.exports = { BaselineKind, COVERAGE_ARTIFACT, MAX_SKIP_CI_HOPS, readBaseSha, selectBaselineRun };
