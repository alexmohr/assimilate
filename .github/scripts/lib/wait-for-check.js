// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Polls every check run on a specific commit until each one (other than the
// ones explicitly excluded) reaches status "completed", or a timeout
// elapses, then reports whether any of them failed. Used by
// pre-review-checks.js to gate Claude on the commit's actual, finished
// state instead of a fixed list of named checks - any check that later gets
// added to the pipeline is covered automatically, with no need to update
// the gate script. `skipped`/`neutral` conclusions count as passing, same
// as sync-pr-labels.js's own CI-conclusion handling. See
// skills/review/SKILL.md for the full reasoning.

const PASSING_CONCLUSIONS = ["success", "skipped", "neutral"];

async function waitForAllChecks(
  github,
  core,
  { owner, repo, ref, excludeNames = [], timeoutMs = 2 * 60 * 60 * 1000, pollIntervalMs = 30_000 },
) {
  const deadline = Date.now() + timeoutMs;

  for (;;) {
    const runs = await github.paginate(
      github.rest.checks.listForRef,
      { owner, repo, ref, per_page: 100 },
      (response) => response.data.check_runs,
    );
    const relevant = runs.filter((run) => !excludeNames.includes(run.name));
    const pending = relevant.filter((run) => run.status !== "completed");

    if (pending.length === 0) {
      const failed = relevant.filter((run) => !PASSING_CONCLUSIONS.includes(run.conclusion));
      return { completed: true, ok: failed.length === 0, failed: failed.map((r) => r.name) };
    }

    if (Date.now() >= deadline) {
      const pendingNames = pending.map((r) => r.name).join(", ");
      core.warning(`Timed out after ${Math.round(timeoutMs / 60000)}m waiting for checks on ${ref}: ${pendingNames}`);
      return { completed: false, ok: false, pending: pending.map((r) => r.name) };
    }

    core.info(`Waiting for checks to complete on ${ref}: ${pending.map((r) => r.name).join(", ")}`);
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
}

module.exports = { waitForAllChecks };
