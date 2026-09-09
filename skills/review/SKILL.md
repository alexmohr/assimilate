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
* Record every review verdict via the `claude-approved` / `claude-changes-requested` labels — never submit a formal native **Approve**/**Request changes** review. A CI-triggered review runs under a fresh GitHub Actions token every time, which can never formally approve a PR at all (`422: GitHub Actions is not permitted to approve pull requests`) and can't reliably clear a `CHANGES_REQUESTED` review it submitted in an earlier run either (dismissing a stale review needs extra bookkeeping a plain label swap doesn't); the labels sidestep both problems since they can always be added or removed outright. Still post the actual findings as review comments (a **Comment**-type native review, or inline comments) either way. No other agent may touch the two verdict labels by hand. See "PR status labels" below.
* If a PR is behind the main branch, this must be flagged as changes requested. Only a fully rebased branch is acceptable.
* When a comment is addressed, the agent shall reply with "solved please re-review" and leave the comment unresolved.
* The agent that addresses a review comment must never resolve it — only the comment author resolves it.

## Workflow

1. Check whether the PR is rebased on the latest default branch. If not, request changes citing that alone as a finding. If `.github/workflows/claude-review.yml` already ran on this commit, its pre-flight comment already reports rebase status (and issue-linking syntax) — read that instead of re-deriving it; both are informational there, not a hard gate, so use judgment on whether they're worth a finding.
2. Review the diff for correctness, duplicated logic, and test coverage (unit + e2e for user-facing functions) per the Required rules above. If the automated pipeline already ran, coverage-diff and duplicate-code detection are hard gates enforced *before* you're invoked (see "Automated pre-flight checks" below) — don't re-scan for either from scratch; focus on correctness and whether the tests that exist are the right *kind* of test, not raw coverage.
3. Post findings via GitHub's native review tools (inline comments / review body), not as free-form chat replies.
4. Record the verdict: submit the review as **Comment** (carries the inline findings; GitHub allows this on any PR, including your own) and then apply `claude-changes-requested` if any finding exists, or `claude-approved` once none remain — remove whichever of the two was previously set. Never apply any other status label yourself, and never submit a formal **Approve**/**Request changes** native review.

   Do this the same way regardless of whether the reviewing account differs from the PR author. Do not set `needs review` / `ready to merge` / etc. yourself — the `PR Status Labels & Merge Gate` workflow (`.github/workflows/pr-status-labels.yml`) derives them automatically from the review verdict and CI. See below.
5. When changes are pushed addressing a specific comment, reply "solved please re-review" on that comment and leave it unresolved — do not resolve it yourself.

## PR status labels

`.github/workflows/pr-status-labels.yml` keeps a small set of mutually-exclusive
status labels in sync automatically, driven only by two objective signals: the
`CI` workflow's conclusion on the head commit, and the review decision. A human
maintainer can still leave a genuine native review via the GitHub UI, in which
case the review decision is GitHub's own `reviewDecision`
(`pulls.pull_request_review_write` approve/request-changes) — but every
automated review (see Workflow above) always records its verdict via the
`claude-approved` / `claude-changes-requested` labels instead, per the Required
rules above, so the workflow reads those when no native decision applies. A
real, distinct-account native review always takes priority over those two
labels if both exist. This rerun happens on every push, every
submitted/dismissed review, every `claude-approved`/`claude-changes-requested`
label change, every `coverage failed`/`duplicate code`/`merge conflict`
change, and every CI completion, so the status label always reflects current
reality — **agents must never add or remove the status labels themselves**
(`pending`, `needs review`, `changes requested`, `ci failing`, `merge conflict`,
`check failed`, `precheck failed`, `ready to merge`,
`coverage failed`, `duplicate code`); only push a fix, submit a review, or
set the verdict labels per the Workflow section to move them.

