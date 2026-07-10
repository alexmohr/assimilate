<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Review Skill

Use when:

* reviewing a pull request
* responding to review comments on your own PR

## Required

* If not stated otherwise, pull requests should always be squashed during merge.
* Reviews must be done with maximum strictness; findings are never optional.
* Logic is never to be duplicated instead of reused — flag any duplication found.
* New code shall have test coverage.
* User-facing functions should be tested via e2e tests.
* Use GitHub's review and changes-requested functions for all reviews — submit the verdict through GitHub's native review API (approve / request changes) whenever the reviewing account differs from the PR author. GitHub rejects self-approval outright (`422: Can not approve your own pull request`), so when the same account authored the PR, record the verdict via the `claude-approved` / `claude-changes-requested` labels instead (still post the actual findings as review comments either way). No other agent may touch these two labels by hand. See "PR status labels" below.
* If a PR is behind the main branch, this must be flagged as changes requested. Only a fully rebased branch is acceptable.
* When a comment is addressed, the agent shall reply with "solved please re-review" and leave the comment unresolved.
* The agent that addresses a review comment must never resolve it — only the comment author resolves it.

## Workflow

1. Check whether the PR is rebased on the latest default branch. If not, request changes citing that alone as a finding. If `.github/workflows/claude-review.yml` already ran on this commit, its pre-flight comment already reports rebase status (and issue-linking syntax) — read that instead of re-deriving it; both are informational there, not a hard gate, so use judgment on whether they're worth a finding.
2. Review the diff for correctness, duplicated logic, and test coverage (unit + e2e for user-facing functions) per the Required rules above. If the automated pipeline already ran, coverage-diff and duplicate-code detection are hard gates enforced *before* you're invoked (see "Automated Claude review" below) — don't re-scan for either from scratch; focus on correctness and whether the tests that exist are the right *kind* of test, not raw coverage.
3. Post findings via GitHub's native review tools (inline comments / review body), not as free-form chat replies.
4. Record the verdict:
   * **Different account than the PR author**: submit the review as **Request changes** if any finding exists, or **Approve** once none remain.
   * **Same account as the PR author**: submit the review as **Comment** (carries the inline findings; GitHub allows this on your own PR) and then apply `claude-changes-requested` if any finding exists, or `claude-approved` once none remain — remove whichever of the two was previously set. Never apply any other status label yourself.

   Either way, do not set `needs review` / `ready to merge` / etc. yourself — the `PR Status Labels & Merge Gate` workflow (`.github/workflows/pr-status-labels.yml`) derives them automatically from the review verdict and CI. See below.
5. When changes are pushed addressing a specific comment, reply "solved please re-review" on that comment and leave it unresolved — do not resolve it yourself.

## PR status labels

`.github/workflows/pr-status-labels.yml` keeps a small set of mutually-exclusive
status labels in sync automatically, driven only by two objective signals: the
`CI` workflow's conclusion on the head commit, and the review decision. The
review decision is normally GitHub's native `reviewDecision`
(`pulls.pull_request_review_write` approve/request-changes) — but GitHub
refuses to record an approval from the PR's own author, so when the same
account authored and reviewed the PR, the workflow instead reads the
`claude-approved` / `claude-changes-requested` labels described in the
Workflow section above. A real other-account review always takes priority
over those two labels if both exist. This rerun happens on every push, every
submitted/dismissed review, every `claude-approved`/`claude-changes-requested`
label change, every `precheck failed`/`merge conflict` change, and every CI
completion, so the status label always reflects current reality — **agents
must never add or remove the status labels themselves** (`needs review`,
`changes requested`, `ci failing`, `merge conflict`, `precheck failed`,
`ready to merge`, `needs human review`); only push a fix, submit a review, or
set the verdict labels per the Workflow section to move them.

