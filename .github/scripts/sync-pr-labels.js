// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Derives a PR's status label from objective signals (CI conclusion + GitHub's
// native review decision) instead of relying on an agent or human remembering
// to apply it by hand. See skills/review/SKILL.md for how agents are expected
// to interact with these labels.

const CI_WORKFLOW_FILE = "ci.yml";

const STATUS_LABELS = {
  CI_FAILING: {
    name: "ci failing",
    color: "d73a4a",
    description: "CI is red on the latest commit — cannot be merged.",
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

module.exports = async ({ github, context, core, prNumber }) => {
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

  const existingLabels = pr.labels.map((l) => l.name);
  const hasHumanLabel = existingLabels.includes(HUMAN_LABEL.name);

  const [ciConclusion, reviewDecision, autoNeedsHuman] = await Promise.all([
    resolveCiConclusion(github, owner, repo, pr.head.sha),
    resolveReviewDecision(github, owner, repo, prNumber),
    hasHumanLabel ? Promise.resolve(false) : needsHumanSignOff(github, owner, repo, prNumber),
  ]);
  const needsHuman = hasHumanLabel || autoNeedsHuman;

  const ciFailed = ciConclusion !== null && !["success", "skipped", "neutral"].includes(ciConclusion);

  let status;
  if (ciFailed) {
    status = STATUS_LABELS.CI_FAILING;
  } else if (reviewDecision === "CHANGES_REQUESTED") {
    status = STATUS_LABELS.CHANGES_REQUESTED;
  } else if (needsHuman) {
    // Even an approved, green PR is capped at "needs review" until a human
    // clears the sign-off gate by removing the label themselves.
    status = STATUS_LABELS.NEEDS_REVIEW;
  } else if (reviewDecision === "APPROVED" && ciConclusion === "success") {
    status = STATUS_LABELS.READY_TO_MERGE;
  } else {
    status = STATUS_LABELS.NEEDS_REVIEW;
  }

  core.info(
    `PR #${prNumber}: ci=${ciConclusion} review=${reviewDecision} needsHuman=${needsHuman} -> ${status.name}`,
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
};