| Label | Meaning | Set when |
|---|---|---|
| `pending` | Nothing to review yet | Default state — CI hasn't concluded on this commit yet (and no other check has already failed either), or CI is green but a precheck stage (coverage-diff, duplicate-code) hasn't finished |
| `needs review` | CI (and every check/precheck stage) is green; no other blocking verdict yet, but not ready to merge either | Set once CI — and every other check on the commit — has actually concluded with nothing failing (see `pending`/`check failed` above) and nothing more specific below applies — including while a review verdict has gone stale, or the last automated review attempt errored. Never set while any stage (other than the derived `PR Merge Gate` check itself) has already failed |
| `changes requested` | A reviewer requested changes | GitHub review decision is `CHANGES_REQUESTED` — fires regardless of whether CI has concluded on this exact commit yet, since a real, current review verdict is meaningful on its own |
| `ci failing` | Latest commit's CI run did not succeed | `CI` workflow conclusion is not `success` — always wins, and strips `ready to merge` |
| `merge conflict` | Real conflicts with the base branch | `mergeable_state == "dirty"` — checked continuously (it's a free API field), same precedence tier as `ci failing` |
| `check failed` | Some check on the commit other than CI itself, coverage-diff, or duplicate-code failed (e.g. `no-ai-check.yml`) | A single-shot look at every check run on the commit (excluding only the derived `PR Merge Gate` and the calling workflow's own in-progress job) finds one that's completed with a failing conclusion. Checked independently of whether CI itself has concluded yet — an already-failed check can never be un-failed by more waiting, so this settles the status immediately rather than reporting `pending` |
| `precheck failed` | A deterministic pre-review stage failed | **Purely derived** — `sync-pr-labels.js` computes it fresh every run from `coverage failed` and/or `duplicate code`, never set directly by anything. This is the one label to look at if you just want "did any pre-flight stage fail" without caring which. See "Automated pre-flight checks" below |
| `coverage failed` | The coverage-diff pre-review stage failed | Set only by `.github/scripts/analyze-coverage-diff.js` via the standalone `.github/workflows/coverage-diff-check.yml`. See "Automated pre-flight checks" below |
| `duplicate code` | The duplicate-code-scan pre-review stage failed | Set only by `.github/scripts/analyze-duplication.js` via the standalone `.github/workflows/duplicate-code-check.yml`. See "Automated pre-flight checks" below |
| `ready to merge` | Fully clear to merge | CI conclusion is `success` **and** neither `coverage failed` nor `duplicate code` is set **and** the review decision is not `CHANGES_REQUESTED` **and** a genuine approval is on record — see note below the table |

`ready to merge` requires a **genuine approval**, on top of every
deterministic gate (CI, merge conflicts, coverage/duplication, an active
`CHANGES_REQUESTED` verdict, sensitive-path sign-off): either GitHub's
native `reviewDecision` is `APPROVED` (a genuine human review via the GitHub
UI — GitHub itself guarantees this is a real, distinct reviewer and never a
self-approval), or the `claude-approved` label is set **and**
the most recent event that added it was actually authored by this repo's
own automation (`github-actions[bot]`) — see "Label forgery protection"
below. A `claude-approved`/`reviewDecision: APPROVED` added by any other
account, or not present at all, leaves the PR at `needs review` even once
CI and every other check is green — a green build alone is not enough. A
review is requested via the normal flow (Workflow section above); an
explicit `changes requested` verdict still blocks the gate the same as any
other failing precheck.

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

The flip side of that requirement is that *something* has to re-run the sync
once each stage's check actually lands, and for coverage-diff nothing did.
Its check is published a few seconds after `pr-status-labels.yml`'s own sync
job has already read the commit's check list — both start together on the
same CI-completion event, but the sync is two API calls while coverage-diff
first locates two CI runs and downloads two artifacts, so the sync lost that
race every single time. It then published `PR Merge Gate` as `in_progress`
("CI is green, but still waiting on: Coverage Diff Check") and, with no
further event on that commit, the gate stayed there indefinitely. Observed
on PRs #452, #454 and #455 simultaneously, with the coverage check landing
3–5s after each sync read the list. `coverage-diff-check.yml`'s final
"Notify the merge gate" step now fires a `precheck-complete`
`repository_dispatch` once its check is genuinely on the commit, which
re-runs the sync (see "Automated Claude review" below for why a dispatch
rather than a `workflow_run` chain). `duplicate-code-check.yml` needs no
equivalent: it runs on `pull_request_target` at push time and its
`Detect duplicate code` job check completes long before CI does.

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

### Review verdict labels

`claude-approved` and `claude-changes-requested` are a separate pair of
labels, not part of the status set above — they only exist to carry an
automated review's verdict, since that path never submits a native review
decision (see the Required rules above). Set
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
  Check" check run on the commit, same pattern as duplication above. A final
  step then fires a `precheck-complete` `repository_dispatch` so the merge
  gate re-syncs against that freshly-published check — see the `ready to
  merge` section above for what went wrong without it.

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
labeled `needs review`, on every non-draft push, and whenever CI finishes —
but never spends a Claude turn until the pre-flight checks above have passed.

The `needs review` transition itself is delivered via a `repository_dispatch`
event (`sync-pr-labels.js`'s `dispatchOnNeedsReview`, fired only from
`pr-status-labels.yml`'s own call site), not the plain `labeled` webhook
event. GitHub does not fire a new workflow run for an event — including
`labeled` — produced by the triggering workflow's own `GITHUB_TOKEN`, which
is exactly how the label gets added; `pull_request_target: types: [labeled]`
is kept as a secondary trigger only for a human adding the label by hand.
Before this existed, the symptom looked like "the review job only starts if
you remove and re-add the label" — doing that by hand uses a real user
token, which isn't subject to the restriction.

`pr-status-labels.yml` consumes a second dispatch type of its own,
`precheck-complete`, for the same underlying reason plus one more: a
`workflow_run` chained off `coverage-diff-check.yml` would be useless, since
that workflow is itself `workflow_run`-triggered and so runs in the default
branch's context — `head_sha` is `main`'s and `workflow_run.pull_requests` is
empty, leaving no route back to the PR number. (The existing CI trigger works
only because `github.event.workflow_run` there describes the *CI* run, which
does sit on the PR's head branch.) A dispatch carries the PR number
explicitly in `client_payload`.

The workflow itself has two jobs: a small `gate` job, and the actual `review`
job it feeds via `needs:`/`if:`. This exists because the `workflow_run: CI
completed` trigger (and a plain push) fires on every open PR regardless of
state, not just ones actually waiting on a review — without the gate, the
full `review` job (checkout, waiting on pre-flight checks, potentially
Claude) would spin up every time regardless. The label-landing and
`/claude-review` triggers are already precise/trusted by construction (see
the workflow's own comments), so `gate` passes those straight through; for a
plain push or CI completing, it checks whether this PR has ever had *any*
review submitted, deliberately **not** whether it currently carries `needs
review` — the label's current value at the moment this job runs can't be
trusted, since this workflow and `pr-status-labels.yml` trigger off similar
events independently with no ordering guarantee, so it could easily still
reflect the previous commit's state either way. This is a coarse, cheap
filter, not the authoritative decision - `review` still does its own full,
fresh check once it starts:

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
review`/`gh pr edit` for the verdict — plus `Read`/`Grep`/`Glob` against the
already-checked-out PR worktree and `TodoWrite` for tracking a large review's
own progress, so a review of a large, many-file diff can check a claim
against the actual source instead of every such attempt being denied (which
on a big enough diff was observed to burn the entire turn budget on nothing
but denials, ending with `is_error: false` and no verdict ever submitted —
see PR #425 and PR #441). A `GH_TOKEN` env var on that step
authenticates `gh`. The prompt tells Claude to start with `gh pr diff`/`gh
pr view` before forming an opinion, to use `gh pr review --comment` to post
its findings as the review body and `gh pr edit --add-label|--remove-label`
for the verdict — never `gh pr review --approve|--request-changes` (see the
Required rules above). It's told
explicitly never to submit a placeholder/test verdict, and never to merge
the PR itself — merging is not part of the review job at all; see "Merge
gate" below for how it actually happens.

**Model:** defaults to `claude-sonnet-5` (overridable repo-wide via the
`CLAUDE_REVIEW_MODEL` Actions variable). If Claude's review fails outright
(quota exhausted, action error) *or* completes cleanly but never actually
posts a verdict (the `is_error: false`/zero-denials-visible failure mode
above, still possible on a diff large enough to exhaust the allowlisted
tools anyway), the workflow treats both the same way: it adds the `claude
review failed` label and posts a plain comment instead of a fake verdict —
detected by checking, after the run, whether this bot actually posted a
review against the commit under review, not just whether the action step
itself errored, so a run that quietly gives up no longer leaves the PR
looking untouched. The PR still needs a review, from a human (native GitHub
review) or any other agent (the verdict labels), through the same channels
documented above; nothing about `ready to merge` depends on Claude
specifically. Retry with `/claude-review` rather than removing and re-adding
`needs review` by hand — the label toggle is a fallback trigger, not a
guaranteed retry path, and doesn't by itself change anything about the
budget that caused the previous attempt to give up.

**Clearing a pre-existing formal review:** a PR that already carries a
bot-authored formal `CHANGES_REQUESTED` review from before this workflow
stopped filing native reviews (see the Required rules above) needs that
review cleared before it can reach `ready to merge` — GitHub's native
`reviewDecision` only updates on a *new* formal `APPROVED`/`CHANGES_REQUESTED`
submission, and this workflow's `Comment`-type reviews never supersede it.
`claude-review.yml`'s "Clear a stale bot-authored changes-requested review"
step handles this automatically: once a run ends with a clean `claude-approved`
verdict, it dismisses any of this bot's own `CHANGES_REQUESTED` reviews still
submitted against the PR's current head commit. No manual dismissal is
normally needed — it only comes up if that step itself fails or a review
predates it landing in the pipeline, in which case a human can dismiss the
stale review via the GitHub UI.

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

**On by default.** A PR that reaches `ready to merge` has already cleared
every deterministic gate the pipeline computes *and* carries a genuine,
provenance-checked approval, so it is squash-merged without waiting for a
human to click the button.

**The approval must cover the commit being merged.** A native `APPROVED`
review is provenance-safe — GitHub guarantees a real, distinct reviewer and
rejects self-approval — but it says nothing about *what* was approved. GitHub
dismisses an approval when a new commit lands only if the branch's protection
rules say to, and `sync-pr-labels.js` cannot see whether that setting is on,
so it does not assume it. Without this check, a reviewer who approved commit
A still reads as `APPROVED` after an unreviewed commit B, and once CI goes
green on B the automation would merge code no reviewer has looked at —
`changesRequestedIsCurrent` already re-checks the opposite verdict against
`pr.head.sha` for exactly this reason.

So `approvalIsCurrent` requires an `APPROVED` review submitted against the
current head (latest review per user, mirroring how GitHub computes
`reviewDecision`). It gates **auto-merge, not the `ready to merge` status**:
a human pressing the button is looking at the PR and that is their call,
whereas the unattended path is the whole question. Same shape as the
protected-path guard — the status stands, the button is not pressed, the
reason is logged. The `claude-approved` path is checked too, by
`claudeVerdictCoversHead`. It looks exempt, because `sync-pr-labels.js`
deletes that label on every `synchronize` event — but that clearing happens
**at push time**, and `claude-review.yml` captures the head sha once when a
run starts. A run pinned to the previous commit can still be in flight and
apply its verdict afterwards, and nothing clears it again until the next push.
The verdict's commit is recoverable because that workflow only counts a run as
having produced a verdict when the bot posted a review against the run's own
head sha (otherwise it sets `claude review failed`), so the bot's most recent
review names the commit the standing label is about.

`approvalCoversThisHead` picks between the two checks, so the paths cannot
drift apart and the choice itself is testable rather than buried in a call
site. Provenance (`claudeApprovedIsGenuine`) and currency are separate
questions and both are asked.

Neither verdict self-invalidates, which is the point both currency checks
exist to handle: a `CHANGES_REQUESTED` review keeps blocking forever, and an
`APPROVED` review keeps approving, until someone submits a new one or branch
protection dismisses it. The kill switch is the `AUTO_MERGE_ENABLED`
repository (or environment) Actions variable: set it to the literal string
`false` (Settings → Secrets and variables → Actions → Variables) to stop
merging. The value is trimmed and compared case-insensitively, so `False`,
`FALSE`, `no`, `off` and `0` all disable it too, and an unset variable means
on. Anything else that is set — including a value that is only whitespace,
which is a typo rather than a request for the default — is treated as **off**,
with a warning in the job log naming the raw value. A kill switch that failed
*open* on a typo would keep merging code unattended, which is the one
direction not worth guessing in. The switch only decides whether the merge
call happens — every gate below (ready to merge, a genuine approval, the
label-provenance check) runs and logs its decision either way, so flipping it
is purely a config change; no code change needed.

Both workflow call sites (`pr-status-labels.yml`'s sync and
`claude-review.yml`'s post-review re-sync) read the variable out of the step
environment rather than interpolating it into the `script:` body, and hand
the raw string to `sync-pr-labels.js` as `autoMergeEnabledRaw` — the module
parses it with `parseAutoMergeEnabled`.

Handing over data rather than calling into the script matters because the two
can come from different commits. `sync-pr-labels.js` is always checked out
from the default branch (`sparse-checkout .github/scripts`, `ref:
default_branch`), but on a `pull_request_review` event the *workflow file*
comes from the PR's head. A workflow body calling
`sync.parseAutoMergeEnabled(...)` therefore fails with "not a function" on
every PR that adds it, until that PR reaches the default branch — which is
exactly what happened while this was being built. An older script simply
ignores an unknown key and keeps its own default (off), so the skew degrades
to "no auto-merge" rather than to a broken label-sync job. `pre-review-checks.js`'s own sync
call pins auto-merge off explicitly: it runs *before* the review it gates, so
it must never be the thing that merges.

Note where "on by default" actually lives: in `parseAutoMergeEnabled`, not in
`syncLabels`. The `autoMergeEnabled` *parameter* defaults to **off**, and all
three call sites pass it explicitly, so that default is only ever reached by
a future caller that forgot to — and a forgotten argument must not be able to
merge code unattended.

The same `sync-pr-labels.js` run that computes `ready to merge` also
squash-merges the PR itself (`--delete-branch` for same-repo branches) the
moment `status === ready to merge`, every time it re-syncs (every push,
review, label change, or CI completion — not just the instant a review is
submitted). `ready to merge` already folds in a genuine approval as one of
its own gates (see the note under the table and "Label forgery protection"
below), so nothing further is checked independently before merging — CI
green, no merge conflict, no `coverage failed`/`duplicate code`, no active
`changes requested` verdict, and a genuine approval are all required for the
status itself.

**Auto-merge never lands a change to the rails.** Before merging,
`autoMergeIfApproved` lists the PR's files and bails if any of them — or any
`previous_filename`, so a rename out of a protected location can't launder one
— is one of the paths below, or if the list came back at GitHub's 3000-file
cap, where it is silently truncated and a rail edit past the cut simply would
not appear. An unprovable list counts as protected rather than as clean, since
a bulk or generated diff hiding a rail edit is the shape this guard exists to
catch.

The rails are everything that **defines or suppresses a gate**, as opposed to
the code the gates run over. The reason they are fenced off is that CI reads
every one of them from the PR's *own head*: a PR editing one takes effect on
the very run that decides whether that PR may merge, so it goes green because
it loosened the thing that would have failed it. `sync-pr-labels.js` matches
them with three structural rules rather than a list of known files:

**1. `AUTO_MERGE_PROTECTED_PREFIXES` — whole directory trees.**

| Prefix | Gate it controls |
|---|---|
| `.github/` | The workflows, the coverage-diff and duplicate-code analyzers, and `sync-pr-labels.js` itself, which decides what `ready to merge` means |
| `lints/` | The dylint library behind `AGENTS.md`'s no-string-control-flow rule; `ci.yml` builds it from the PR's checkout and runs it with `-D no_string_control_flow` |
| `frontend/eslint-rules/` | The frontend's mirror of that same rule, `no-string-literal-control-flow` |
| `scripts/` | Entry points for the repo's local pre-commit hooks (`check-no-raw-sqlx-queries.sh`, `no-typography-in-comments.py`) |

**2. `AUTO_MERGE_PROTECTED_CONFIG_DIRS` — immediate children only.** The two
places this repo keeps gate configuration: the **repository root** and
**`frontend/`**.

**Every** immediate child of those two directories is protected, not only the
files that happen to configure a gate today. `README.md`, `LICENSE`,
`AGENTS.md`, `Cargo.lock` and `Dockerfile.agent` all block auto-merge exactly
as `deny.toml` does. That is the point of the rule rather than a side effect:
listing only the known gate files is the enumeration this replaced, and the
next config file added at either level would land outside it. The gate configs
this currently covers, as examples and not as the list — root: `Cargo.toml`,
`deny.toml`, `.jscpd.json`, `.pre-commit-config.yaml`, `clippy.toml`,
`.rustfmt.toml`, `ruff.toml`, `.markdownlint.yaml`, `.yamlfmt`, `REUSE.toml`,
`mkdocs.yml`; `frontend/`: `package.json` (whose `lint`, `build`, `test` and
`format:check` scripts are what `ci.yml` actually invokes),
`package-lock.json`, `eslint.config.js`, the tsconfigs `vue-tsc -b` enforces,
`playwright.config.ts`, `vite.config.ts`, `.prettierrc`,
`.npm-audit-allowlist.json`.

Subdirectories are *not* covered — `crates/`, `docs/`, `skills/`,
`frontend/src/` and `frontend/e2e/` all stay auto-mergeable, which is what
keeps auto-merge useful at all.

**3. Any path whose top-level segment starts with a dot.** At a repository
root a dot-directory is tool configuration by convention, and every one this
repo has is a gate: `.github/` (the workflows and analyzers), `.sqlx/` (the
offline query cache sqlx's macros compile against, so an entry added there
makes a query build that the database would reject), `.reuse/` (the REUSE
hook's templates).

The two that matter most **don't exist yet**, which is exactly why this is
structural rather than another pair of names — the move is to *add* them.
`.cargo/config.toml`'s `[build] rustflags = ["--cap-lints=allow"]` caps every
lint level rustc-wide, silently defanging `cargo clippy --workspace -- -D
warnings` on the very run that adds it, and its `[target.*.runner]` can
replace the test harness outright; `.config/nextest.toml` is the same shape
for the test job. Rule 2 does not reach either — they sit one level below the
root, not as immediate children. The rule is top-level only, so a `.vscode/`
nested under `crates/` is not covered.

**4. `AUTO_MERGE_PROTECTED_BASENAMES` — filenames anywhere in the tree.**
Currently just `Cargo.toml`. A member crate's copy carries
`[lints] workspace = true`, the only thing applying the root's
`[workspace.lints.clippy]` deny list to that crate; deleting those two lines
drops every clippy deny for it and CI stays green, because the
`validate-cargo-lints` hook compares the *root* file against the shared
baseline and never checks that members opt in.

These are structural on purpose. An enumerated list has to be extended every
time a config file is added, and a rail nobody remembered to enumerate is a
rail that silently becomes auto-mergeable — which is how `deny.toml`,
`.jscpd.json` and `frontend/eslint.config.js` were missed when this guard
covered only `.github/`. Being a little over-broad costs a person one click;
being under-broad costs the gate.

`skills/` is deliberately not protected: `claude-review.yml` reads
`skills/review/SKILL.md` from the `pull_request_target` **base** checkout, not
the PR head, so it is not in this class.

The merge call itself pins `sha: pr.head.sha`, so it can only ever land the
head the guard was computed against. Without it GitHub merges whatever the
head is at merge time, and a commit landing between the file listing and the
merge would go in having never been checked — around the guard rather than
through it. A moved head is then a 409, which is a no-op: nothing merges, the
branch survives, the job stays green, and the next sync re-evaluates every
gate against the new head.

A one-line epsilon in `analyze-coverage-diff.js` would turn "aggregate
coverage must not drop" into a suggestion; an advisory id in `deny.toml`
silences `deps-audit`; `unwrap_used = "allow"` in the root `Cargo.toml`
defangs clippy workspace-wide; a `"lint"` script rewritten to `true` in
`frontend/package.json` defangs eslint. All the same move. Such a PR still
reaches `ready to merge` and still gets its labels; it just waits for a person
to press the button, with the offending path logged in the job.

**The aggregate coverage gate is zero tolerance, and pinned as such.** Any
decrease fails, however small — the comparison is on raw floats, not on the
two decimals the finding prints. Coverage that moves between runs of
identical source is a determinism bug in the tests (#489 removed the last
known source of it by tracking fire-and-forget spawns), never a reason to
widen the gate. `.github/scripts/__tests__/analyze-coverage-diff.test.js`
holds that line: `sub_precision_decrease_is_still_a_regression` fails if
anyone adds an epsilon or rounds before comparing, so widening the gate means
visibly editing a test — which `AGENTS.md` forbids without human approval —
rather than quietly editing one comparison. The `CI Scripts (node --test)`
job runs those tests, plus the auto-merge guard and kill-switch cases, using
node's built-in runner (no dependency, no `package.json` outside
`frontend/`).

This is deliberately **not** something the reviewing agent does itself
anymore — `claude-review.yml`'s prompt explicitly tells Claude never to run
`gh pr merge`; it ends its job at submitting the verdict. Moving the merge
decision into the same deterministic script that already computes every
other gate means it re-fires on its own whenever anything relevant changes
rather than existing only as a one-shot action inside a single review turn
that could be skipped, time out, or simply never run again.

**Label forgery protection:** unlike a native review, `claude-approved` is
an ordinary label — anyone with triage-level (or higher) repo access can add
any label to any PR by hand via the UI or their own token, with no review
ever having happened. Treating the label's mere presence as a genuine
approval would let that forge a clean verdict and reach both `ready to
merge` and auto-merge with no review ever having happened. Before trusting
it for either, `sync-pr-labels.js` checks the PR's timeline for the most
recent event that added `claude-approved` and requires its actor to be
`github-actions[bot]` — the identity both `claude-review.yml`'s `gh pr edit
--add-label` call and this workflow's own label mutations run under. A
label added by any other account is left in place on the PR (so a human can
still see it was set) but is never trusted as a genuine approval — the PR
stays at `needs review` and is never auto-merged on its basis.

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
* [ ] Verdict recorded via the `claude-approved`/`claude-changes-requested` labels, never a native approve/request-changes review — status labels themselves are never set by hand
* [ ] Own comments never self-resolved; "solved please re-review" used instead
