// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Derives a PR's status label from objective signals (CI conclusion + GitHub's
// native review decision) instead of relying on an agent or human remembering
// to apply it by hand, and publishes the same verdict as a "PR Merge Gate"
// check run so it can be made a required status check - merging a PR that
// isn't `ready to merge` then requires an explicit branch-protection bypass,
// not just human attentiveness. See skills/review/SKILL.md.

const CI_WORKFLOW_FILE = "ci.yml";

// Name of the check run that enforces the status label as a mergeability
// gate. Must be added as a required status check in branch protection for
// it to actually block merging - see skills/review/SKILL.md.
const GATE_CHECK_NAME = "PR Merge Gate";

const STATUS_LABELS = {
  CI_FAILING: {
    name: "ci failing",
    color: "d73a4a",
    description: "CI is red on the latest commit — cannot be merged.",
  },
  MERGE_CONFLICT: {
    name: "merge conflict",
    color: "b60205",
    description: "PR has real conflicts with the base branch — cannot be merged.",
  },
  PRECHECK_FAILED: {
    name: "precheck failed",
    color: "d93f0b",
    description:
      "A deterministic pre-review check failed (coverage or duplication) — set by code, not a reviewer.",
  },
  NEEDS_REVIEW: {
    name: "needs review",
    color: "fbca04",
    description: "No outstanding blocking review verdict yet.",
  },
  CHANGES_REQUESTED: {
    name: "changes requested",
    color: "e99695",
    description: "A reviewer requested changes.",
  },
  READY_TO_MERGE: {
    name: "ready to merge",
    color: "0e8a16",
    description: "CI is green and the PR has been approved.",
  },
};

const HUMAN_LABEL = {
  name: "needs human review",
  color: "5319e7",
  description: "Requires a human sign-off. Only a human may remove this label.",
};

// GitHub rejects an APPROVE review from the PR's own author (422: "Can not
// approve your own pull request"). In this repo the coding agent and the
// reviewing agent can share one GitHub account, so a real reviewDecision may
// never be reachable. These labels are the fallback verdict channel: set
// ONLY by the review workflow itself, never by any other agent, and treated
// as equivalent to a native review decision when no other-account review
// exists. See skills/review/SKILL.md.
const REVIEW_VERDICT_LABELS = {
  APPROVED: {
    name: "claude-approved",
    color: "0e8a16",
    description: "Claude's review verdict: approved. Set only by the review workflow.",
  },
  CHANGES_REQUESTED: {
    name: "claude-changes-requested",
    color: "e99695",
    description: "Claude's review verdict: changes requested. Set only by the review workflow.",
  },
};

// Paths where a change requires human sign-off even if an agent reviewed it.
// Kept in sync with the "Non-negotiable rules" in AGENTS.md and skills/security/SKILL.md.
const SENSITIVE_PATH_PATTERNS = [
  /^\.github\/workflows\//,
  /^\.github\/scripts\//,
  /(^|\/)auth[^/]*\.(rs|ts|vue)$/i,
  /(^|\/)crypto[^/]*\.rs$/i,
  /(^|\/)token[^/]*\.rs$/i,
  /(^|\/)passphrase[^/]*\.rs$/i,
  /(^|\/)ssh[_-]?agent/i,
  /^crates\/server\/migrations\//,
  /^deny\.toml$/,
  /^Cargo\.lock$/,
  /^frontend\/package-lock\.json$/,
  /^frontend\/\.npm-audit-allowlist\.json$/,
];

// Lines added by the diff that introduce a self-authorized suppression
// (forbidden by AGENTS.md without explicit human approval). deny.toml
// `ignore` entries are already covered by SENSITIVE_PATH_PATTERNS above,
// since any edit to that file matches on path alone.
const SUPPRESSION_LINE_PATTERNS = [/^\+\s*#!?\[allow\(/];

async function ensureLabelExists(github, owner, repo, label) {
  try {
    await github.rest.issues.createLabel({
      owner,
      repo,
      name: label.name,
      color: label.color,
      description: label.description,
    });
  } catch (err) {
    if (err.status !== 422) throw err; // 422 = label already exists
  }
}

async function resolveCiConclusion(github, owner, repo, headSha) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    owner,
    repo,
    workflow_id: CI_WORKFLOW_FILE,
    head_sha: headSha,
  });
  const latest = data.workflow_runs.sort(
    (a, b) => new Date(b.created_at) - new Date(a.created_at),
  )[0];
  return latest ? latest.conclusion : null;
}

