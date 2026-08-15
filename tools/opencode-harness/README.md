<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# opencode-harness

A deterministic Python supervisor around a coding-agent CLI's full-auto mode,
built because a cheap coding model will happily forget to run pre-commit,
write a non-conventional commit message, or declare victory without actually
fixing CI. None of that is left to the model here: every decision about
*what's broken* and *whether the fix actually works* is plain Python and
shells out to `gh`/`git`/`cargo`/`npm`/`pre-commit` directly. The agent CLI is
only ever asked to edit files.

The agent CLI backend itself is pluggable: `opencode` (the original, still
default backend) or `claude` (the Claude Code CLI) — see "Agent CLI" below.

## What it does, in priority order

1. **Work pull requests first.** Every poll cycle, list open PRs (oldest
   first) and find the first one that currently has something fixable:
   `ci failing`, `merge conflict`, `precheck failed` (coverage or duplicate
   code), or `changes requested`. These are this repo's own
   [`pr-status-labels.yml`](../../.github/workflows/pr-status-labels.yml)
   labels — the harness reads them, it never sets or clears them itself
   (see [`skills/review/SKILL.md`](../../skills/review/SKILL.md): *"agents
   must never add or remove the status labels themselves"*). A PR whose
   checks are still mid-flight is skipped for *this* cycle rather than
   judged early — except once some check other than `PR Merge Gate` (the
   check that's deliberately posted last, after everything else) has
   already completed with a failing conclusion: no amount of waiting on the
   remaining, still-running checks (e.g. a 20-minute e2e/nightly job) can
   un-fail that, so the harness stops waiting on that PR right away instead
   of sitting idle until literally every check finishes. If no open PR is
   actionable this cycle (all momentarily mid-CI, stuck, or otherwise
   skipped), the harness falls through to step 6 below and picks up an open
   issue instead, rather than idling until the next poll —
   `HARNESS_FALLBACK_TO_ISSUES=0` turns this off if you'd rather it only
   ever touch issues once there are zero open PRs at all.
2. CI is always discovered and reacted to by the harness's own Python, never
   by the agent CLI — it never queries CI itself, it only ever sees
   log text the harness already fetched, handed to it in a prompt. If the
   *only* thing CI is failing on is the deterministic `pre-commit` check,
   the harness fixes it directly — re-runs `pre-commit` locally (its hooks
   autofix in place: `ruff --fix`, `cargo +nightly fmt`,
   trailing-whitespace, etc.) and pushes the result — without spending an
   agent call at all. This is also why the local validation gate in step
   3 below retries itself once before ever falling back to the agent: an
   auto-fixing hook has often already fixed the disk state by the time it
   reports failure.
3. Otherwise, fetch the concrete diagnostic content in Python — the
   failing CI job's log via `gh run view --log-failed`, the actual review
   comments, or the coverage-diff/duplicate-code bot's PR comment — and hand
   it to the agent CLI as a fix prompt. The agent edits files only; it is
   explicitly told not to commit or push. Then run this repo's own
   validation gate before ever pushing: `uv run pre-commit run --all-files`,
   then — if the change touches Rust/frontend code — the exact commands
   from [`skills/rust/SKILL.md`](../../skills/rust/SKILL.md) and
   [`skills/frontend/SKILL.md`](../../skills/frontend/SKILL.md)'s validation
   checklists. `cargo test` always runs `--lib --bins`; if a Postgres is
   also reachable at `DATABASE_URL` (default
   `postgres://borg:borg_secret@localhost:5432/borg`, same as this repo's
   own CI), it additionally runs the DB-backed tests in
   `crates/server/tests/{db_queries,integration}.rs` and the server lib's
   own `#[ignore = "requires DATABASE_URL"]` tests — the same tests CI's
   "Database Integration Tests"/"Nightly Tests" jobs run, including the
   `--ignored` runs those tests need (`cargo test` skips them by default).
   Without a
   reachable DB this step is skipped and only CI can catch a regression
   there; with one, the agent gets the same fast local feedback loop CI has,
   instead of finding out several minutes later via a full push+CI
   round-trip that its fix broke an integration test. If any step fails,
   its exact output is fed back to the agent and it retries (up to
   `HARNESS_MAX_LOCAL_ATTEMPTS`). Only once everything passes does the
   harness itself `git commit` (with a conventional-commits message it
   generates) and `git push`. No review pass runs here, and none runs
   separately once CI is green either: this repo's own `claude-review.yml`
   already reviews every PR once its checks settle (see
   `pre-review-checks.js`, which waits for every other check on the commit
   to finish before the actual review runs) - a second automated review pass
   over the same commit would just be spending twice for one job.
4. From there the repo's own automation takes back over: CI runs,
   `pr-status-labels.yml` re-syncs labels and `claude-review.yml` reviews;
   once the result is `ready to merge` with a genuine approval and no
   `needs human review` label, `pr-status-labels.yml` can squash-merge it
   deterministically (see `skills/review/SKILL.md`'s "Auto-merge" section) -
   currently disabled by default pending
   [#390](https://github.com/alexmohr/assimilate/issues/390), so until then
   a human still clicks merge. The harness never merges anything itself. It
   just polls; once a PR is merged or closed it moves to the next one.
5. If the *same* underlying problem (same failing-check content, same
   review comments, etc.) survives `HARNESS_MAX_STUCK_CYCLES` push attempts,
   the harness stops touching that PR: it adds its own
   `opencode-harness-stuck` label (distinct from the repo's status labels)
   and posts a comment with the actual diagnostic content (the failing log
   tail, or the review comments) explaining what it tried. If the recurring
   problem is unresolved review feedback with CI/merge/pre-flight otherwise
   clean, it also adds `opencode-harness-question` - a signal that this
   likely needs a maintainer's decision (e.g. a policy call raised in
   review), not another code fix. A human pushing a new commit, or removing
   the label(s), makes the harness pick it back up. If a PR carries the
   repo's own `needs human review` label (see `sync-pr-labels.js`) *and*
   `changes requested` is the only outstanding problem (no CI failure,
   merge conflict, or coverage/duplicate-code precheck failure alongside
   it), the harness skips this cycle-and-retry process entirely instead of
   burning attempts on it: that label only clears when a human removes it -
   dismissing the review that triggered it does not - and whoever requested
   changes keeps that verdict in GitHub's own `reviewDecision` until they
   personally submit a new review or dismiss it, so no amount of pushed
   commits can make it refresh. The harness marks it
   `opencode-harness-stuck` + `opencode-harness-question` immediately (no
   retries burned) and leaves it alone until the label is gone. This is
   deliberately narrow: an ordinary CI/merge/coverage/duplicate-code problem
   on the same PR is still fixed as normal regardless of this label - it's
   only the review verdict itself that's a dead end while it holds. A
   pushed commit isn't the only thing that can un-stick a PR, though: the
   harness also notices whenever `HARNESS_BASE_BRANCH` itself advances (a
   merge landed on it) and clears `opencode-harness-stuck` (+
   `opencode-harness-question`, if set) from every open PR carrying it for a
   fresh look next cycle - a merge conflict or a CI failure that looked
   unresolvable can simply disappear once base moves past whatever caused
   it, which a per-PR "did *this* branch get a new commit" check alone can
   never observe. PRs stuck specifically for the `needs human review`
   reason are left alone by this - that one only ever clears when a human
   removes the label themselves, regardless of what base does. Nor is a new
   commit or a base-branch merge the only thing that can un-stick a PR on its
   *own* commit: the harness also compares which stage (CI, merge conflict,
   coverage precheck, duplicate-code precheck, review) was failing at the
   moment it gave up against which stage is failing now, and clears
   `opencode-harness-stuck` (+ `opencode-harness-question`, if set) the moment
   they differ, even with no new commit pushed - a stage other than the
   derived `PR Merge Gate` check (see "Merge gate" in
   `skills/review/SKILL.md`) settling late (a slow coverage-diff/duplicate-code
   run finally posting a result) or being re-run can otherwise fail
   differently than whatever originally got the PR parked, and that's new
   information worth a fresh look rather than something to sit on until a
   human notices.
6. **Once there are zero open PRs, or no open PR is actionable this cycle**
   (see step 1's `HARNESS_FALLBACK_TO_ISSUES`), it picks the newest open
   issue, implements it on a new `opencode/issue-<n>` branch using the same
   fix-and-validate loop, and opens a PR — which flows back into step 1 on
   the next cycle. An issue stays open (and so keeps showing up as a
   candidate) until the PR that closes it actually *merges*, not just once
   one is opened - the harness checks for an already-open PR against that
   issue's branch first and skips it if one exists, rather than trying to
   open a second one.

## Requirements

* Python 3.11+, no third-party packages (stdlib only).
* `gh` (authenticated: `gh auth login`, with access to the target repo).
* `git`, configured with push access to the repo.
* The agent CLI backend you've chosen via `HARNESS_AGENT_CLI` (see "Agent
  CLI" below) - either:
  * `opencode`, installed and authenticated with whatever model provider(s)
    you use - by default (see "Model routing" below) the harness spreads
    work across several different models, so make sure `opencode models`
    lists every model in `model_router.ROUTING_TABLE` (or override
    `HARNESS_ROUTER_MODEL`/the routing table in `model_router.py` to match
    whatever you actually have configured); or
  * `claude` (the Claude Code CLI), installed and authenticated - same idea,
    but against `model_router.CLAUDE_ROUTING_TABLE`, whose models are
    `claude-sonnet-5`/`claude-opus-5`/`claude-haiku-4-5-20251001`.
* `uv` (for `pre-commit`), `cargo` + the `nightly` toolchain, `npm` — same
  toolchain this repo's `AGENTS.md`/skills already assume for local dev.
* A local clone of `alexmohr/assimilate` that this process can freely
  `checkout`/`reset --hard`/`clean -fdx` in. **Use a disposable clone, not
  your working checkout** — see Safety below.
* Optional but strongly recommended: `cargo install sqlx-cli --locked
  --no-default-features --features postgres`, and `docker` on `PATH`. If no
  Postgres is already reachable at `DATABASE_URL`, the harness starts one
  itself (`docker run`, matching this repo's own CI service exactly - image,
  credentials, port 5432) rather than requiring you to set one up by hand,
  and leaves it running across cycles. Without `sqlx-cli`/`docker`, the
  harness can't run this repo's `#[sqlx::test]`-based integration suite
  locally at all - only CI will ever catch a regression there, several
  minutes and a full push later instead of within the agent's own local
  retry loop.

## Agent CLI

`HARNESS_AGENT_CLI` (env var only, default `opencode`) picks which
coding-agent CLI the harness drives - `opencode` or `claude` (the Claude Code
CLI). `--agent-cli` is the equivalent CLI-only override, same idea as
`--model` below. This changes two things:

* **How the agent is invoked** (`agent_runner.py`): `opencode run --dir
  <repo> --format json --auto <prompt>` vs. `claude --output-format
  stream-json --verbose --dangerously-skip-permissions -p <prompt>`. Both are
  fully unattended, auto-approving invocations - `--auto` and
  `--dangerously-skip-permissions` are each backend's own way of skipping
  every permission prompt, which is what makes running this harness
  unattended possible at all (see Safety below).
* **Which model routing table is used** (`model_router.py`): `ROUTING_TABLE`
  (opencode's own hosted-gateway model ids, e.g. `opencode-go/kimi-k2.7-code`)
  or `CLAUDE_ROUTING_TABLE` (`claude-sonnet-5`/`claude-opus-5`/
  `claude-haiku-4-5-20251001`). Both tables share the same `task_type` keys,
  so switching `HARNESS_AGENT_CLI` doesn't change what the classifier is
  asked, only which concrete models it resolves to - see "Model routing"
  below.

`HARNESS_ROUTER_MODEL`'s own default also depends on `HARNESS_AGENT_CLI` (see
the table below) - set it explicitly if you want a different classifier model
than that backend's own default cheap/fast one.

## Configuration

Most settings are environment variables (see `config.py` for defaults). The
agent model is the one exception - it's a `--model` CLI flag only, not an
env var, precisely so a forgotten `export` can't silently fall back to
pinning every task to one model with no error. The startup log line always
prints the fully-resolved config (including which agent CLI is active, and
whether a single model is pinned or every task is being routed per-task), so
check that first if a run doesn't seem to be using the model/backend you
expected.

The same model can be reachable through more than one provider prefix when
using the `opencode` backend - e.g. `deepseek/deepseek-v4-flash` routes
directly to DeepSeek's own API (needs your own DeepSeek API key configured in
opencode), while `opencode-go/deepseek-v4-flash` routes through opencode's
own hosted gateway. Using the wrong one for how you've authenticated opencode
surfaces as an opaque `UnknownError: Unexpected server error` from opencode
itself, not as a harness bug. Run `opencode models` to see which provider
prefixes are actually configured and working before pointing
`--model`/`HARNESS_ROUTER_MODEL` at one - `model_router.ROUTING_TABLE`
defaults to the `opencode-go/` prefix (this harness's own hosted-gateway
setup); change it in `model_router.py` (and `config.py`'s
`_DEFAULT_ROUTER_MODEL_BY_CLI`, and `opencode.json`) if your own opencode is
authenticated against a different provider instead. This provider-prefix
concern is specific to the `opencode` backend - `claude --model` just takes a
plain model id (e.g. `claude-sonnet-5`).

| Variable | Default | Meaning |
|---|---|---|
| `HARNESS_REPO` | `alexmohr/assimilate` | `owner/repo` |
| `HARNESS_REPO_DIR` | `.` | Path to the local clone the harness operates on |
| `HARNESS_BASE_BRANCH` | `main` | Base branch for rebases and new issue branches |
| `HARNESS_POLL_INTERVAL` | `180` | Seconds between cycles |
| `HARNESS_AGENT_CLI` | `opencode` | `opencode` or `claude` - which coding-agent CLI to drive, see "Agent CLI" above |
| `HARNESS_ROUTER_MODEL` | `opencode-go/deepseek-v4-flash` for `opencode`, `claude-haiku-4-5-20251001` for `claude` | Cheap/fast model used to classify each task before picking the model that actually does the work - see "Model routing" below |
| `HARNESS_ROUTER_TIMEOUT` | `120` | Seconds before the classifier call itself is killed - it only has to answer a question, not edit anything, so this is far shorter than `HARNESS_OPENCODE_TIMEOUT` |
| `HARNESS_OPENCODE_TIMEOUT` | `14400` (4h) | Seconds before an agent invocation is killed (the env var name predates `HARNESS_AGENT_CLI` but applies to either backend). Killing the whole process group, not just the agent process itself, so nothing it spawned (e.g. a `pre-commit`/`cargo` call from its bash tool) is left running orphaned |
| `HARNESS_MAX_LOCAL_ATTEMPTS` | `3` | Consecutive *identical* local validation failures before giving up *this cycle* - an attempt whose failure differs from the last one counts as progress and doesn't count against this (up to a hard cap of 3x this value regardless), so a chain of distinct, real bugs (fix one, reveal the next) gets a fair shot instead of exhausting the budget on genuine progress |
| `HARNESS_MAX_STUCK_CYCLES` | `3` | Cycles the same problem may survive before the PR/issue is marked stuck |
| `HARNESS_STUCK_LABEL` | `opencode-harness-stuck` | Harness-owned label, unrelated to the repo's status labels |
| `HARNESS_QUESTION_LABEL` | `opencode-harness-question` | Added alongside the stuck label when the recurring blocker looks like it needs a maintainer's decision rather than another fix attempt |
| `HARNESS_IGNORE_LABEL` | `opencode-harness-ignore` | Add this to a PR/issue by hand to have the harness skip it entirely |
| `HARNESS_STATE_FILE` | `tools/opencode-harness/.state.json` | Persisted attempt-tracking state (survives restarts) |
| `HARNESS_LOG_FILE` | (none, stdout only) | Optional path to also log to a file |
| `HARNESS_DRY_RUN` | `0` | `1` to log intended actions without invoking the agent CLI or pushing |
| `HARNESS_ONCE` | `0` | `1` to run a single cycle and exit (also `--once`) |
| `HARNESS_MAX_SOLVED` | (unlimited) | Stop after successfully solving N problems - a PR fix pushed, or an issue implemented into a new PR (also `--max-solved N`). A cycle that finds nothing actionable doesn't count against this |
| `HARNESS_FALLBACK_TO_ISSUES` | `1` | `0` to only ever pick up an issue once there are zero open PRs at all, instead of also falling back to issues whenever every open PR is momentarily unactionable (e.g. all mid-CI) - see step 1 above |

`--pr [N ...]` and `--issue N ...` are CLI-only, like `--model`/`--agent-cli` - point the
harness at specific PR(s)/issue(s) instead of letting it auto-select.
Mutually exclusive with each other. Each accepts one or more numbers
(`--pr 12 34`, `--issue 5 6`); if more than one is given, the harness
round-robins through the list, working exactly one target per poll cycle
(still respects `--once`/`--max-solved`/stuck tracking, tracked
independently per number). Bare `--pr` with no numbers targets *every*
currently open PR, re-resolved fresh each cycle (so a PR opened or merged
mid-run is picked up/dropped automatically) - `--issue` has no equivalent
"all open issues" mode, since that's just the normal auto-select behavior
once there are no open PRs left. `--pr N [N ...]` keeps the normal
fix-and-validate loop but always targets the given PR(s) instead of
auto-selecting; `--issue N [N ...]` implements each given issue and opens a
PR for it, ignoring the "newest open issue" auto-pick entirely. The first
time the harness sees a given PR number under `--pr` (explicit list or the
bare "all open PRs" form) each run, it clears any prior stuck state/labels
on it once, the same one-time override a lone `--pr N` has always done -
a human pointing the harness at a PR is choosing to retry it now, not
forever on every cycle it comes back up in the rotation.

## Model routing

A single fixed model for every job this harness does - a mechanical CI fix,
a large refactor, a security-sensitive review, a one-line boilerplate change -
is never the right tradeoff between cost and capability. By default (no
`--model` flag) the harness classifies each task before doing any real work:
it hands a short description of the task to the cheap/fast `HARNESS_ROUTER_MODEL`
and asks it to name which kind of task it is from the table below - it never
picks a model itself (see `model_router.py`). One classification per fix
attempt - a PR's merge-conflict resolution, its post-conflict validation
retry, and its main CI/review fix all share the same routed model; a task
shouldn't switch models mid-retry.

The model for the classified task is then resolved deterministically, from
whichever routing table matches `HARNESS_AGENT_CLI` (`model_router.ROUTING_TABLE`
for `opencode`, `model_router.CLAUDE_ROUTING_TABLE` for `claude` - same
`task_type` keys and descriptions in both, only the underlying models
differ). A `complexity: "low"` verdict always downgrades to that backend's
own cheap background model (`deepseek-v4-flash` for opencode,
`claude-haiku-4-5-20251001` for claude) regardless of task_type - a simple
instance of an otherwise expensive task (e.g. a small, single-file "Repo
exploration" question) doesn't need that row's model just because the
*category* usually does. Otherwise: the row's own model, *except* "Simple
boilerplate code" - the one row whose alternative is specifically that cheap
background model - which routes there instead regardless of complexity,
since that model is cheap enough to prefer outright for that kind of task.
Every other alternative below is purely informational (kept for context on
why that model was chosen); the router itself never picks between a row's
model and a non-cheap-background alternative.

**opencode** (`model_router.ROUTING_TABLE`):

| Task | Model used | Alternative (informational only) | Notes |
|---|---|---|---|
| Fix failing PRs / CI failures | `opencode-go/kimi-k2.7-code` | `opencode-go/glm-5.2` | Kimi is a good default for code repair. Use GLM when the failure requires deeper architecture reasoning. |
| Implement new features | `opencode-go/kimi-k2.7-code` | `opencode-go/glm-5.2` | Kimi for most coding; GLM for large cross-module features. |
| Large refactors | `opencode-go/glm-5.2` | `opencode-go/kimi-k2.7-code` | Better when many files and dependencies are involved. |
| Code review | `opencode-go/glm-5.2` | `opencode-go/kimi-k2.7-code` | GLM as reviewer, Kimi as implementer. |
| Debug mysterious bugs | `opencode-go/glm-5.2` | `opencode-go/kimi-k2.7-code` | Use the stronger reasoning model first. |
| Write tests | `opencode-go/kimi-k2.7-code` | `opencode-go/qwen3.7-plus` | Good balance of speed and correctness. |
| Unit test fixes | `opencode-go/kimi-k2.7-code` | `opencode-go/deepseek-v4-pro` | Usually straightforward. |
| Documentation generation | `opencode-go/qwen3.7-plus` | `opencode-go/kimi-k2.7-code` | Saves your stronger models for harder tasks. |
| Simple boilerplate code | `opencode-go/deepseek-v4-flash` **(alternative used)** | `opencode-go/qwen3.7-plus` (row model) | High quota, lower importance. |
| Dependency upgrades | `opencode-go/glm-5.2` | `opencode-go/kimi-k2.7-code` | Needs awareness of ecosystem changes. |
| Security review | `opencode-go/glm-5.2` | `opencode-go/kimi-k2.7-code` | Prefer deeper reasoning. |
| Architecture design | `opencode-go/glm-5.2` | `opencode-go/grok-4.5` | Planning > raw coding speed. |
| Repo exploration / onboarding | `opencode-go/deepseek-v4-flash` | `opencode-go/glm-5.2` | Read-only search/orientation - the same job the `search` opencode.json subagent already does on deepseek-v4-flash. Cheap and fast is the right default; GLM is there for a question that genuinely needs deep reasoning, not just wide reading. |
| Small bug fixes | `opencode-go/kimi-k2.7-code` | `opencode-go/qwen3.7-plus` | Fast turnaround. |
| Mass automated PR repair bot | `opencode-go/kimi-k2.7-code` | `opencode-go/qwen3.7-plus` | Best quota/capability ratio. |
| Cheap background agent tasks | `opencode-go/deepseek-v4-flash` | `opencode-go/mimo-v2.5` | Use only for low-risk work. |

**claude** (`model_router.CLAUDE_ROUTING_TABLE`) - Opus 5 stands in for GLM's
"deeper reasoning" role, Sonnet 5 for Kimi's "good default for most coding"
role, and Haiku 4.5 for deepseek-v4-flash's "cheap/fast" role; there's no
third, mid-tier model the way opencode's table sometimes uses qwen3.7-plus:

| Task | Model used | Alternative (informational only) | Notes |
|---|---|---|---|
| Fix failing PRs / CI failures | `claude-sonnet-5` | `claude-opus-5` | Sonnet 5 is a good default for code repair. Use Opus 5 when the failure requires deeper architecture reasoning. |
| Implement new features | `claude-sonnet-5` | `claude-opus-5` | Sonnet 5 for most coding; Opus 5 for large cross-module features. |
| Large refactors | `claude-opus-5` | `claude-sonnet-5` | Better when many files and dependencies are involved. |
| Code review | `claude-opus-5` | `claude-sonnet-5` | Opus 5 as reviewer, Sonnet 5 as implementer. |
| Debug mysterious bugs | `claude-opus-5` | `claude-sonnet-5` | Use the stronger reasoning model first. |
| Write tests | `claude-sonnet-5` | `claude-haiku-4-5-20251001` | Good balance of speed and correctness. |
| Unit test fixes | `claude-sonnet-5` | `claude-haiku-4-5-20251001` | Usually straightforward. |
| Documentation generation | `claude-haiku-4-5-20251001` | `claude-sonnet-5` | Saves your stronger models for harder tasks. |
| Simple boilerplate code | `claude-haiku-4-5-20251001` **(alternative used)** | `claude-sonnet-5` (row model) | High quota, lower importance. |
| Dependency upgrades | `claude-opus-5` | `claude-sonnet-5` | Needs awareness of ecosystem changes. |
| Security review | `claude-opus-5` | `claude-sonnet-5` | Prefer deeper reasoning. |
| Architecture design | `claude-opus-5` | `claude-sonnet-5` | Planning favors Opus 5's deeper reasoning over raw coding speed. |
| Repo exploration / onboarding | `claude-haiku-4-5-20251001` | `claude-sonnet-5` | Read-only search/orientation work. Cheap and fast is the right default; escalate by hand for a question that genuinely needs deep cross-file reasoning, not just wide reading. |
| Small bug fixes | `claude-sonnet-5` | `claude-haiku-4-5-20251001` | Fast turnaround. |
| Mass automated PR repair bot | `claude-sonnet-5` | `claude-haiku-4-5-20251001` | Best quota/capability ratio. |
| Cheap background agent tasks | `claude-haiku-4-5-20251001` | `claude-sonnet-5` | Use only for low-risk work. |

The classifier answers with a strict JSON object (`task_type`, `complexity`,
`files_affected`, `reason`) - `complexity` picks between a row's model and
the cheap background model as described above; `files_affected` isn't
currently used to pick the model, it's logged alongside the decision for
diagnostics. An unrecognized `task_type`, or an unparsable/failed
classification run, falls back to `model_router.default_fallback_model(HARNESS_AGENT_CLI)`
(`opencode-go/kimi-k2.7-code` for opencode, `claude-sonnet-5` for claude) -
this harness's own job is overwhelmingly "Fix failing PRs / CI failures" and
"Mass automated PR repair bot", both of which land near there anyway.

Passing `--model` (still CLI-only, same as before this feature existed) pins
every task to that one model and skips the classifier call entirely - useful
for pinning a specific model for testing, or if the routing tables above
don't fit your own setup. Make sure the model id matches whichever backend
`--agent-cli`/`HARNESS_AGENT_CLI` is actually pointed at.

### `opencode.json` agents

The repo root also ships an `opencode.json` defining a few narrow-purpose
subagents an interactive `opencode` session (human or agent-driven) can
delegate to via the Task tool or `@agent-name`, so the primary/expensive model
doesn't spend its own turns on work a cheaper, more specialized model handles
just as well - e.g. the `search` subagent runs on `opencode-go/deepseek-v4-flash` and is
denied edit/bash/webfetch entirely, so it can only locate files/symbols and
report back. See that file for the full list (`docs-writer`, `test-writer`,
`reviewer`) and `https://opencode.ai/docs/agents/` for the config schema.
This is specific to the `opencode` backend - it has no effect (and isn't
read at all) when `HARNESS_AGENT_CLI=claude`.

## Running it

```bash
# one cycle, see what it would do, touch nothing
HARNESS_DRY_RUN=1 python3 tools/opencode-harness/harness.py --once

# the real thing, as a long-running process - per-task model routing on by
# default, see "Model routing" above
HARNESS_REPO_DIR=/path/to/disposable/clone \
python3 tools/opencode-harness/harness.py

# pin every task to one model instead of routing per-task
python3 tools/opencode-harness/harness.py --model opencode-go/deepseek-v4-flash

# use the Claude Code CLI instead of opencode (also settable via
# HARNESS_AGENT_CLI=claude)
python3 tools/opencode-harness/harness.py --agent-cli claude

# supervised: stop after 5 solved problems instead of running forever
python3 tools/opencode-harness/harness.py --max-solved 5

# targeted: only work on a specific PR or issue instead of auto-selecting
python3 tools/opencode-harness/harness.py --pr 334
python3 tools/opencode-harness/harness.py --issue 231

# targeted: round-robin across several PRs/issues, one per poll cycle
python3 tools/opencode-harness/harness.py --pr 334 335 340
python3 tools/opencode-harness/harness.py --issue 231 232

# targeted: every currently open PR, re-resolved each cycle
python3 tools/opencode-harness/harness.py --pr

# pin every task to one model instead of routing per-task, combined with
# any of the modes above
python3 tools/opencode-harness/harness.py --model opencode-go/deepseek-v4-flash --pr 334

# claude backend, pinned model, targeted PR
python3 tools/opencode-harness/harness.py --agent-cli claude --model claude-sonnet-5 --pr 334
```

### systemd (recommended for unattended, restart-surviving operation)

```ini
[Unit]
Description=opencode-harness for alexmohr/assimilate

[Service]
Environment=HARNESS_REPO_DIR=/home/you/assimilate-harness-clone
ExecStart=/usr/bin/python3 /home/you/assimilate-harness-clone/tools/opencode-harness/harness.py
Restart=on-failure
RestartSec=30

[Install]
WantedBy=default.target
```

### cron (alternative: one cycle at a time)

```cron
*/3 * * * * cd /home/you/assimilate-harness-clone && HARNESS_ONCE=1 python3 tools/opencode-harness/harness.py >> harness.log 2>&1
```

## Safety notes

* **Use a disposable clone.** Every cycle does `git fetch` + `checkout -B` +
  `reset --hard` + `clean -fdx` on whatever branch it's working, to
  guarantee a clean starting point even after a crash. That will destroy any
  uncommitted work sitting in that checkout. Don't point `HARNESS_REPO_DIR`
  at a clone you use for anything else.
* **The agent CLI runs fully unattended, whichever backend you pick** -
  `opencode run --auto` and Claude Code's `--dangerously-skip-permissions`
  both auto-approve every permission prompt, which means the agent can run
  arbitrary shell commands on this machine, unattended, with whatever the
  harness process's credentials can reach. Run it under a dedicated,
  low-privilege user or inside a container/VM — not on a machine with access
  to production secrets, other repos, or your main SSH agent.
* The harness never merges a PR and never touches the repo's status labels
  or the `claude-approved`/`claude-changes-requested` verdict labels — that's
  fully owned by the existing GitHub Actions automation
  (`pr-status-labels.yml`, `claude-review.yml`). If you want different
  behavior there, change those workflows, not this tool.
* `HARNESS_IGNORE_LABEL` (`opencode-harness-ignore` by default) is your
  manual override: add it to any PR or issue you want the harness to leave
  alone entirely.
* If Docker is on `PATH` and no Postgres is already reachable, the harness
  will start one itself (a container named `opencode-harness-postgres`,
  bound to host port 5432) to run the DB-backed test suite locally - see
  Requirements above. It reuses/restarts that same container across cycles
  rather than tearing it down, and never touches any other container.
