<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# opencode-harness

A deterministic Python supervisor around `opencode`'s full-auto mode, built
because a cheap coding model will happily forget to run pre-commit, write a
non-conventional commit message, or declare victory without actually fixing
CI. None of that is left to the model here: every decision about *what's
broken* and *whether the fix actually works* is plain Python and shells out
to `gh`/`git`/`cargo`/`npm`/`pre-commit` directly. opencode is only ever
asked to edit files.

## What it does, in priority order

1. **Work pull requests first.** Every poll cycle, list open PRs (oldest
   first) and find the first one that currently has something fixable:
   `ci failing`, `merge conflict`, `precheck failed` (coverage or duplicate
   code), or `changes requested`. These are this repo's own
   [`pr-status-labels.yml`](../../.github/workflows/pr-status-labels.yml)
   labels — the harness reads them, it never sets or clears them itself
   (see [`skills/review/SKILL.md`](../../skills/review/SKILL.md): *"agents
   must never add or remove the status labels themselves"*).
2. For that PR, fetch the concrete diagnostic content in Python — the
   failing CI job's log via `gh run view --log-failed`, the actual review
   comments, or the coverage-diff/duplicate-code bot's PR comment — and hand
   it to opencode as a fix prompt. opencode edits files only; it is
   explicitly told not to commit or push.
3. Run this repo's own validation gate before ever pushing:
   `uv run pre-commit run --all-files`, then — if the change touches
   Rust/frontend code — the exact commands from
   [`skills/rust/SKILL.md`](../../skills/rust/SKILL.md) and
   [`skills/frontend/SKILL.md`](../../skills/frontend/SKILL.md)'s validation
   checklists. If any step fails, its exact output is fed back to opencode
   and it retries (up to `HARNESS_MAX_LOCAL_ATTEMPTS`). Only once everything
   passes does the harness itself `git commit` (with a conventional-commits
   message it generates) and `git push`.
4. From there the repo's own automation takes back over: CI runs,
   `pr-status-labels.yml` re-syncs labels, `claude-review.yml` reviews and —
   if it's a clean approval with no `needs human review` label — merges it.
   The harness never merges anything itself. It just polls; once a PR is
   merged or closed it moves to the next one.
5. If the *same* underlying problem (same failing-check content, same
   review comments, etc.) survives `HARNESS_MAX_STUCK_CYCLES` push attempts,
   the harness stops touching that PR: it adds its own
   `opencode-harness-stuck` label (distinct from the repo's status labels)
   and posts a comment explaining what it tried. A human pushing a new
   commit, or removing that label, makes the harness pick it back up.
6. **Only once there are zero open PRs at all**, it picks the newest open
   issue, implements it on a new `opencode/issue-<n>` branch using the same
   fix-and-validate loop, and opens a PR — which flows back into step 1 on
   the next cycle.

## Requirements

* Python 3.11+, no third-party packages (stdlib only).
* `gh` (authenticated: `gh auth login`, with access to the target repo).
* `git`, configured with push access to the repo.
* `opencode`, installed and authenticated with whatever model provider you use.
* `uv` (for `pre-commit`), `cargo` + the `nightly` toolchain, `npm` — same
  toolchain this repo's `AGENTS.md`/skills already assume for local dev.
* A local clone of `alexmohr/assimilate` that this process can freely
  `checkout`/`reset --hard`/`clean -fdx` in. **Use a disposable clone, not
  your working checkout** — see Safety below.

## Configuration

All via environment variables (see `config.py` for defaults):

| Variable | Default | Meaning |
|---|---|---|
| `HARNESS_REPO` | `alexmohr/assimilate` | `owner/repo` |
| `HARNESS_REPO_DIR` | `.` | Path to the local clone the harness operates on |
| `HARNESS_BASE_BRANCH` | `main` | Base branch for rebases and new issue branches |
| `HARNESS_POLL_INTERVAL` | `180` | Seconds between cycles |
| `HARNESS_OPENCODE_MODEL` | (opencode's default) | `provider/model`, e.g. `deepseek/deepseek-v4-flash`. Also settable via `--model`, which takes priority - useful since a forgotten `export` on this env var silently falls back to opencode's default with no error. The startup log line always prints the resolved value, so check that first if a run doesn't seem to be using the model you expected |
| `HARNESS_OPENCODE_TIMEOUT` | `1800` | Seconds before an opencode invocation is killed |
| `HARNESS_MAX_LOCAL_ATTEMPTS` | `3` | Retries against local validation before giving up *this cycle* |
| `HARNESS_MAX_STUCK_CYCLES` | `3` | Cycles the same problem may survive before the PR/issue is marked stuck |
| `HARNESS_STUCK_LABEL` | `opencode-harness-stuck` | Harness-owned label, unrelated to the repo's status labels |
| `HARNESS_IGNORE_LABEL` | `opencode-harness-ignore` | Add this to a PR/issue by hand to have the harness skip it entirely |
| `HARNESS_STATE_FILE` | `tools/opencode-harness/.state.json` | Persisted attempt-tracking state (survives restarts) |
| `HARNESS_LOG_FILE` | (none, stdout only) | Optional path to also log to a file |
| `HARNESS_DRY_RUN` | `0` | `1` to log intended actions without invoking opencode or pushing |
| `HARNESS_ONCE` | `0` | `1` to run a single cycle and exit (also `--once`) |

## Running it

```bash
# one cycle, see what it would do, touch nothing
HARNESS_DRY_RUN=1 python3 tools/opencode-harness/harness.py --once

# the real thing, as a long-running process
HARNESS_REPO_DIR=/path/to/disposable/clone \
HARNESS_OPENCODE_MODEL=your-provider/cheap-model \
python3 tools/opencode-harness/harness.py
```

### systemd (recommended for unattended, restart-surviving operation)

```ini
[Unit]
Description=opencode-harness for alexmohr/assimilate

[Service]
Environment=HARNESS_REPO_DIR=/home/you/assimilate-harness-clone
Environment=HARNESS_OPENCODE_MODEL=your-provider/cheap-model
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
* **`opencode run --auto` auto-approves permissions**, which means opencode
  can run arbitrary shell commands on this machine, unattended, with
  whatever the harness process's credentials can reach. Run it under a
  dedicated, low-privilege user or inside a container/VM — not on a machine
  with access to production secrets, other repos, or your main SSH agent.
* The harness never merges a PR and never touches the repo's status labels
  or the `claude-approved`/`claude-changes-requested` verdict labels — that's
  fully owned by the existing GitHub Actions automation
  (`pr-status-labels.yml`, `claude-review.yml`). If you want different
  behavior there, change those workflows, not this tool.
* `HARNESS_IGNORE_LABEL` (`opencode-harness-ignore` by default) is your
  manual override: add it to any PR or issue you want the harness to leave
  alone entirely.