// GitHub computes mergeable_state asynchronously and may still report
// "unknown" right after a push. One short retry (re-fetching the PR) covers
// most cases; if it's still unknown we don't block on it - the next trigger
// (another label sync, or CI completion) will re-check with a settled value.
async function resolveMergeableState(github, owner, repo, prNumber, initialState) {
  if (initialState !== "unknown") return initialState;
  await new Promise((resolve) => setTimeout(resolve, 3000));
  const { data } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  return data.mergeable_state;
}

async function resolveReviewDecision(github, owner, repo, prNumber) {
  const result = await github.graphql(
    `query($owner: String!, $repo: String!, $number: Int!) {
      repository(owner: $owner, name: $repo) {
        pullRequest(number: $number) { reviewDecision }
      }
    }`,
    { owner, repo, number: prNumber },
  );
  return result.repository.pullRequest.reviewDecision;
}

// A genuine other-account review always wins. Otherwise, fall back to the
// claude-approved / claude-changes-requested labels (see REVIEW_VERDICT_LABELS)
// for the same-account case where GitHub can't record a native verdict.
function resolveEffectiveReviewDecision(nativeDecision, existingLabels) {
  if (nativeDecision === "APPROVED" || nativeDecision === "CHANGES_REQUESTED") {
    return nativeDecision;
  }
  if (existingLabels.includes(REVIEW_VERDICT_LABELS.CHANGES_REQUESTED.name)) {
    return "CHANGES_REQUESTED";
  }
  if (existingLabels.includes(REVIEW_VERDICT_LABELS.APPROVED.name)) {
    return "APPROVED";
  }
  return nativeDecision;
}

async function needsHumanSignOff(github, owner, repo, prNumber) {
  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  if (files.some((f) => SENSITIVE_PATH_PATTERNS.some((p) => p.test(f.filename)))) {
    return true;
  }

  return files.some((f) =>
    (f.patch || "")
      .split("\n")
      .some((line) => SUPPRESSION_LINE_PATTERNS.some((p) => p.test(line))),
  );
}

async function createGateCheck(github, owner, repo, headSha, status, summary) {
  await github.rest.checks.create({
    owner,
    repo,
    name: GATE_CHECK_NAME,
    head_sha: headSha,
    status: "completed",
    conclusion: status.name === STATUS_LABELS.READY_TO_MERGE.name ? "success" : "failure",
    output: {
      title: status.name,
      summary,
    },
  });
}

