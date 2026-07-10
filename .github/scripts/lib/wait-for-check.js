// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Polls a named GitHub check run on a specific commit until it reaches
// status "completed", or a timeout elapses. Used by pre-review-checks.js to
// wait for the coverage-diff and duplicate-code stages' own check runs
// instead of trusting a label that might still be in flight - since both
// stages run in workflows that can race claude-review.yml on the same
// trigger event, only a definitively finished check's own conclusion is
// safe to gate a Claude review on.

async function waitForCheckCompletion(
  github,
  core,
  { owner, repo, ref, checkName, timeoutMs = 2 * 60 * 60 * 1000, pollIntervalMs = 30_000 },
) {
  const deadline = Date.now() + timeoutMs;

  for (;;) {
    const { data } = await github.rest.checks.listForRef({
      owner,
      repo,
      ref,
      check_name: checkName,
    });
    // Most recent first, in case this commit somehow accumulated more than
    // one run of the same check (e.g. a manual CI re-run with no new push).
    const run = data.check_runs
      .slice()
      .sort((a, b) => new Date(b.started_at) - new Date(a.started_at))[0];

    if (run && run.status === "completed") {
      return { completed: true, conclusion: run.conclusion };
    }

    if (Date.now() >= deadline) {
      core.warning(`Timed out after ${Math.round(timeoutMs / 60000)}m waiting for "${checkName}" on ${ref}.`);
      return { completed: false, conclusion: null };
    }

    core.info(`Waiting for "${checkName}" to complete on ${ref}...`);
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
}

module.exports = { waitForCheckCompletion };
