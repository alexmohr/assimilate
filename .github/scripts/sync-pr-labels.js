// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Derives a PR's status label from objective signals (CI conclusion + GitHub's
// native review decision) instead of relying on an agent or human remembering
// to apply it by hand, and publishes the same verdict as a "PR Merge Gate"
// check run so it can be made a required status check - merging a PR that
// isn't `ready to merge` then requires an explicit branch-protection bypass,
// not just human attentiveness. See skills/review/SKILL.md.

const { waitForAllChecks, latestRunPerName } = require("./lib/wait-for-check");

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

// The check-run names labelReflectsCurrentCommit below looks for - not
// necessarily each workflow's job name. DUPLICATE_CODE_CHECK_NAME matches
// duplicate-code-check.yml's job name ("Detect duplicate code"), which
// GitHub reliably auto-registers as its own check run. COVERAGE_DIFF_CHECK_NAME
// does NOT use coverage-diff-check.yml's job name ("Check coverage diff") -
// that job-level check was confirmed live on PR #399 to never actually
// register on the PR's commit (checked repeatedly over several days), so
// labelReflectsCurrentCommit(..., "Check coverage diff") always returned
// false and permanently stuck every PR's "PR Merge Gate" at "still waiting
// on: Check coverage diff", no matter how many times it was re-triggered.
// "Coverage Diff Check" is the name analyze-coverage-diff.js itself posts
// via its own checks.create call (CHECK_NAME there) and reliably appears -
// use that instead.
const COVERAGE_DIFF_CHECK_NAME = "Coverage Diff Check";
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
  // this (a stale changes-requested verdict, a failed automated-review
  // attempt, or simply nothing at all) until CI has actually concluded once
  // on this commit.
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
// Checks the MOST RECENT run of this name for this sha, not "any" run -
// re-running CI on an unchanged sha (rather than pushing a new commit) can
// leave an older, already-completed, since-superseded run of the same name
// sitting alongside a fresh one still in flight for several seconds to a
// minute. "any completed run exists" reads the stale one as current and
// reports its (possibly failing) verdict as settled, even while the fresh
// rerun that will supersede it hasn't posted yet - PR #383 hit exactly this:
// a CI rerun after fixing a flaky coverage regression left the original
// failing "Coverage Diff Check" run queryable here well after the rerun
// started, long enough for a workflow_run-triggered sync to read it as
// current and publish "PR Merge Gate" as a hard failure a second before the
// rerun's own passing verdict landed. Collapsing to the latest attempt closes
// that window: a newer run that hasn't completed yet correctly reports false
// here (falling through to the ordinary pending handling below) instead of
// resurrecting a superseded verdict. Reuses lib/wait-for-check.js's
// latestRunPerName rather than re-deriving "latest" here, so both callers
// agree on what "latest" means - and specifically so this orders by the
// monotonic `id` instead of `started_at`, which a queued-but-not-yet-started
// rerun hasn't populated yet (that would let the stale completed run win and
// reopen a narrower version of this same bug). Filtering the query by
// check_name means the helper collapses to at most one entry.
async function labelReflectsCurrentCommit(github, owner, repo, headSha, checkName) {
  const runs = await github.paginate(github.rest.checks.listForRef, {
    owner,
    repo,
    ref: headSha,
    check_name: checkName,
    per_page: 100,
  });
  const [latest] = latestRunPerName(runs);
  return latest !== undefined && latest.status === "completed";
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

// The two sets parseAutoMergeEnabled below recognises. Both are needed: the
// kill switch is documented as `false`, but the variable's previous meaning
// was "set me to `true` to turn auto-merge on", so a repo that already set
// `true` under the old polarity must keep meaning "on" rather than silently
// flipping to off the moment this lands.
const AUTO_MERGE_ON_VALUES = new Set(["true", "1", "yes", "on", "enabled"]);
const AUTO_MERGE_OFF_VALUES = new Set(["false", "0", "no", "off", "disabled"]);

// Parses the AUTO_MERGE_ENABLED repo/environment variable into the boolean
// `autoMergeEnabled` below - the one place any of this is decided, so the
// two workflow call sites can't drift apart on it.
//
// Unset is the normal case and means on: a workflow expression renders a
// variable that was never set as the empty string, and auto-merge is the
// documented default (see skills/review/SKILL.md). Only a genuinely absent
// or empty raw value counts as unset - a variable someone actually filled in
// with blanks is treated as set-but-unrecognised below, since that is a
// mistake rather than a request for the default.
//
// Anything set but unrecognised fails *closed*, with a warning. A bare
// `!== "false"` would instead fail open: an operator reaching for the
// documented kill switch and typing `False`, `FALSE`, ` false` or `no`
// would leave auto-merge running with nothing said anywhere - the one
// direction where guessing wrong merges code unattended. Trimming and
// lower-casing means the near-misses above are simply understood; the
// warning is for whatever is left.
function parseAutoMergeEnabled(rawValue, core) {
  const raw = String(rawValue ?? "");
  if (raw === "") return true;

  const value = raw.trim().toLowerCase();
  if (AUTO_MERGE_ON_VALUES.has(value)) return true;
  if (AUTO_MERGE_OFF_VALUES.has(value)) return false;
  // `raw`, not the trimmed value: a whitespace-only variable would otherwise
  // report itself as `""`, which reads exactly like the unset case it is
  // deliberately not being treated as.
  core.warning(
    `AUTO_MERGE_ENABLED is set to an unrecognised value (${JSON.stringify(raw)}) - treating ` +
      `it as off. Set it to "false" to disable auto-merge, or unset it to use the default ` +
      `(enabled).`,
  );
  return false;
}

// Which of the two ways a caller can express "auto-merge is allowed" wins.
// The raw variable does when the caller passed one - that's the workflows,
// which hand over AUTO_MERGE_ENABLED's text and let this module parse it. The
// boolean stands otherwise, for pre-review-checks.js, which pins it off.
// `autoMergeEnabled` defaults here too, not just in syncLabels' signature: a
// caller that passes neither input must not end up with `undefined` standing
// in for a decision about whether to merge code unattended.
function resolveAutoMerge({ autoMergeEnabled = false, autoMergeEnabledRaw, core }) {
  if (autoMergeEnabledRaw === undefined) return autoMergeEnabled;
  return parseAutoMergeEnabled(autoMergeEnabledRaw, core);
}

// Paths whose contents auto-merge will never land on its own.
//
// These are the rails: everything that defines or suppresses a gate, as
// opposed to the code the gates run over. CI reads every one of them from the
// PR's *own head*, so a PR editing one takes effect on the very run that
// decides whether that PR may merge - it goes green because it loosened the
// thing that would have failed it, then merges unattended on the automation's
// own approval. A one-line epsilon in analyze-coverage-diff.js, an advisory id
// in deny.toml, `unwrap_used = "allow"` in the root Cargo.toml, a `"lint"`
// script rewritten to `true` in frontend/package.json: all the same move.
//
// The three rules below are structural rather than a list of known files, on
// purpose. An enumerated list has to be extended every time a config file is
// added, and a rail nobody remembered to enumerate is a rail that silently
// becomes auto-mergeable - which is exactly how `deny.toml`, `.jscpd.json` and
// `frontend/eslint.config.js` were missed when this guard covered only
// `.github/`. Being a little over-broad costs a person one click; being
// under-broad costs the gate.

// 1. Whole directory trees, matched by prefix.
//
// - `.github/` - the workflows, the coverage-diff and duplicate-code
//   analyzers, and this script, which decides what "ready to merge" even means.
// - `lints/` - the dylint library behind AGENTS.md's no-string-control-flow
//   rule; ci.yml builds it from the PR's own checkout and runs it with
//   `-D no_string_control_flow`.
// - `frontend/eslint-rules/` - the frontend's mirror of that same rule,
//   `no-string-literal-control-flow`, wired up in frontend/eslint.config.js.
// - `scripts/` - the entry points for the repo's local pre-commit hooks
//   (check-no-raw-sqlx-queries.sh, no-typography-in-comments.py).
const AUTO_MERGE_PROTECTED_PREFIXES = [
  ".github/",
  "lints/",
  "frontend/eslint-rules/",
  "scripts/",
];

// 2. Directories whose *immediate* children are protected, but whose
// subdirectories are not. These are the two places this repo keeps gate
// configuration, and treating the whole level as protected is what stops the
// next config file added there from being an unguarded rail:
//
// - `""` (the repository root) - Cargo.toml, deny.toml, .jscpd.json,
//   .pre-commit-config.yaml, clippy.toml, .rustfmt.toml, ruff.toml,
//   .markdownlint.yaml, .yamlfmt, REUSE.toml, mkdocs.yml. `crates/`, `docs/`
//   and `skills/` are subdirectories and stay auto-mergeable.
// - `frontend/` - package.json (whose `lint`, `build`, `test` and
//   `format:check` scripts are what `ci.yml` actually invokes), package-lock,
//   eslint.config.js, the tsconfigs vue-tsc reads, playwright.config.ts,
//   vite.config.ts, .prettierrc, .npm-audit-allowlist.json. `frontend/src/`
//   and `frontend/e2e/` are subdirectories and stay auto-mergeable.
const AUTO_MERGE_PROTECTED_CONFIG_DIRS = ["", "frontend/"];

// 3. Filenames protected wherever in the tree they appear.
//
// A member crate's Cargo.toml carries `[lints] workspace = true`, which is the
// only thing applying the root's `[workspace.lints.clippy]` deny list to that
// crate. Deleting those two lines drops every clippy deny for the crate and CI
// stays green: the `validate-cargo-lints` hook compares the *root* Cargo.toml
// against the shared baseline and never checks that members opt in.
const AUTO_MERGE_PROTECTED_BASENAMES = ["Cargo.toml"];

// GitHub's `pulls.listFiles` returns at most this many files for a PR, even
// paginated. Past it the list is silently truncated rather than an error.
const LISTED_FILES_CAP = 3000;

// Whether `path` is one of the rails above. A non-string (an absent
// `previous_filename`) is not.
function isProtectedPath(path) {
  if (typeof path !== "string" || path === "") return false;
  if (AUTO_MERGE_PROTECTED_PREFIXES.some((prefix) => path.startsWith(prefix))) return true;
  if (AUTO_MERGE_PROTECTED_BASENAMES.includes(path.split("/").pop())) return true;
  // An immediate child of a config directory: inside it, with nothing left to
  // descend through afterwards.
  return AUTO_MERGE_PROTECTED_CONFIG_DIRS.some(
    (dir) => path.startsWith(dir) && !path.slice(dir.length).includes("/"),
  );
}

// The first protected path `files` (as returned by `pulls.listFiles`) touches,
// or null. Both the current and the previous path are checked, so a rename
// *out of* a protected location can't launder a change through.
function protectedPathIn(files) {
  for (const file of files) {
    for (const path of [file.filename, file.previous_filename]) {
      if (isProtectedPath(path)) return path;
    }
  }
  return null;
}

// Whether `files` touches anything auto-merge must not land unattended.
function touchesProtectedPaths(files) {
  return protectedPathIn(files) !== null;
}

// Why auto-merge must leave this PR alone, or null if it may proceed.
//
// A file list at the cap is treated as protected even when nothing in it
// matches: past 3000 files the list is truncated, so a rail edit sitting
// beyond the cut simply isn't in `files` and `protectedPathIn` would answer
// "no" to a question it couldn't actually see. A bulk or generated diff hiding
// a rail edit is precisely the shape this guard exists to stop, so an
// unprovable list counts as protected rather than as clean.
function autoMergeBlockedReason(files) {
  if (files.length >= LISTED_FILES_CAP) {
    return (
      `its file list hit GitHub's ${LISTED_FILES_CAP}-file cap, so it cannot be shown not to ` +
      "change a gate CI reads from this PR's own head"
    );
  }
  const protectedPath = protectedPathIn(files);
  if (protectedPath) return `it changes ${protectedPath}`;
  return null;
}

// Squash-merges `pr` and deletes its branch (same-repo PRs only - a fork's
// branch can't be deleted by this token, mirroring `gh pr merge
// --delete-branch`'s own behavior). Called only once every deterministic
// gate this script already computes - CI green, no merge conflict, no
// coverage/duplicate-code failure, no active changes-requested verdict (all
// folded into `status === READY_TO_MERGE`) - plus an actual,
// provenance-checked approval agrees. Tolerates the PR
// already being merged/closed or genuinely not mergeable right now (a
// concurrent push, a race with another trigger) as a no-op rather than
// failing the whole label-sync job over it - the next sync will simply
// re-evaluate from scratch.
async function autoMergeIfApproved(github, core, owner, repo, prNumber, pr) {
  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner,
    repo,
    pull_number: prNumber,
    per_page: 100,
  });
  const blockedReason = autoMergeBlockedReason(files);
  if (blockedReason) {
    core.info(
      `PR #${prNumber}: ready to merge, but ${blockedReason} - auto-merge deliberately does ` +
        "not land changes to the gates it trusts (CI, the coverage and duplication analyzers, " +
        "the lint and suppression configs, or this script itself). Merge it by hand once a " +
        "person has read the diff.",
    );
    return;
  }

  // `sha` pins the merge to the head the guard above was computed against.
  // Without it GitHub merges whatever the head is *now*, so a commit landing
  // between the file listing and this call would be merged without ever having
  // been checked - the one race that could defeat the protected-path guard by
  // going around it. With it, a moved head is a 409 and the sync simply does
  // nothing; the next one re-evaluates from scratch against the new head.
  try {
    await github.rest.pulls.merge({
      owner,
      repo,
      pull_number: prNumber,
      merge_method: "squash",
      sha: pr.head.sha,
    });
    core.info(`PR #${prNumber}: auto-merged (squash) - ready to merge with a genuine approval.`);
  } catch (err) {
    // 405: not mergeable right now. 409: the head moved since `sha` was read,
    // or a concurrent trigger got there first. Both are no-ops rather than job
    // failures - the next sync re-evaluates every gate from scratch.
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
  // Whether the merge call happens at all. A PR that reaches `ready to merge`
  // has already cleared every deterministic gate this script computes *and*
  // carries a genuine, provenance-checked approval (see hasGenuineApproval
  // below), so there's nothing left for a human to add by clicking the
  // button. Every gate still runs and gets logged whichever way this lands,
  // so this only decides whether the merge happens, never how the verdict is
  // computed.
  //
  // The *parameter* defaults to off while the *feature* is on by default:
  // "on unless AUTO_MERGE_ENABLED says otherwise" is decided by
  // parseAutoMergeEnabled, and all three call sites pass this explicitly
  // (both workflows through that parser, pre-review-checks.js pinned off).
  // So this default is only ever reached by a future caller that forgot to
  // pass it - and a forgotten argument should not be able to merge code
  // unattended. Same reasoning as the kill switch itself: the ambiguous case
  // resolves in the direction that doesn't merge.
  //
  // A boolean. pre-review-checks.js passes it directly (pinned off); the
  // workflows instead pass `autoMergeEnabledRaw` below and let this module
  // do the parsing. See the "Auto-merge" section in skills/review/SKILL.md.
  autoMergeEnabled = false,
  // The raw AUTO_MERGE_ENABLED variable, straight out of the workflow's step
  // env. When present it decides `autoMergeEnabled` via
  // parseAutoMergeEnabled; when absent the boolean above stands.
  //
  // The workflows pass the *string* rather than calling the parser
  // themselves because the two can come from different commits: this script
  // is always checked out from the default branch (`sparse-checkout
  // .github/scripts`, `ref: default_branch`), while on a `pull_request_review`
  // event the workflow file itself comes from the PR's head. A workflow body
  // calling `sync.parseAutoMergeEnabled(...)` therefore crashes with
  // "not a function" on any PR that adds it, until that PR reaches the
  // default branch. Passing data instead of calling across that seam means an
  // older script simply ignores this key and keeps its own default - off,
  // which is the safe direction to be wrong in.
  autoMergeEnabledRaw,
  // Off by default - only pr-status-labels.yml's own call site turns this
  // on. See the "notify claude-review.yml" comment below for why this can't
  // just always be on: claude-review.yml calls this same function on itself
  // (via pre-review-checks.js and its own post-review re-sync), and letting
  // either of those dispatch would let a review job re-trigger a second,
  // concurrent review of the same commit while the first is still running.
  dispatchOnNeedsReview = false,
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

  const hasCoverageFailed = existingLabels.includes(COVERAGE_LABEL.name);
  const hasDuplicateCode = existingLabels.includes(DUPLICATE_CODE_LABEL.name);
  const hasClaudeReviewFailed = existingLabels.includes(CLAUDE_REVIEW_FAILED_LABEL.name);

  // Hard guarantee: claude-approved must never survive while a pre-flight
  // stage is failing, no matter how it got set. Even though `ready to merge`
  // does depend on a genuine review verdict now (see hasGenuineApproval
  // below), a stale approval sitting next to a currently failing precheck is
  // still misleading on its own - a human glancing at labels shouldn't see
  // "approved" next to "coverage failed"/"duplicate code", regardless of
  // whether it's also blocking the merge gate. pre-review-checks.js already
  // waits for both stages'
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
    coverageLabelCurrent,
    duplicateLabelCurrent,
  ] = await Promise.all([
    resolveCiConclusion(github, owner, repo, pr.head.sha),
    resolveMergeableState(github, owner, repo, prNumber, pr.mergeable_state),
    resolveReviewDecision(github, owner, repo, prNumber),
    hasCoverageFailed
      ? labelReflectsCurrentCommit(github, owner, repo, pr.head.sha, COVERAGE_DIFF_CHECK_NAME)
      : Promise.resolve(false),
    hasDuplicateCode
      ? labelReflectsCurrentCommit(github, owner, repo, pr.head.sha, DUPLICATE_CODE_CHECK_NAME)
      : Promise.resolve(false),
  ]);
  const reviewDecision = resolveEffectiveReviewDecision(nativeReviewDecision, existingLabels);
  // Only a label backed by a completed check run on *this* commit counts as
  // an actual failure of this commit - see labelReflectsCurrentCommit above.
  const coverageFailedForThisCommit = hasCoverageFailed && coverageLabelCurrent;
  const duplicateCodeForThisCommit = hasDuplicateCode && duplicateLabelCurrent;

  // The same "genuine approval" gate the auto-merge path already applied
  // (see autoMergeIfApproved's call site below) now also gates the
  // `ready to merge` status itself, not just the merge action - see
  // skills/review/SKILL.md. A native APPROVED review is intrinsically
  // provenance-safe (GitHub guarantees a real, distinct reviewer and
  // rejects self-approval outright). The same-account fallback,
  // `claude-approved`, is an ordinary label anyone with triage+ access could
  // add by hand, so it's only trusted here when the most recent event that
  // added it was actually authored by this repo's own automation - see
  // claudeApprovedIsGenuine.
  const isNativeApproval = nativeReviewDecision === "APPROVED";
  const isLabelApproval = !isNativeApproval && existingLabels.includes(REVIEW_VERDICT_LABELS.APPROVED.name);
  const hasGenuineApproval =
    isNativeApproval || (isLabelApproval && (await claudeApprovedIsGenuine(github, owner, repo, prNumber)));
  if (isLabelApproval && !hasGenuineApproval) {
    core.info(
      `PR #${prNumber}: claude-approved is present but wasn't applied by ${TRUSTED_AUTOMATION_LOGIN} - not treating it as a genuine approval.`,
    );
  }

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
    // failed automated-review attempt, or simply nothing outstanding at all)
    // behind a known-good build first, the same "nobody should approve a red
    // build" reasoning `ready to merge` already applies (see
    // skills/review/SKILL.md) - `needs review` inviting review attention
    // before CI has even run once is exactly the misleading, always-true
    // default this status exists to avoid. Positioned after the
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
      if (hasGenuineApproval) {
        status = STATUS_LABELS.READY_TO_MERGE;
        summary = "CI is green and a genuine approval is on record — ready to merge.";
      } else {
        // Every deterministic gate is clear, but nobody's actually signed
        // off yet - a genuine approval (native review, or `claude-approved`
        // applied by this repo's own automation) is required before this
        // can be `ready to merge`, not just a green build. See
        // skills/review/SKILL.md.
        status = STATUS_LABELS.NEEDS_REVIEW;
        summary =
          "CI is green, but no genuine approval yet — needs a native approving review or a " +
          `\`${REVIEW_VERDICT_LABELS.APPROVED.name}\` label applied by ${TRUSTED_AUTOMATION_LOGIN}.`;
      }
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
    `PR #${prNumber}: ci=${ciConclusion} mergeable=${mergeableState} coverageFailed=${hasCoverageFailed} duplicateCode=${hasDuplicateCode} claudeReviewFailed=${hasClaudeReviewFailed} review=${reviewDecision} (native=${nativeReviewDecision}) -> ${status.name}`,
  );

  const desired = [status.name];

  const toAdd = desired.filter((name) => !existingLabels.includes(name));
  const statusNames = Object.values(STATUS_LABELS).map((l) => l.name);
  const toRemove = statusNames.filter(
    (name) => name !== status.name && existingLabels.includes(name),
  );

  for (const name of toAdd) {
    await ensureLabelExists(github, owner, repo, status);
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

  // Notify claude-review.yml directly rather than relying on it to notice
  // the label change itself. GitHub deliberately does not fire a new
  // workflow run for an event (here, `labeled`) produced by the calling
  // workflow's own GITHUB_TOKEN - this is what stops accidental recursive
  // workflow runs, but it also means claude-review.yml's `pull_request_target:
  // labeled` trigger can never actually fire for this addLabels call above,
  // no matter how correctly it's configured. In practice this showed up as
  // "the review job only starts once a human manually removes and re-adds
  // the label" - a human's token isn't GITHUB_TOKEN, so that label event
  // fires normally. `repository_dispatch` is explicitly exempted from the
  // GITHUB_TOKEN restriction (same as `workflow_dispatch`), so firing one
  // here closes the gap without needing a personal access token secret.
  // Gated on the caller opting in (see dispatchOnNeedsReview above) and on
  // this being a genuine transition (toAdd, not merely "still needs
  // review") so it fires once per status change, mirroring what the
  // `labeled` event would have done if it could.
  if (dispatchOnNeedsReview && toAdd.includes(STATUS_LABELS.NEEDS_REVIEW.name)) {
    try {
      await github.rest.repos.createDispatchEvent({
        owner,
        repo,
        event_type: "needs-review",
        client_payload: { pr_number: prNumber },
      });
    } catch (err) {
      // Don't let a notification failure take down the label sync itself -
      // the PR still has the correct label and check run either way; worst
      // case the review job needs a manual `/claude-review` nudge.
      core.warning(`PR #${prNumber}: failed to dispatch needs-review event: ${err.message}`);
    }
  }

  await createGateCheck(github, owner, repo, pr.head.sha, status, summary, pending);

  // Auto-merge: every deterministic gate this function computes (CI green,
  // no merge conflict, no coverage/duplicate-code failure, no active
  // changes-requested verdict) plus a genuine approval is already folded
  // into `status === READY_TO_MERGE` itself now (see hasGenuineApproval
  // above and skills/review/SKILL.md) - reaching this branch already
  // implies `approved`, nothing left to re-derive here.
  if (status.name === STATUS_LABELS.READY_TO_MERGE.name) {
    if (!resolveAutoMerge({ autoMergeEnabled, autoMergeEnabledRaw, core })) {
      core.info(
        `PR #${prNumber}: ready to merge with a genuine approval, but auto-merge is switched off - leaving it for a human to merge.`,
      );
    } else {
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
// Exported so both workflow call sites turn the AUTO_MERGE_ENABLED variable
// into the `autoMergeEnabled` boolean the same way - the parsing lives here,
// not duplicated in two `script:` blocks that could drift apart.
module.exports.parseAutoMergeEnabled = parseAutoMergeEnabled;
// Exported for the tests that pin the guard: auto-merge must never land a
// change to `.github/`, which is where every gate it trusts lives.
module.exports.touchesProtectedPaths = touchesProtectedPaths;
// Exported for the tests that pin which of the two auto-merge inputs wins.
module.exports.resolveAutoMerge = resolveAutoMerge;
// Exported so a test can drive the enforcement point itself, not just the
// predicate it consults: an inverted condition or a dropped `return` here
// would merge exactly the changes the guard exists to hold back, and these
// scripts are not lcov-instrumented, so `node --test` is the only net.
module.exports.autoMergeIfApproved = autoMergeIfApproved;
module.exports.AUTO_MERGE_PROTECTED_PREFIXES = AUTO_MERGE_PROTECTED_PREFIXES;
module.exports.AUTO_MERGE_PROTECTED_CONFIG_DIRS = AUTO_MERGE_PROTECTED_CONFIG_DIRS;
module.exports.AUTO_MERGE_PROTECTED_BASENAMES = AUTO_MERGE_PROTECTED_BASENAMES;
module.exports.autoMergeBlockedReason = autoMergeBlockedReason;
module.exports.LISTED_FILES_CAP = LISTED_FILES_CAP;
// Exported so pre-review-checks.js can exclude this workflow's own derived,
// circular check run (its conclusion depends on the review having already
// happened) from the "wait for every other check on this commit" gate.
module.exports.GATE_CHECK_NAME = GATE_CHECK_NAME;