module.exports = async ({ github, context, core, prNumber, eventAction }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const { data: pr } = await github.rest.pulls.get({
    owner,
    repo,
    pull_number: prNumber,
  });

  if (pr.draft) {
    core.info(`PR #${prNumber} is a draft — skipping label sync.`);
    return;
  }

  let existingLabels = pr.labels.map((l) => l.name);

  // New commits invalidate any prior verdict recorded via the fallback
  // labels, mirroring GitHub's own stale-review-dismissal behavior. Native
  // GitHub reviews already go stale/pending on their own; these labels don't,
  // so they must be cleared explicitly.
  if (eventAction === "synchronize") {
    // precheck failed is set by pre-review-checks.js against a specific commit;
    // like the review-verdict labels, a new push makes it stale.
    const staleVerdictLabels = [
      ...Object.values(REVIEW_VERDICT_LABELS).map((l) => l.name),
      STATUS_LABELS.PRECHECK_FAILED.name,
    ].filter((name) => existingLabels.includes(name));
    for (const name of staleVerdictLabels) {
      await github.rest.issues
        .removeLabel({ owner, repo, issue_number: prNumber, name })
        .catch((err) => {
          if (err.status !== 404) throw err;
        });
    }
    existingLabels = existingLabels.filter((name) => !staleVerdictLabels.includes(name));
  }

  const hasHumanLabel = existingLabels.includes(HUMAN_LABEL.name);
  const hasPrecheckFailed = existingLabels.includes(STATUS_LABELS.PRECHECK_FAILED.name);

  const [ciConclusion, mergeableState, nativeReviewDecision, autoNeedsHuman] = await Promise.all([
    resolveCiConclusion(github, owner, repo, pr.head.sha),
    resolveMergeableState(github, owner, repo, prNumber, pr.mergeable_state),
    resolveReviewDecision(github, owner, repo, prNumber),
    hasHumanLabel ? Promise.resolve(false) : needsHumanSignOff(github, owner, repo, prNumber),
  ]);
  const reviewDecision = resolveEffectiveReviewDecision(nativeReviewDecision, existingLabels);
  const needsHuman = hasHumanLabel || autoNeedsHuman;

  const ciFailed = ciConclusion !== null && !["success", "skipped", "neutral"].includes(ciConclusion);
  const mergeConflict = mergeableState === "dirty";

  let status;
  let summary;
  if (ciFailed) {
    status = STATUS_LABELS.CI_FAILING;
    summary = `CI is failing on the latest commit (conclusion: ${ciConclusion}) — cannot be merged until it's green.`;
  } else if (mergeConflict) {
    status = STATUS_LABELS.MERGE_CONFLICT;
    summary = "This PR has real conflicts with the base branch — rebase and resolve them before it can be merged.";
  } else if (hasPrecheckFailed) {
    status = STATUS_LABELS.PRECHECK_FAILED;
    summary =
      "A deterministic pre-review check (coverage or duplication) failed — see the automated pre-flight comment for specifics.";
  } else if (reviewDecision === "CHANGES_REQUESTED") {
    status = STATUS_LABELS.CHANGES_REQUESTED;
    summary = "A reviewer requested changes — address them and re-request review.";
  } else if (needsHuman) {
    // Even an approved, green PR is capped at "needs review" until a human
    // clears the sign-off gate by removing the label themselves.
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary =
      "This PR requires a human sign-off (`needs human review`). Only a human removing that label counts as sign-off.";
  } else if (reviewDecision === "APPROVED" && ciConclusion === "success") {
    status = STATUS_LABELS.READY_TO_MERGE;
    summary = "CI is green and the PR has been approved — ready to merge.";
  } else {
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary = "Awaiting an approving review and/or CI completion.";
  }

  core.info(
    `PR #${prNumber}: ci=${ciConclusion} mergeable=${mergeableState} precheckFailed=${hasPrecheckFailed} review=${reviewDecision} (native=${nativeReviewDecision}) needsHuman=${needsHuman} -> ${status.name}`,
  );

  const desired = [status.name];
  if (needsHuman) desired.push(HUMAN_LABEL.name);

  const toAdd = desired.filter((name) => !existingLabels.includes(name));
  const statusNames = Object.values(STATUS_LABELS).map((l) => l.name);
  const toRemove = statusNames.filter(
    (name) => name !== status.name && existingLabels.includes(name),
  );

  for (const name of toAdd) {
    const label = name === HUMAN_LABEL.name ? HUMAN_LABEL : status;
    await ensureLabelExists(github, owner, repo, label);
    await github.rest.issues.addLabels({
      owner,
      repo,
      issue_number: prNumber,
      labels: [name],
    });
  }

  for (const name of toRemove) {
    await github.rest.issues
      .removeLabel({ owner, repo, issue_number: prNumber, name })
      .catch((err) => {
        if (err.status !== 404) throw err;
      });
  }

  await createGateCheck(github, owner, repo, pr.head.sha, status, summary);
};

// Exported so pre-review-checks.js can (a) invoke this exact sync as its own
// "is CI green / does this conflict" check instead of re-deriving it, and
// (b) reuse the PRECHECK_FAILED label definition/helper without duplicating
// them and risking drift.
module.exports.STATUS_LABELS = STATUS_LABELS;
module.exports.REVIEW_VERDICT_LABELS = REVIEW_VERDICT_LABELS;
module.exports.ensureLabelExists = ensureLabelExists;
