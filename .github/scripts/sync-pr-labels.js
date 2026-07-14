// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Derives a PR's status label from objective signals (CI conclusion + GitHub's
// native review decision) instead of relying on an agent or human remembering
// to apply it by hand, and publishes the same verdict as a "PR Merge Gate"
// check run so it can be made a required status check - merging a PR that
// isn't `ready to merge` then requires an explicit branch-protection bypass,
// not just human attentiveness. See skills/review/SKILL.md.

const { waitForAllChecks } = require("./lib/wait-for-check");

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
      "A deterministic pre-review stage failed — purely derived from the `coverage failed` / `duplicate code` labels below, never set directly.",
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

// DUPLICATE_CODE_LABEL and COVERAGE_LABEL are each set/cleared solely by
// their own standalone workflow - duplicate-code-check.yml via
// analyze-duplication.js, and coverage-diff-check.yml via
// analyze-coverage-diff.js - two independent deterministic pre-review
// stages, neither waiting on the other or on the Claude pipeline. Both are
// deliberately NOT members of STATUS_LABELS: two checks failing at once
// need to both stay visible, which a single mutually-exclusive status slot
// can't do, and PRECHECK_FAILED above is purely a derived umbrella computed
// fresh from these two every sync - never read back as an input itself, so
// it can't go stale or get self-reinforcing. Each label also owns its own
// add/remove lifecycle end to end (its workflow sets or clears it on every
// run based on that run's fresh result) rather than being blindly cleared
// here on every push - both workflows react to the same "synchronize"/
// "workflow_run" events this script does, so a blind clear here could race
// a fresh finding from the other workflow and wipe it.
const DUPLICATE_CODE_LABEL = {
  name: "duplicate code",
  color: "c5def5",
  description: "jscpd found duplicate code touching this PR's changed files — set by code, not a reviewer.",
};

const COVERAGE_LABEL = {
  name: "coverage failed",
  color: "f9d0c4",
  description:
    "The coverage-diff pre-review check failed (new/changed lines uncovered, or aggregate coverage regressed) — set by code, not a reviewer.",
};

// Set/cleared solely by claude-review.yml itself, mirroring the
// DUPLICATE_CODE_LABEL/COVERAGE_LABEL pattern above: owns its own full
// add/remove lifecycle (set when the "Run Claude review" step errors out -
// auth, quota, action failure, anything short of producing a verdict; cleared
// the moment a subsequent attempt succeeds), rather than being touched by the
// generic synchronize-triggered clear in this file. A review attempt that
// failed to run is a materially different state from "no review happened
// yet" (which `ready to merge` already tolerates by design - see
// skills/review/SKILL.md) - this label exists so that difference actually
// blocks the gate instead of silently falling through to a green PR.
const CLAUDE_REVIEW_FAILED_LABEL = {
  name: "claude review failed",
  color: "e99695",
  description:
    "The last automated Claude review attempt errored (auth/quota/action failure) instead of producing a verdict — set only by claude-review.yml.",
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
  /^\.pre-commit-config\.ya?ml$/,
  /^\.devcontainer\//,
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

// A PR that talks about security in its own title/body, or that closes an
// issue which itself talks about security (title, body, or a "security"
// label), gets the same sign-off gate as a sensitive-path change - the
// judgment call of whether it's actually security-relevant belongs to a
// human, not a keyword match, so this only widens the net, never narrows it.
const SECURITY_MENTION_PATTERN = /\bsecurity\b/i;
const CLOSING_KEYWORD_PATTERN = /\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)/gi;

function extractClosingIssueNumbers(text) {
  if (!text) return [];
  const numbers = new Set();
  let match;
  // Reset lastIndex - this regex has the global flag and is module-level,
  // so a prior exec() elsewhere could leave it mid-string otherwise.
  CLOSING_KEYWORD_PATTERN.lastIndex = 0;
  while ((match = CLOSING_KEYWORD_PATTERN.exec(text)) !== null) {
    numbers.add(Number(match[1]));
  }
  return [...numbers];
}

async function closesSecurityIssue(github, owner, repo, prBody) {
  const issueNumbers = extractClosingIssueNumbers(prBody);
  if (issueNumbers.length === 0) return false;

  const issues = await Promise.all(
    issueNumbers.map((issue_number) =>
      github.rest.issues.get({ owner, repo, issue_number }).catch(() => null),
    ),
  );

  return issues.some((issue) => {
    if (!issue) return false; // not a real issue number, or inaccessible - not our call to make
    const { title, body, labels } = issue.data;
    if (SECURITY_MENTION_PATTERN.test(title) || SECURITY_MENTION_PATTERN.test(body || "")) {
      return true;
    }
    return (labels || []).some((label) =>
      SECURITY_MENTION_PATTERN.test((typeof label === "string" ? label : label.name) || ""),
    );
  });
}

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

