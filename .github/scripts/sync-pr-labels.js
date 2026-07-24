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

// The only actor this script trusts a claude-approved label-add event from
// (see claudeApprovedIsGenuine below) - both this workflow's own label
// mutations and claude-review.yml's `gh pr edit --add-label` calls run under
// the workflow's own GITHUB_TOKEN, which GitHub always attributes to this
// exact login. A human (or anything else) adding the label by hand via the
// UI or their own token shows up under their real account instead.
const TRUSTED_AUTOMATION_LOGIN = "github-actions[bot]";

// Job names from coverage-diff-check.yml / duplicate-code-check.yml - see
// labelReflectsCurrentCommit below for why these matter.
const COVERAGE_DIFF_CHECK_NAME = "Check coverage diff";
const DUPLICATE_CODE_CHECK_NAME = "Detect duplicate code";

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
  // Any check run on the commit *other than* CI itself, coverage-diff, or
  // duplicate-code (each of which already has its own, more specific status
  // above/below) that has completed with a failing conclusion - e.g.
  // `no-ai-check.yml`'s "No AI Banners" job, or anything else added to the
  // pipeline later. Deliberately excludes `PR Merge Gate` - see GATE_CHECK_NAME
  // and the `completeness` computation below for why that one's exclusion is
  // never optional (it's this exact sync's own derived, circular output).
  CHECK_FAILED: {
    name: "check failed",
    color: "d73a4a",
    description:
      "A check other than CI, coverage-diff, or duplicate-code failed on this commit — cannot be merged.",
  },
  PRECHECK_FAILED: {
    name: "precheck failed",
    color: "d93f0b",
    description:
      "A deterministic pre-review stage failed — purely derived from the `coverage failed` / `duplicate code` labels below, never set directly.",
  },
  // Distinct from `needs review`: this is genuinely "nothing to review yet",
  // not "reviewed and something's still outstanding" - see the
  // `ciConclusion === null` branch below for exactly what's deferred behind
  // this (a stale changes-requested verdict, a pending human-sign-off
  // reminder, a failed automated-review attempt, or simply nothing at all)
  // until CI has actually concluded once on this commit.
  PENDING: {
    name: "pending",
    color: "ededed",
    description: "CI (or a precheck stage) hasn't concluded on this commit yet — nothing to review yet.",
  },
  NEEDS_REVIEW: {
    name: "needs review",
    color: "fbca04",
    description:
      "CI (and any precheck stage) is green — no other blocking verdict yet, but not ready to merge either.",
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

// coverage-diff-check.yml and duplicate-code-check.yml each own their own
// label's full add/remove lifecycle (see the comment on
// DUPLICATE_CODE_LABEL/COVERAGE_LABEL above) - neither clears it on a new
// push, only their own next run does. coverage-diff-check.yml's next run
// doesn't even start until the *new* commit's CI finishes (it triggers on
// workflow_run: CI completed), which can be 20+ minutes away. A push landing
// on the PR runs this script almost immediately, long before that - so
// trusting the label at face value right then would treat "hasn't been
// assessed for this commit yet" the same as "failed this commit", exactly
// the misleading red X this gate exists to avoid. Only treat the label as
// blocking once a check run with this name has actually completed for the
// current head commit (any conclusion - completion is what matters here,
// not pass/fail); otherwise it's stale, carried over from a prior commit,
// and the ordinary CI/other-checks pending logic below already handles
// "still waiting on this check" correctly once it registers.
async function labelReflectsCurrentCommit(github, owner, repo, headSha, checkName) {
  const runs = await github.paginate(github.rest.checks.listForRef, {
    owner,
    repo,
    ref: headSha,
    check_name: checkName,
    per_page: 100,
  });
  return runs.some((run) => run.status === "completed");
}

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

// GitHub's own reviewDecision is intrinsically provenance-safe: it only ever
// reflects a real, distinct account's formal review, and GitHub itself
// refuses to record a self-approval (422). The claude-approved label has no
// such guarantee - it's an ordinary label, and anyone with triage+ access
// can add any label to any PR via the UI or their own token, which would
// otherwise let them forge a clean verdict and trigger the auto-merge below
// with no review ever having happened. Trust it for merging only when the
// most recent event that added it was actually authored by this repo's own
// automation (TRUSTED_AUTOMATION_LOGIN), never by whichever account happens
// to currently hold triage/write access.
async function claudeApprovedIsGenuine(github, owner, repo, prNumber) {
  const events = await github.paginate(github.rest.issues.listEvents, {
    owner,
    repo,
    issue_number: prNumber,
    per_page: 100,
  });
  const labelEvents = events.filter(
    (e) => e.event === "labeled" && e.label && e.label.name === REVIEW_VERDICT_LABELS.APPROVED.name,
  );
  if (labelEvents.length === 0) return false;
  const latest = labelEvents[labelEvents.length - 1];
  return Boolean(latest.actor) && latest.actor.login === TRUSTED_AUTOMATION_LOGIN;
}

// Squash-merges `pr` and deletes its branch (same-repo PRs only - a fork's
// branch can't be deleted by this token, mirroring `gh pr merge
// --delete-branch`'s own behavior). Called only once every deterministic
// gate this script already computes - CI green, no merge conflict, no
// coverage/duplicate-code failure, no active changes-requested verdict, no
// pending human sign-off (all folded into `status === READY_TO_MERGE`) -
// plus an actual, provenance-checked approval agrees. Tolerates the PR
// already being merged/closed or genuinely not mergeable right now (a
// concurrent push, a race with another trigger) as a no-op rather than
// failing the whole label-sync job over it - the next sync will simply
// re-evaluate from scratch.
async function autoMergeIfApproved(github, core, owner, repo, prNumber, pr) {
  try {
    await github.rest.pulls.merge({ owner, repo, pull_number: prNumber, merge_method: "squash" });
    core.info(`PR #${prNumber}: auto-merged (squash) - ready to merge with a genuine approval.`);
  } catch (err) {
    if (err.status === 405 || err.status === 409) {
      core.info(`PR #${prNumber}: auto-merge attempt skipped (${err.status}): ${err.message}`);
      return;
    }
    throw err;
  }

  if (pr.head.repo && pr.base.repo && pr.head.repo.id === pr.base.repo.id) {
    await github.rest.git
      .deleteRef({ owner, repo, ref: `heads/${pr.head.ref}` })
      .catch((err) => {
        if (err.status !== 422 && err.status !== 404) throw err;
      });
  }
}

// `pending` is true only for states where nothing has actually failed and
// we're purely still waiting on other work to finish (CI hasn't concluded
// yet, or every other check hasn't reached a conclusion). Publishing those
// as a completed "failure" check is misleading - it reads as "this PR is
// broken" when the honest state is "not done yet" - and it's exactly what a
// required check being red during a perfectly normal CI run looks like to a
// human glancing at the PR. `in_progress` (no conclusion) still blocks a
// required-status-check merge exactly the same as `failure` would, so this
// changes nothing about mergeability, only the misleading red X.
async function createGateCheck(github, owner, repo, headSha, status, summary, pending) {
  if (pending) {
    await github.rest.checks.create({
      owner,
      repo,
      name: GATE_CHECK_NAME,
      head_sha: headSha,
      status: "in_progress",
      output: {
        title: status.name,
        summary,
      },
    });
    return;
  }

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

module.exports = async ({
  github,
  context,
  core,
  prNumber,
  eventAction,
  selfCheckNames = [],
  // Off by default - flip the AUTO_MERGE_ENABLED repo/environment variable
  // to `true` once the pipeline has earned enough trust to merge PRs with
  // no human clicking the button. Every gate below (ready to merge, a
  // genuine approval, the label-provenance check) still runs and gets
  // logged either way, so turning this on later is a config change, not a
  // code change - see the "Auto-merge" section in skills/review/SKILL.md.
  autoMergeEnabled = false,
}) => {
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

  const [
    ciConclusion,
    mergeableState,
    nativeReviewDecision,
    autoNeedsHuman,
    signOffStillStands,
    coverageLabelCurrent,
    duplicateLabelCurrent,
  ] = await Promise.all([
    resolveCiConclusion(github, owner, repo, pr.head.sha),
    resolveMergeableState(github, owner, repo, prNumber, pr.mergeable_state),
    resolveReviewDecision(github, owner, repo, prNumber),
    hasHumanLabel ? Promise.resolve(false) : needsHumanSignOff(github, owner, repo, prNumber, pr),
    hasHumanLabel ? Promise.resolve(false) : humanSignOffStillStands(github, owner, repo, prNumber, pr.head.sha),
    hasCoverageFailed
      ? labelReflectsCurrentCommit(github, owner, repo, pr.head.sha, COVERAGE_DIFF_CHECK_NAME)
      : Promise.resolve(false),
    hasDuplicateCode
      ? labelReflectsCurrentCommit(github, owner, repo, pr.head.sha, DUPLICATE_CODE_CHECK_NAME)
      : Promise.resolve(false),
  ]);
  const reviewDecision = resolveEffectiveReviewDecision(nativeReviewDecision, existingLabels);
  // A human's own removal of the label overrides re-derivation from the
  // same unchanged file patterns - see humanSignOffStillStands() above.
  const needsHuman = hasHumanLabel || (autoNeedsHuman && !signOffStillStands);
  // Only a label backed by a completed check run on *this* commit counts as
  // an actual failure of this commit - see labelReflectsCurrentCommit above.
  const coverageFailedForThisCommit = hasCoverageFailed && coverageLabelCurrent;
  const duplicateCodeForThisCommit = hasDuplicateCode && duplicateLabelCurrent;

  const ciFailed = ciConclusion !== null && !["success", "skipped", "neutral"].includes(ciConclusion);
  const mergeConflict = mergeableState === "dirty";

  // Single-shot (timeoutMs: 0 - never polls/waits) look at every check run
  // on this commit, computed unconditionally and up front so a stage other
  // than CI/coverage-diff/duplicate-code that's already completed with a
  // failing conclusion (e.g. no-ai-check.yml's "No AI Banners" job) is never
  // missed regardless of which branch below would otherwise fire - `needs
  // review` (or any status beyond a known-bad one) must never be assigned
  // while a real stage failure like this is sitting unaddressed. Excludes
  // only this exact sync's own derived, circular `PR Merge Gate` check (see
  // GATE_CHECK_NAME) and the calling workflow's own still-running job(s)
  // (selfCheckNames) - every other check run, including CI's own per-job
  // checks, coverage-diff, and duplicate-code, is fair game here, but those
  // three already have their own more specific status above/below that takes
  // priority whenever it applies.
  const completeness = await waitForAllChecks(github, core, {
    owner,
    repo,
    ref: pr.head.sha,
    excludeNames: [...selfCheckNames, GATE_CHECK_NAME],
    timeoutMs: 0,
  });

  let status;
  let summary;
  // See the comment on createGateCheck for what this controls.
  let pending = false;
  if (ciFailed) {
    status = STATUS_LABELS.CI_FAILING;
    summary = `CI is failing on the latest commit (conclusion: ${ciConclusion}) — cannot be merged until it's green.`;
  } else if (mergeConflict) {
    status = STATUS_LABELS.MERGE_CONFLICT;
    summary = "This PR has real conflicts with the base branch — rebase and resolve them before it can be merged.";
  } else if (coverageFailedForThisCommit || duplicateCodeForThisCommit) {
    // Two independent stages can each fail on their own: coverage-diff-check.yml
    // sets `coverage failed`, duplicate-code-check.yml sets `duplicate code`.
    // Either one blocks merge, and both stay visible on the PR even though
    // this status label is the single umbrella shown here - that's why the
    // summary spells out which one(s) failed. Gated on *ForThisCommit, not
    // the raw has*/label read, so a label stale from a prior commit (that
    // commit's own check hasn't re-run yet) can't masquerade as a failure of
    // the current one - see labelReflectsCurrentCommit.
    status = STATUS_LABELS.PRECHECK_FAILED;
    const causes = [];
    if (coverageFailedForThisCommit) causes.push("coverage-diff");
    if (duplicateCodeForThisCommit) causes.push("duplicate-code");
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
  } else if (completeness.completed && !completeness.ok) {
    // Some check other than CI/coverage-diff/duplicate-code (each already
    // handled above) has already completed with a failing conclusion -
    // settled, known-bad, exactly like `ci failing` or `merge conflict`
    // above. Checked *before* the `ciConclusion === null` branch below,
    // deliberately: this can already be true even while CI itself hasn't
    // concluded yet (e.g. `no-ai-check.yml` runs independently of CI and
    // finishes fast) - no amount of waiting on CI or anything else can
    // un-fail an already-completed, already-failed check, so there's nothing
    // to gain from reporting "still pending" over a real, known failure.
    status = STATUS_LABELS.CHECK_FAILED;
    summary = `A check other than CI is failing: ${completeness.failed.join(", ")}.`;
  } else if (ciConclusion === null) {
    // CI hasn't concluded even once on this commit yet - defer every
    // review-related signal below (a stale changes-requested verdict, a
    // human-sign-off reminder, a failed automated-review attempt, or simply
    // nothing outstanding at all) behind a known-good build first, the same
    // "nobody should approve a red build" reasoning `ready to merge` already
    // applies (see skills/review/SKILL.md) - `needs review` inviting review
    // attention before CI has even run once is exactly the misleading,
    // always-true default this status exists to avoid. `needs human review`
    // itself is unaffected: that's a separate, additive label applied below
    // regardless of this branch, so the sign-off reminder is never hidden by
    // this - only the *main* status is deferred. Positioned after the
    // CHANGES_REQUESTED (current) branch above, deliberately: a real,
    // current review verdict is meaningful information on its own and
    // shouldn't be hidden behind "still waiting on CI" the way the more
    // advisory signals below are.
    status = STATUS_LABELS.PENDING;
    summary = "Awaiting CI completion.";
    pending = true;
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
    // (CI, merge conflicts, coverage/duplication, any other failed check, an
    // active changes-requested verdict, sensitive-path sign-off) already
    // cover the cases that matter. See skills/review/SKILL.md.
    //
    // CI (and every other check) already known to be green/passing at this
    // point (the `completeness` computed above already ruled out any failing
    // check) doesn't mean *every* stage has actually run yet, though -
    // coverage-diff-check.yml/duplicate-code-check.yml fire off the same
    // CI-completion event `pr-status-labels.yml` itself reacts to, with no
    // ordering guarantee between them, so one or both simply not having
    // registered a check run yet is indistinguishable from "nothing to wait
    // for" in `completeness.completed` alone - a check that hasn't been
    // scheduled yet by GitHub Actions doesn't exist for `waitForAllChecks` to
    // see at all. Confirmed live on PR #373: a stale `coverage failed` label
    // sat alongside an already-passing "Coverage Diff Check" run for hours
    // with no fresh sync ever reconciling the two. Requiring both named
    // checks to have actually completed for this exact head sha (any
    // conclusion - completeness.ok already covers their pass/fail once they
    // exist) closes that gap without needing to know every check's name up
    // front, the same way labelReflectsCurrentCommit already guards the
    // opposite direction (a failing label stale from a prior commit).
    const coverageChecked = await labelReflectsCurrentCommit(
      github,
      owner,
      repo,
      pr.head.sha,
      COVERAGE_DIFF_CHECK_NAME,
    );
    const duplicateChecked = await labelReflectsCurrentCommit(
      github,
      owner,
      repo,
      pr.head.sha,
      DUPLICATE_CODE_CHECK_NAME,
    );
    if (completeness.completed && coverageChecked && duplicateChecked) {
      // completeness.ok is already implied true here: the
      // `completeness.completed && !completeness.ok` branch above would
      // have caught a failing check before ever reaching this point.
      status = STATUS_LABELS.READY_TO_MERGE;
      summary = "CI is green — ready to merge.";
    } else {
      // Not "nothing to review yet" in quite the same sense as the
      // ciConclusion === null branch above (CI itself is done), but every
      // precheck stage isn't necessarily settled yet - same "don't invite
      // review attention on an incomplete picture" reasoning applies.
      status = STATUS_LABELS.PENDING;
      const stillWaiting = new Set(completeness.completed ? [] : completeness.pending);
      if (!coverageChecked) stillWaiting.add(COVERAGE_DIFF_CHECK_NAME);
      if (!duplicateChecked) stillWaiting.add(DUPLICATE_CODE_CHECK_NAME);
      summary = `CI is green, but still waiting on: ${[...stillWaiting].join(", ")}.`;
      pending = true;
    }
  } else {
    // Residual case: ciConclusion is a concluded-but-neither-success-nor-
    // failure value (e.g. "skipped"/"neutral" - excluded from `ciFailed`
    // above). Nothing upstream distinguishes this from "still running" in
    // practice, so it gets the same treatment.
    status = STATUS_LABELS.PENDING;
    summary = "Awaiting CI completion.";
    pending = true;
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

  await createGateCheck(github, owner, repo, pr.head.sha, status, summary, pending);

  // Auto-merge: every deterministic gate this function computes (CI green,
  // no merge conflict, no coverage/duplicate-code failure, no active
  // changes-requested verdict, no pending human sign-off) is already folded
  // into `status === READY_TO_MERGE` - the one thing it deliberately does
  // NOT require is an actual approval (see skills/review/SKILL.md: "An
  // approving review is not required" for the label itself, since nobody
  // should approve a red build). Squash-merging on top of that still needs
  // a real approval to have happened, so check that separately here rather
  // than loosening READY_TO_MERGE's own meaning.
  if (status.name === STATUS_LABELS.READY_TO_MERGE.name) {
    const isNativeApproval = nativeReviewDecision === "APPROVED";
    // The label path only ever kicks in when there's no real native
    // decision to trust yet - see resolveEffectiveReviewDecision.
    const isLabelApproval = !isNativeApproval && existingLabels.includes(REVIEW_VERDICT_LABELS.APPROVED.name);
    const approved =
      isNativeApproval || (isLabelApproval && (await claudeApprovedIsGenuine(github, owner, repo, prNumber)));
    if (isLabelApproval && !approved) {
      core.info(
        `PR #${prNumber}: claude-approved is present but wasn't applied by ${TRUSTED_AUTOMATION_LOGIN} - not auto-merging.`,
      );
    }
    if (approved && !autoMergeEnabled) {
      core.info(
        `PR #${prNumber}: ready to merge with a genuine approval, but AUTO_MERGE_ENABLED is off - leaving it for a human to merge.`,
      );
    } else if (approved) {
      await autoMergeIfApproved(github, core, owner, repo, prNumber, pr);
    }
  }
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
