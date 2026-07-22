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
2. Review the diff for correctness, duplicated logic, and test coverage (unit + e2e for user-facing functions) per the Required rules above. If the automated pipeline already ran, coverage-diff and duplicate-code detection are hard gates enforced *before* you're invoked (see "Automated pre-flight checks" below) — don't re-scan for either from scratch; focus on correctness and whether the tests that exist are the right *kind* of test, not raw coverage.
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
label change, every `coverage failed`/`duplicate code`/`merge conflict`
change, and every CI completion, so the status label always reflects current
reality — **agents must never add or remove the status labels themselves**
(`needs review`, `changes requested`, `ci failing`, `merge conflict`,
`precheck failed`, `ready to merge`, `needs human review`, `coverage failed`,
`duplicate code`); only push a fix, submit a review, or set the verdict
labels per the Workflow section to move them.

| Label | Meaning | Set when |
|---|---|---|
| `needs review` | No blocking verdict yet | Default state; also holds while `needs human review` is outstanding |
| `changes requested` | A reviewer requested changes | GitHub review decision is `CHANGES_REQUESTED` |
| `ci failing` | Latest commit's CI run did not succeed | `CI` workflow conclusion is not `success` — always wins, and strips `ready to merge` |
| `merge conflict` | Real conflicts with the base branch | `mergeable_state == "dirty"` — checked continuously (it's a free API field), same precedence tier as `ci failing` |
| `precheck failed` | A deterministic pre-review stage failed | **Purely derived** — `sync-pr-labels.js` computes it fresh every run from `coverage failed` and/or `duplicate code`, never set directly by anything. This is the one label to look at if you just want "did any pre-flight stage fail" without caring which. See "Automated pre-flight checks" below |
| `coverage failed` | The coverage-diff pre-review stage failed | Set only by `.github/scripts/analyze-coverage-diff.js` via the standalone `.github/workflows/coverage-diff-check.yml`. See "Automated pre-flight checks" below |
| `duplicate code` | The duplicate-code-scan pre-review stage failed | Set only by `.github/scripts/analyze-duplication.js` via the standalone `.github/workflows/duplicate-code-check.yml`. See "Automated pre-flight checks" below |
| `ready to merge` | Fully clear to merge | CI conclusion is `success` **and** no human sign-off is pending **and** neither `coverage failed` nor `duplicate code` is set **and** the review decision is not `CHANGES_REQUESTED`. An approving review is *not* required — see note below the table |
| `needs human review` | Requires a human's sign-off before merge, regardless of agent review | Auto-applied when: the diff touches security/crypto/auth/SSH-forwarding code, CI/CD workflow files, `.github/scripts/`, `.pre-commit-config.yaml`, `.devcontainer/`, dependency lockfiles, `deny.toml`, or DB migrations; the diff adds a new `#[allow(...)]`/`deny.toml` `ignore` suppression; the PR title or body mentions "security"; or the PR closes an issue whose title, body, or labels mention "security" |

`ready to merge` does not require an approving review. Waiting on an
approval when CI hasn't even confirmed the commit builds/passes is a
contradiction — nobody should approve a red build — so the deterministic
gates (CI, merge conflicts, coverage/duplication, an active
`CHANGES_REQUESTED` verdict, sensitive-path sign-off) are what actually
matter; the absence of an approval by itself is not a blocker. A review is
still worth doing and still worth requesting via the normal flow (Workflow
section above) — an explicit `changes requested` verdict still blocks the
gate the same as any other failing precheck.

`ready to merge` also requires `coverage-diff-check.yml` and
`duplicate-code-check.yml` to have each actually **completed** a check run
on this exact commit — not merely that neither's failure label is currently
set. Both trigger off the same `workflow_run: CI completed` event
`pr-status-labels.yml` itself reacts to, with no ordering guarantee between
any of them (see "How Claude's gate uses them" below) — a check that simply
hasn't been scheduled yet is indistinguishable from "nothing to wait for"
to a snapshot that only looks at checks which already exist, so without
this, `sync-pr-labels.js` could grant `ready to merge` in the instant
before either stage has even started analyzing the commit. Confirmed live
on PR #373: a stale `coverage failed` label sat next to an already-passing
"Coverage Diff Check" run for hours with no fresh sync ever reconciling
them, exactly the kind of inconsistency this guards against.

`coverage failed` and `duplicate code` are two independent stages and can
both be present on a PR at once — neither erases the other. Each owns its
own add/remove lifecycle end to end in its own script
(`analyze-coverage-diff.js`, `analyze-duplication.js`) rather than being
cleared by the generic `synchronize` handling, since both can run off the
same push/CI-completion events `pr-status-labels.yml` reacts to, and a blind
clear there could otherwise race a fresh failing result. `precheck failed`
is never set or cleared directly — it's recomputed as a pure
`coverage failed OR duplicate code` umbrella every time `sync-pr-labels.js`
runs, from whichever of the two labels currently exist, so it can't go stale
independently of its inputs.

`needs human review` is additive-only: the workflow will add it but will
**never** remove it — only a human clearing the label counts as sign-off. Even
a PR that is fully approved and green stays capped at `needs review` while
this label is present. If your review touches one of the sensitive areas
above, expect this label and do not treat an agent approval as sufficient to
merge. The "security" text/issue checks are a keyword match, not a judgment
call — expect occasional false positives (a PR that merely discusses security
in passing) and treat them as a cheap net that widens the gate, not a
guarantee that every gated PR is actually security-critical.

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

### Automated pre-flight checks

This repo adds two deterministic, code-only pre-flight stages on top of
`CI` itself, each entirely owned by its own standalone workflow — analysis,
PR comment, status label, **and** a GitHub check run on the commit. Neither
workflow is ever re-triggered or re-run from `claude-review.yml`; the
Claude-gating check (below) waits for check runs, not just these two, to
know when it's safe to proceed. See "How Claude's gate uses them" below for
why a check run, not a label, is what's waited on.

* **Duplicate-code scan** — `.github/workflows/duplicate-code-check.yml`
  runs standalone on every `opened`/`synchronize`/`reopened` event; it
  doesn't wait for CI or the `needs review` label. It checks out the PR
  head, runs `jscpd` over the whole tree, and hands the JSON report to
  `.github/scripts/analyze-duplication.js`, which keeps only clusters that
  touch this PR's changed files. What counts as "generated, never flag it"
  (the sqlx offline query cache in `.sqlx/`, generated frontend types,
  lockfiles, build output, ...) is configured in `.jscpd.json` at the repo
  root — extend that file's `ignore` array to exempt new generated code, no
  workflow change needed. On a hit, the workflow posts a PR comment with the
  actual duplicated source for each cluster (not just file:line ranges),
  sets its own `duplicate code` label, and publishes a "Duplicate Code
  Check" check run on the commit. Because it's a separate workflow, it
  isn't re-triggered by `/claude-review` — only a new push re-runs the
  duplication scan itself.
* **Coverage-diff** — `.github/workflows/coverage-diff-check.yml` runs
  standalone on every CI completion (it needs CI's coverage artifact, so it
  can't run any earlier than that). It locates the PR's own CI run and the
  latest successful `main` CI run, downloads both `coverage-final` lcov
  artifacts, and hands them to `.github/scripts/analyze-coverage-diff.js`:
  every new/changed line must have test coverage, and the PR's aggregate
  line coverage must not be lower than the `main` baseline (zero tolerance
  — this catches removed/weakened tests even when the source lines they used
  to cover weren't touched by the diff). A failure posts its own PR comment,
  sets its own `coverage failed` label, and publishes a "Coverage Diff
  Check" check run on the commit, same pattern as duplication above.

#### How Claude's gate uses them

`pre-review-checks.js` (run from `claude-review.yml`) never runs any
pre-flight check itself and never triggers any of their workflows — it only
*waits*, via `.github/scripts/lib/wait-for-check.js`, for **every** check
run currently on the PR's head commit (not a fixed list of named checks) to
reach `status: completed`, up to a 2-hour ceiling, then requires all of
their conclusions to be `success`, `skipped`, or `neutral`. This
automatically covers "Coverage Diff Check", "Duplicate Code Check", every
individual `CI` job (rust, frontend, e2e, docs, ...), and anything else
added to the pipeline later - no change to this script needed when a new
check is introduced. Two check runs are explicitly excluded from the wait,
both to avoid nonsensical outcomes rather than to skip real signal:

* `claude-review.yml`'s own two jobs ("Check if a review is actually
  needed", "Review PR") - the latter is literally the job this script is
  running inside, so waiting on it would wait forever.
* `PR Merge Gate` - it's a *derived*, point-in-time check: `pre-review-
  checks.js` force-syncs labels (and republishes this exact check run) right
  before it starts waiting, so the freshly-created run already reflects
  whatever the deterministic gates (coverage-diff, duplicate-code, CI) look
  like *at that instant* - almost certainly still pending or failing, since
  those other checks haven't necessarily finished yet. Every sync publishes
  a brand-new check run rather than updating one in place, so this snapshot
  never changes to `success` on its own; waiting on it would make
  `run_claude` permanently false.

This is deliberate: `coverage-diff-check.yml` and `claude-review.yml` both
trigger on the same `workflow_run: CI completed` event with no ordering
guarantee between them, so anything short of waiting for an already-finished,
authoritative result could gate Claude on stale or incomplete data - a label
read at the wrong moment could be missing not because the check passed, but
because it simply hasn't run yet. Earlier iterations of this design either
re-ran checks inline (which raced the standalone workflows' own
comment-writing and produced duplicate PR comments) or trusted specific
labels directly (which could read stale state, or relied on one stage
happening to finish before another with no real guarantee); waiting for
every check run to reach a completed, authoritative conclusion removes the
race entirely rather than narrowing it, and generalizes to whatever checks
exist rather than needing to know their names in advance.

If there's already a known-bad signal — `ci failing` or `merge conflict` —
`pre-review-checks.js` exits immediately on that, without waiting for
anything; there's nothing to gain from waiting on the rest once the PR is
already blocked for an unrelated, faster-to-detect reason. If not everything
completes within the 2-hour wait, it treats that as inconclusive and does
not invoke Claude either — the same "deny on any pipeline failure" bias
applies to "we couldn't confirm it passed" as it does to "we confirmed it
failed."

Fix the findings and push: coverage-diff-check.yml and duplicate-code-check.yml
each re-run automatically, clear their own label, and publish a fresh check
run once clean. Rebase-behind-main and issue-linking-syntax checks also run
but are informational only — they don't block Claude, they're just handed
to it (or posted) as pre-known facts so it doesn't have to re-derive them.

### Automated Claude review

`.github/workflows/claude-review.yml` reviews a PR automatically once it's
labeled `needs review` (and again whenever CI finishes, in case the label
landed while CI was still pending) — but never spends a Claude turn until
the pre-flight checks above have passed.

The workflow itself has two jobs: a small `gate` job, and the actual `review`
job it feeds via `needs:`/`if:`. This exists because the `workflow_run: CI
completed` trigger fires on every push to every open PR, not just ones
actually waiting on a review — without the gate, the full `review` job
(checkout, waiting on pre-flight checks, potentially Claude) would spin up
every time regardless. `gate` checks whether the PR currently has the `needs
review` label for CI-completion events (the label-landing and `/claude-review`
triggers are already precise, so `gate` passes them through unconditionally)
and only lets `review` start if so. This is a coarse, cheap filter, not the
authoritative decision - `review` still does its own full, fresh check once
it starts:

1. It force-reruns the label sync (`sync-pr-labels.js` directly, not a
   re-derived judgment call) — if the result is `ci failing` or
   `merge conflict`, it stops. Nothing to review yet.
2. If a `claude-approved`/`claude-changes-requested` label is already present,
   this exact commit has already been reviewed — it stops (skip the wasted
   token spend), unless invoked via `/claude-review` (see below), which
   always forces a fresh run.
3. It waits for every other check run on the commit to complete (see "How
   Claude's gate uses them" above) and stops if any of them failed, or if
   they didn't all complete within the wait window.

**Tool access:** `anthropics/claude-code-action@v1`'s own built-in MCP tools
deliberately don't include review-verdict submission, label management, or
fetching a PR's diff — only CI-status lookups, a single tracked comment,
file edits, and `create_inline_comment` (itself explicitly scoped down so
Claude can't use it to approve a PR). Everything else goes through `gh`
instead: `claude_args` in `claude-review.yml` grants Bash access to a
specific set of subcommands (not blanket Bash access) — `gh pr diff`/`gh pr
view`/`git log`/`git diff`/`git show` to actually see the change, `gh pr
review`/`gh pr edit` for the verdict — and a `GH_TOKEN` env var on that step
authenticates `gh`. The prompt tells Claude to start with `gh pr diff`/`gh
pr view` before forming an opinion, to use `gh pr review
--approve|--request-changes` for the native verdict path and `gh pr edit
--add-label|--remove-label` for the same-account label path. It's told
explicitly never to submit a placeholder/test verdict, and never to merge
the PR itself — merging is not part of the review job at all; see "Merge
gate" below for how it actually happens.

**Model:** defaults to `claude-sonnet-5` (overridable repo-wide via the
`CLAUDE_REVIEW_MODEL` Actions variable). If Claude's review fails outright
(quota exhausted, action error), the workflow posts a plain comment instead
of a fake verdict — the PR still needs a review, from a human or any other
agent, through the same native-review/fallback-label channels documented
above; nothing about `ready to merge` depends on Claude specifically.

**Manual retrigger:** comment `/claude-review` on the PR (requires write
access — org member/collaborator/owner) to force a fresh review through the
claude-review.yml pipeline (label sync, waiting on pre-flight checks,
Claude), e.g. after a quota outage or to get a second opinion. Add
`model=<id>` to use a specific model for that one run instead of the
default, e.g. `/claude-review model=claude-opus-4-8` for a harder PR.
Allowed models: `claude-sonnet-5`, `claude-opus-4-8`, `claude-haiku-4-5`.
This does **not** re-run `coverage-diff-check.yml` or
`duplicate-code-check.yml` themselves — it only waits for whatever their
check runs already say about the current commit; if you need either
re-checked, that needs a new commit (or, for coverage, a fresh CI run).

### Merge gate (enforced, not just informational)

`pr-status-labels.yml` (via `sync-pr-labels.js`) publishes its verdict as a
check run named `PR Merge Gate` on the head commit — `success` only when the
status is `ready to merge`, `failure` otherwise, with a summary explaining
why. Labels alone are advisory (nothing stops a human from clicking "Merge"
on a red-labeled PR); `PR Merge Gate` makes the verdict machine-enforceable
once it is added as a **required status check** in the repo's branch
protection settings (Settings → Branches → Branch protection rule for
`main` → Require status checks to pass → add `PR Merge Gate`). That's a
one-time, repo-owner-only change — agents must not attempt to modify branch
protection themselves.

### Auto-merge (deterministic, not agent-driven)

**Currently disabled by default**, gated behind the `AUTO_MERGE_ENABLED`
repository (or environment) Actions variable — set it to the literal
string `true` (Settings → Secrets and variables → Actions → Variables) to
turn it on. It defaults off because merging code with no human clicking a
button deserves the pipeline having actually earned that trust first, not
because the mechanism itself is provisional — every gate described below
(ready to merge, a genuine approval, the label-provenance check) runs and
logs its decision regardless of the flag; turning it on later is purely a
config change; no code change needed. See
[#390](https://github.com/alexmohr/assimilate/issues/390) for what should
be true before flipping it.

The same `sync-pr-labels.js` run that computes `ready to merge` also
squash-merges the PR itself (`--delete-branch` for same-repo branches) the
moment **all** of the following hold, every time it re-syncs (every push,
review, label change, or CI completion — not just the instant a review is
submitted):

* `status === ready to merge` — CI green, no merge conflict, no
  `coverage failed`/`duplicate code`, no active `changes requested` verdict,
  and no pending `needs human review` sign-off (all the same gates the label
  itself already requires — see the table above).
* **A genuine approval** — either GitHub's native `reviewDecision` is
  `APPROVED` (a different-account review; GitHub itself guarantees this is a
  real, distinct reviewer and never a self-approval), or the same-account
  `claude-approved` label is set. `ready to merge` on its own does **not**
  require an approval (see the note under the table), so this is checked
  independently — merging still needs one even though the status label
  doesn't.

This is deliberately **not** something the reviewing agent does itself
anymore — `claude-review.yml`'s prompt explicitly tells Claude never to run
`gh pr merge`; it ends its job at submitting the verdict. Moving the merge
decision into the same deterministic script that already computes every
other gate means it re-fires on its own whenever anything relevant changes
(e.g. a human clearing `needs human review` on an already-approved PR later)
rather than existing only as a one-shot action inside a single review turn
that could be skipped, time out, or simply never run again.

**Label forgery protection:** unlike a native review, `claude-approved` is
an ordinary label — anyone with triage-level (or higher) repo access can add
any label to any PR by hand via the UI or their own token, with no review
ever having happened. Merging on the label's mere presence would let that
forge a clean verdict. Before trusting it for merging, `sync-pr-labels.js`
checks the PR's timeline for the most recent event that added
`claude-approved` and requires its actor to be `github-actions[bot]` — the
identity both `claude-review.yml`'s `gh pr edit --add-label` call and this
workflow's own label mutations run under. A label added by any other
account is left in place (still advisory for the status labels above) but
is never trusted to trigger a merge.

`coverage-diff-check.yml` and `duplicate-code-check.yml` each publish their
own check run too ("Coverage Diff Check", "Duplicate Code Check") — these
exist primarily so `pre-review-checks.js` has something authoritative to
poll (see "How Claude's gate uses them" above), but they're ordinary check
runs and could also be added as required status checks the same way
`PR Merge Gate` is, if the repo owner wants either stage to block merging
directly rather than only through the derived `precheck failed`/`ready to
merge` labels.

## Validation checklist

* [ ] Rebase status checked and flagged if stale
* [ ] Every finding filed as an actual GitHub review comment, not just prose
* [ ] Duplicated logic called out
* [ ] Test coverage (unit + e2e where user-facing) checked
* [ ] Verdict recorded correctly for the account situation: native approve/request-changes if reviewing a different account's PR, or `claude-approved`/`claude-changes-requested` if reviewing your own account's PR — status labels themselves are never set by hand
* [ ] Own comments never self-resolved; "solved please re-review" used instead