// GitHub's reviewDecision does NOT go stale on its own for a
// CHANGES_REQUESTED verdict the way the code comment at this function's call
// site once assumed - that assumption only holds for APPROVED (and even then
// only with "dismiss stale approvals" branch protection enabled). A reviewer
// who requested changes on an old commit keeps blocking forever unless they
// personally submit a new review, even after every finding they raised has
// been fixed in a later commit they've never looked at. Returns true only if
// at least one of the CHANGES_REQUESTED reviews behind the current decision
// was actually submitted against the PR's current head commit - i.e. a real
// reviewer has seen this exact code and still wants changes, as opposed to
// an old review of a commit that's since moved on.
async function changesRequestedIsCurrent(github, owner, repo, prNumber, headSha) {
  const reviews = await github.paginate(github.rest.pulls.listReviews, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });
  // Latest review per user, mirroring how GitHub itself computes
  // reviewDecision (only each reviewer's most recent submission counts).
  const latestByUser = new Map();
  for (const r of reviews) {
    if (!r.user) continue;
    const existing = latestByUser.get(r.user.login);
    if (!existing || new Date(r.submitted_at) > new Date(existing.submitted_at)) {
      latestByUser.set(r.user.login, r);
    }
  }
  return [...latestByUser.values()].some(
    (r) => r.state === "CHANGES_REQUESTED" && r.commit_id === headSha,
  );
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

async function needsHumanSignOff(github, owner, repo, prNumber, pr) {
  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });

  if (files.some((f) => SENSITIVE_PATH_PATTERNS.some((p) => p.test(f.filename)))) {
    return true;
  }

  if (
    files.some((f) =>
      (f.patch || "")
        .split("\n")
        .some((line) => SUPPRESSION_LINE_PATTERNS.some((p) => p.test(line))),
    )
  ) {
    return true;
  }

  if (
    SECURITY_MENTION_PATTERN.test(pr.title || "") ||
    SECURITY_MENTION_PATTERN.test(pr.body || "")
  ) {
    return true;
  }

  return closesSecurityIssue(github, owner, repo, pr.body);
}