| Label | Meaning | Set when |
|---|---|---|
| `needs review` | No blocking verdict yet | Default state; also holds while `needs human review` is outstanding |
| `changes requested` | A reviewer requested changes | GitHub review decision is `CHANGES_REQUESTED` |
| `ci failing` | Latest commit's CI run did not succeed | `CI` workflow conclusion is not `success` — always wins, and strips `ready to merge` |
| `merge conflict` | Real conflicts with the base branch | `mergeable_state == "dirty"` — checked continuously (it's a free API field), same precedence tier as `ci failing` |
| `precheck failed` | A deterministic pre-review check failed | Set only by `.github/scripts/pre-review-checks.js` (coverage-diff or duplicate-code hard gate) — never by a reviewer, human or AI. See "Automated Claude review" below |
| `ready to merge` | Fully clear to merge | CI conclusion is `success` **and** review decision is `APPROVED` **and** no human sign-off is pending |
| `needs human review` | Requires a human's sign-off before merge, regardless of agent review | Auto-applied when the diff touches security/crypto/auth/SSH-forwarding code, CI/CD workflow files, dependency lockfiles, `deny.toml`, DB migrations, or adds a new `#[allow(...)]`/`deny.toml` `ignore` suppression |

`needs human review` is additive-only: the workflow will add it but will
**never** remove it — only a human clearing the label counts as sign-off. Even
a PR that is fully approved and green stays capped at `needs review` while
this label is present. If your review touches one of the sensitive areas
above, expect this label and do not treat an agent approval as sufficient to
merge.

### Same-account review verdict labels

`claude-approved` and `claude-changes-requested` are a separate pair of
labels, not part of the status set above — they only exist to carry a review
verdict when GitHub can't record a native one (same-account review). Set
**only** by the reviewing agent per the Workflow section, read by the status
workflow as a stand-in for `reviewDecision`. A push (`synchronize`) clears
whichever of the two is present, since new commits invalidate the prior
verdict and the PR needs a fresh review — same as GitHub does for native
reviews via "dismiss stale reviews," which doesn't apply to labels
automatically.

### Automated Claude review

`.github/workflows/claude-review.yml` reviews a PR automatically once it's
labeled `needs review` (and again whenever CI finishes, in case the label
landed while CI was still pending) — but never spends a Claude turn until
deterministic, code-only checks have passed:

1. It force-reruns the label sync (`sync-pr-labels.js` directly, not a
   re-derived judgment call) — if the result is `ci failing` or
   `merge conflict`, it stops. Nothing to review yet.
2. If a `claude-approved`/`claude-changes-requested` label is already present,
   this exact commit has already been reviewed — it stops (skip the wasted
   token spend), unless invoked via `/claude-review` (see below), which
   always forces a fresh run.
3. Coverage-diff: every new/changed line must have test coverage, and the
   PR's aggregate line coverage must not be lower than the latest `main`
   baseline (zero tolerance — this catches removed/weakened tests even when
   the source lines they used to cover weren't touched by the diff).
4. Duplicate-code scan (`jscpd`), filtered to clusters touching this PR's
   changed files.

Either (3) or (4) failing sets `precheck failed` with the specific findings
posted as a PR comment, and Claude is not invoked — fix the findings and push
(which clears the label and re-triggers review) rather than waiting on a
review that mechanically can't happen yet. Rebase-behind-main and
issue-linking-syntax checks also run but are informational only — they don't
block Claude, they're just handed to it (or posted) as pre-known facts so it
doesn't have to re-derive them.

**Model:** defaults to `claude-sonnet-5` (overridable repo-wide via the
`CLAUDE_REVIEW_MODEL` Actions variable). If Claude's review fails outright
(quota exhausted, action error), the workflow posts a plain comment instead
of a fake verdict — the PR still needs a review, from a human or any other
agent, through the same native-review/fallback-label channels documented
above; nothing about `ready to merge` depends on Claude specifically.

**Manual retrigger:** comment `/claude-review` on the PR (requires write
access — org member/collaborator/owner) to force a fresh review through the
same pipeline, e.g. after a quota outage or to get a second opinion. Add
`model=<id>` to use a specific model for that one run instead of the default,
e.g. `/claude-review model=claude-opus-4-8` for a harder PR. Allowed models:
`claude-sonnet-5`, `claude-opus-4-8`, `claude-haiku-4-5`.

### Merge gate (enforced, not just informational)

The same workflow publishes its verdict as a check run named `PR Merge Gate`
on the head commit — `success` only when the status is `ready to merge`,
`failure` otherwise, with a summary explaining why. Labels alone are
advisory (nothing stops a human from clicking "Merge" on a red-labeled PR);
`PR Merge Gate` makes the verdict machine-enforceable once it is added as a
**required status check** in the repo's branch protection settings
(Settings → Branches → Branch protection rule for `main` → Require status
checks to pass → add `PR Merge Gate`). That's a one-time, repo-owner-only
change — agents must not attempt to modify branch protection themselves.

## Validation checklist

* [ ] Rebase status checked and flagged if stale
* [ ] Every finding filed as an actual GitHub review comment, not just prose
* [ ] Duplicated logic called out
* [ ] Test coverage (unit + e2e where user-facing) checked
* [ ] Verdict recorded correctly for the account situation: native approve/request-changes if reviewing a different account's PR, or `claude-approved`/`claude-changes-requested` if reviewing your own account's PR — status labels themselves are never set by hand
* [ ] Own comments never self-resolved; "solved please re-review" used instead