// Whether a human's removal of `needs human review` still stands for the
// PR's current head commit. Nothing in this codebase ever calls removeLabel
// on HUMAN_LABEL (grep it - only ever added, in the toAdd loop below), so
// any "unlabeled" event in its history is a human's own sign-off action via
// the GitHub UI, not something this automation did. Without this check,
// needsHumanSignOff() above would simply re-derive "true" from the same
// unchanged file patterns on the very next sync and the label would
// reappear immediately - the additive-only, human-clears-it-only contract
// documented in skills/review/SKILL.md would be pure documentation with no
// code behind it.
//
// Scoped to the current commit the same way claude-approved/claude-changes-
// requested are (see the eventAction === "synchronize" handling above): a
// sign-off is only honored if it happened after the current head commit was
// pushed, so a new commit re-opens the question instead of carrying forward
// an approval of different code. Approximates "when was this commit pushed"
// with the commit's own authored/committed date, which can be inaccurate
// for a rebased/cherry-picked commit - a reasonable trade-off given GitHub
// exposes no direct "push timestamp" for an arbitrary SHA.
async function humanSignOffStillStands(github, owner, repo, prNumber, headSha) {
  const events = await github.paginate(github.rest.issues.listEvents, {
    owner,
    repo,
    issue_number: prNumber,
    per_page: 100,
  });
  const labelEvents = events.filter(
    (e) => (e.event === "labeled" || e.event === "unlabeled") && e.label && e.label.name === HUMAN_LABEL.name,
  );
  if (labelEvents.length === 0) return false;

  const latest = labelEvents[labelEvents.length - 1];
  if (latest.event !== "unlabeled") return false; // most recent action re-added it

  const { data: commit } = await github.rest.repos.getCommit({ owner, repo, ref: headSha });
  const commitDate = new Date(commit.commit.committer?.date || commit.commit.author.date);
  return new Date(latest.created_at) > commitDate;
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

module.exports = async ({ github, context, core, prNumber, eventAction, selfCheckNames = [] }) => {
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
    // claude-approved / claude-changes-requested are set against a specific
    // commit; a new push makes them stale. precheck failed needs no special
    // handling here - it's a derived STATUS_LABELS member, so the generic
    // toAdd/toRemove logic below already drops it the moment neither
    // coverage failed nor duplicate code is true. coverage failed / duplicate
    // code themselves are deliberately NOT included here - see the comment
    // on DUPLICATE_CODE_LABEL/COVERAGE_LABEL above for why they own their
    // own clearing instead.
    const staleVerdictLabels = Object.values(REVIEW_VERDICT_LABELS)
      .map((l) => l.name)
      .filter((name) => existingLabels.includes(name));
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
  const hasCoverageFailed = existingLabels.includes(COVERAGE_LABEL.name);
  const hasDuplicateCode = existingLabels.includes(DUPLICATE_CODE_LABEL.name);
  const hasClaudeReviewFailed = existingLabels.includes(CLAUDE_REVIEW_FAILED_LABEL.name);

  // Hard guarantee: claude-approved must never survive while a pre-flight
  // stage is failing, no matter how it got set. `ready to merge` no longer
  // depends on any review verdict (see the status precedence chain below),
  // so this is purely about not leaving a misleading label on the PR - a
  // human glancing at labels shouldn't see "approved" next to a currently
  // failing precheck. pre-review-checks.js already waits for both stages'
  // check runs to conclude before ever invoking Claude, so this isn't the
  // primary defense anymore - it's for a new push landing on the PR while
  // Claude's review of the previous commit is still in progress, which
  // starts fresh coverage-diff/duplicate-code runs that could fail before
  // Claude finishes and approves. Strip it here, unconditionally (not gated
  // on eventAction), so the very next sync - including the one each of those
  // two workflows triggers itself right after setting its failure label -
  // corrects it immediately.
  if ((hasCoverageFailed || hasDuplicateCode) && existingLabels.includes(REVIEW_VERDICT_LABELS.APPROVED.name)) {
    await github.rest.issues
      .removeLabel({ owner, repo, issue_number: prNumber, name: REVIEW_VERDICT_LABELS.APPROVED.name })
      .catch((err) => {
        if (err.status !== 404) throw err;
      });
    existingLabels = existingLabels.filter((name) => name !== REVIEW_VERDICT_LABELS.APPROVED.name);
    core.info(`PR #${prNumber}: stripped claude-approved - a pre-flight stage is currently failing.`);
  }

  const [ciConclusion, mergeableState, nativeReviewDecision, autoNeedsHuman, signOffStillStands] = await Promise.all([
    resolveCiConclusion(github, owner, repo, pr.head.sha),
    resolveMergeableState(github, owner, repo, prNumber, pr.mergeable_state),
    resolveReviewDecision(github, owner, repo, prNumber),
    hasHumanLabel ? Promise.resolve(false) : needsHumanSignOff(github, owner, repo, prNumber, pr),
    hasHumanLabel ? Promise.resolve(false) : humanSignOffStillStands(github, owner, repo, prNumber, pr.head.sha),
  ]);
  const reviewDecision = resolveEffectiveReviewDecision(nativeReviewDecision, existingLabels);
  // A human's own removal of the label overrides re-derivation from the
  // same unchanged file patterns - see humanSignOffStillStands() above.
  const needsHuman = hasHumanLabel || (autoNeedsHuman && !signOffStillStands);

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
  } else if (hasCoverageFailed || hasDuplicateCode) {
    // Two independent stages can each fail on their own: coverage-diff-check.yml
    // sets `coverage failed`, duplicate-code-check.yml sets `duplicate code`.
    // Either one blocks merge, and both stay visible on the PR even though
    // this status label is the single umbrella shown here - that's why the
    // summary spells out which one(s) failed.
    status = STATUS_LABELS.PRECHECK_FAILED;
    const causes = [];
    if (hasCoverageFailed) causes.push("coverage-diff");
    if (hasDuplicateCode) causes.push("duplicate-code");
    summary = `A deterministic pre-review check failed (${causes.join(" and ")}) — see the automated pre-flight comment(s) for specifics.`;
  } else if (
    reviewDecision === "CHANGES_REQUESTED" &&
    // Only the native (other-account) path needs this check - the
    // claude-changes-requested fallback label already goes stale on its own
    // via the eventAction === "synchronize" handling above.
    (nativeReviewDecision !== "CHANGES_REQUESTED" ||
      (await changesRequestedIsCurrent(github, owner, repo, prNumber, pr.head.sha)))
  ) {
    status = STATUS_LABELS.CHANGES_REQUESTED;
    summary = "A reviewer requested changes — address them and re-request review.";
  } else if (reviewDecision === "CHANGES_REQUESTED") {
    // A real CHANGES_REQUESTED verdict exists, but every such review is
    // against an older commit - none of them have seen the code as it
    // stands now, so this shouldn't block forever waiting on a re-review
    // that may never come. Treat it as needing a fresh look instead.
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary =
      "A reviewer requested changes on an earlier commit, but new commits have landed since - re-review needed.";
  } else if (needsHuman) {
    // Even a green PR is capped at "needs review" until a human clears the
    // sign-off gate by removing the label themselves.
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary =
      "This PR requires a human sign-off (`needs human review`). Only a human removing that label counts as sign-off.";
  } else if (hasClaudeReviewFailed) {
    // Distinct from "no review yet" (which ready-to-merge tolerates by
    // design): a review was attempted and errored out without producing a
    // verdict, so treat it like any other outstanding blocker rather than
    // silently falling through to ready-to-merge.
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary =
      "The last automated Claude review attempt failed to run (see the PR comment) — retry with `/claude-review` or get a manual review before this can be marked ready to merge.";
  } else if (ciConclusion === "success") {
    // An approving review is not required: waiting on approval when CI
    // hasn't even confirmed the commit builds/passes is a contradiction
    // (nobody should approve a red build), and the deterministic gates above
    // (CI, merge conflicts, coverage/duplication, an active changes-requested
    // verdict, sensitive-path sign-off) already cover the cases that matter.
    // See skills/review/SKILL.md.
    //
    // But CI green alone doesn't mean *every* stage has actually run yet -
    // hasCoverageFailed/hasDuplicateCode/hasClaudeReviewFailed above are only
    // ever set on *failure*; their absence is ambiguous between "passed" and
    // "hasn't finished yet", and coverage-diff-check.yml/duplicate-code-
    // check.yml/claude-review.yml all fire off the same CI-completion event
    // with no ordering guarantee between them (see skills/review/SKILL.md).
    // A single-shot (non-polling: timeoutMs 0) look at every other check run
    // on this commit closes that gap - if anything relevant is still
    // in-flight, fall back to NEEDS_REVIEW instead of asserting ready-to-
    // merge prematurely. selfCheckNames lets the calling workflow exclude
    // its own currently-running job (always still "in progress" at the
    // moment it's the one calling this) so it isn't mistaken for a stalled
    // check - see the call sites in claude-review.yml, duplicate-code-
    // check.yml, coverage-diff-check.yml, and pr-status-labels.yml.
    const completeness = await waitForAllChecks(github, core, {
      owner,
      repo,
      ref: pr.head.sha,
      excludeNames: [...selfCheckNames, GATE_CHECK_NAME],
      timeoutMs: 0,
    });
    if (completeness.completed && completeness.ok) {
      status = STATUS_LABELS.READY_TO_MERGE;
      summary = "CI is green — ready to merge.";
    } else if (completeness.completed) {
      status = STATUS_LABELS.NEEDS_REVIEW;
      summary = `CI is green, but another check is failing: ${completeness.failed.join(", ")}.`;
    } else {
      status = STATUS_LABELS.NEEDS_REVIEW;
      summary = `CI is green, but still waiting on: ${completeness.pending.join(", ")}.`;
    }
  } else {
    status = STATUS_LABELS.NEEDS_REVIEW;
    summary = "Awaiting CI completion.";
  }

  core.info(
    `PR #${prNumber}: ci=${ciConclusion} mergeable=${mergeableState} coverageFailed=${hasCoverageFailed} duplicateCode=${hasDuplicateCode} claudeReviewFailed=${hasClaudeReviewFailed} review=${reviewDecision} (native=${nativeReviewDecision}) needsHuman=${needsHuman} -> ${status.name}`,
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
// (b) reuse the PRECHECK_FAILED (derived umbrella) label without duplicating
// it and risking drift. DUPLICATE_CODE_LABEL / COVERAGE_LABEL are exported
// so analyze-duplication.js and analyze-coverage-diff.js can each own their
// own label's full add/remove lifecycle.
module.exports.STATUS_LABELS = STATUS_LABELS;
module.exports.REVIEW_VERDICT_LABELS = REVIEW_VERDICT_LABELS;
module.exports.DUPLICATE_CODE_LABEL = DUPLICATE_CODE_LABEL;
module.exports.COVERAGE_LABEL = COVERAGE_LABEL;
module.exports.CLAUDE_REVIEW_FAILED_LABEL = CLAUDE_REVIEW_FAILED_LABEL;
module.exports.ensureLabelExists = ensureLabelExists;
// Exported so pre-review-checks.js can exclude this workflow's own derived,
// circular check run (its conclusion depends on the review having already
// happened) from the "wait for every other check on this commit" gate.
module.exports.GATE_CHECK_NAME = GATE_CHECK_NAME;
