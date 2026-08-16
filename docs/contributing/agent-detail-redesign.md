<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Agent Detail Redesign

The design record for `frontend/src/views/AgentDetailView.vue`. It began as a
proposal; the layout described here, and all three decisions at the bottom,
are now implemented. User-facing documentation lives in
[`agents.md`](../agents.md) -- this page keeps the reasoning, so a later
change can tell which parts of the layout are load-bearing.

Two details changed between the mockup and the build, both because the data
was not there: the Repositories tile lists repository names rather than a
deduplicated total, since `/agents/{hostname}/repos` returns `RepoResponse`,
which carries no size; and the run strip draws 20 runs rather than 30, to
match the dashboard's existing run-count sampling convention.

An interactive mockup, rendered with the tokens from `frontend/src/style.css`
in both themes, lives at
[`assets/agent-detail-redesign.html`](../assets/agent-detail-redesign.html).
Open it in a browser.

## The problem

The Overview tab is the landing tab for an agent, so it is what you see when a
backup fails at 03:00. It renders as one vertical run of `.info-card` blocks,
each with the same border, the same `.info-title` and the same bottom-right
Edit button.

| Metric | Today |
| --- | ---: |
| Cards stacked on Overview | 10 |
| Of those, pure configuration | 6 |
| Buttons in the single action row | 8 |
| Backup results visible on Overview | 0 |
| `AgentDetailView.vue` | 1,320 lines |

The stack, in order:

1. Agent Information -- a nine-row `<dl>` plus the action row
2. Deploy SSH Key -- inline expander
3. Edit Agent Identity -- inline expander
4. Tags
5. Default Backup Paths
6. Default Exclude Patterns
7. Default File Change Patterns
8. Default Hook Commands
9. Hostname Aliases
10. Danger Zone

Four things go wrong:

* **No hierarchy.** Ten identical cards. Whether the agent is online is one
  badge in row three of a definition list, sitting between "Display Name" and
  "Agent Version"; the build timestamp gets equal billing.
* **The action row has no grades.** Activity Log (navigation), Edit
  (common), Regenerate Token (rare, security-relevant), Restart Agent
  (disruptive), Deploy SSH Key and Upgrade all render as `btn-sm btn-ghost`
  in one flat row of up to eight.
* **Two inline expanders reflow the page.** Edit Identity and Deploy SSH Key
  appear mid-page and push six cards down, while the rest of the app opens
  dialogs through `BaseModal`.
* **The operational content is behind tabs.** Schedules and Backups each have
  their own tab, so the landing tab shows neither the last backup nor the next
  run.

## The proposal

Four tabs -- Overview, Schedules, Backups, Settings -- under a persistent
header. All configuration moves to Settings, which empties Overview, and
Overview is rebuilt to answer the four questions the page is opened for: is it
up, did the last backup work, when does the next one run, is anything overdue.

### Persistent header

Shown on every tab: hostname, display name, status badge, and a mono meta
strip carrying version, revision, build time, registration and last-seen.
Actions are graded rather than listed:

* **Primary (accented), conditional** -- `Upgrade to vX.Y.Z` when a newer
  agent exists; `Adopt` and `Merge into...` on imported hosts.
* **Secondary** -- `Activity log`.
* **Overflow menu** -- Edit identity, Deploy SSH key, Regenerate token,
  Restart agent, then a separator and Delete agent.

Edit Identity and Deploy SSH Key become `BaseModal` dialogs instead of inline
expanders.

### Overview

1. Live backup progress (`BackupProgressCard`), when a backup is running --
   promoted from inside the info card to the top of the tab.
2. Needs-attention strip, rendered only when there is something to say
   (overdue schedule, failed last run, agent offline).
3. Four tiles: last backup, next run, repository count, 30-day success rate.
4. Schedules preview -- two rows plus "View all".
5. Recent backups preview -- three rows plus "View all".

### Schedules and Backups

Both tabs use one row grammar: a status stripe, a name, a time, stats pushed
right. Today Schedules is a card grid and Backups is a list of four-line
cards; a month of history does not fit on a screen. One line per run does.
Failure output expands in place; success rows keep their link to the archive
list.

### Settings

A left sub-nav (Identity, Backup defaults, Hostname aliases, Tags, Danger
zone) with one pane visible at a time. The four `Default ...` cards become
four sections of a single Backup defaults form: `PUT /agents/:hostname` is a
whole-object replace, so each of the four cards already had to resend the
other three's values with its own patch (see `AgentDefaultsCards.vue`). One
form means one request.

## What moves where

| Today | Proposed | Why |
| --- | --- | --- |
| Agent Information list | Header meta strip + Settings > Identity | Version, revision and build time are looked up once a quarter |
| Status badge | Header | Visible on every tab, not only Overview |
| Activity Log button | Header, still a button | Navigation, used often enough to stay visible |
| Edit / Regenerate Token / Restart / Deploy SSH Key | Header overflow menu | Rare or destructive; four ghost buttons that looked identical to Activity Log |
| Deploy / Upgrade button | Header primary, only when an upgrade exists | Conditional and actionable, so it earns the accented slot |
| Merge into... / Adopt | Header primary pair on imported hosts | An imported host has one job: get adopted or merged |
| Edit Identity inline panel | `BaseModal` from the overflow menu | The app already has the dialog; the inline panel is the odd one out |
| Deploy SSH Key inline panel | `BaseModal` from the overflow menu | Same |
| Four `Default ...` cards | Settings > Backup defaults, one card, four sections | The PUT is a whole-object replace already |
| Tags, Hostname Aliases, Danger Zone | Settings > own sections | Configuration, not status |
| Backup progress card | Top of Overview | Currently rendered below nine rows of static metadata |

Nothing is removed. Eleven things move.

## Cost

**No new endpoints.** Every number on the redesigned Overview is derivable
from data `loadTabData()` already fetches:

| Element | Source |
| --- | --- |
| Last backup, 30-day success rate, recent list | `GET /agents/{hostname}/reports` |
| Next run, schedule rows | `GET /schedules`, filtered by target hostname |
| Overdue badge, needs-attention strip | `GET /stats/health` |
| Repository count | `GET /agents/{hostname}/repos` |
| Run now, on a schedule row | `POST /schedules/{id}/run` (exists) |

**Mostly existing components.** `BaseTabs`, `BaseModal`, `BaseSegmented`,
`EntityStatusBadges`, `EntityTags`, `BackupProgressCard`, `EmptyState`,
`AgentDefaultsCards`, `AgentHostnameAliases` and `AgentDangerZone` all carry
over. New units: `AgentHeader.vue`, `AgentOverviewTab.vue`,
`AgentBackupsTab.vue`, `AgentSettingsTab.vue`.

**Shrinks the view.** `AgentDetailView.vue` is 1,320 lines carrying the tab
shell, the backup list, two inline expanders, three dialogs and the WebSocket
wiring. Splitting along the four tabs is the same move F-24 made on
`RepoDetailView`, which came out at 367 lines. See
[`ui-design-audit.md`](ui-design-audit.md).

## Three questions, settled

Each of these is drawn as options in the mockup; the sections are linked from
its table of contents.

### Settings: a fourth tab, not a header link

Both were mocked. The tab wins on router cost (`?tab=settings` on the existing
route versus a new route, a second view and a second breadcrumb), on
convention (`RepoDetailView` and today's `AgentDetailView` both drive tabs off
`route.query.tab`), and on the header (a `Settings` button would take the
action row back to three buttons plus an overflow -- the row this redesign
exists to cut). The link wins on one thing: a tab bar of three state tabs plus
one configuration tab is mixed grammar.

Not a one-way door. Settings is a self-contained component either way, so
promoting it to its own route later is a router change and nothing else.

### Imported hosts keep every tab

Nothing is dropped. An imported host has archives but no agent, so its
Schedules tab is empty -- and an empty tab that explains itself is worth more
than a tab bar whose contents shift depending on which host you opened. The
position of Backups stays fixed across the fleet, and the tab is exactly where
the emptiness ends the moment the host is adopted. `EmptyState` renders the
reason and offers Adopt and `Merge into...`.

The imported header differs in two ways: no version, revision or build
timestamp (there is no agent to report them), and the two adoption actions
take the primary slot.

### The success tile counts runs, not days

`GET /agents/{hostname}/reports` takes a `limit` and defaults to 50; the view
calls it with no parameters. Any window measured in *days* is therefore only
honest while the agent runs fewer than fifty backups in that span, and an
hourly schedule does 720 a month. That eliminates both calendar options:

| Window | Hourly agent | Daily agent | Weekly agent |
| --- | --- | --- | --- |
| Last 30 days | 99%, five failures rounded away; needs 720 reports | 93%, reads right | 75% off one missed week |
| Last 7 days | 97%, still 168 reports | 86%, every run worth 14 points | 0 or 1 runs, so no number |
| Last 30 runs | 83%, past 30h | 93%, past 30d | 97%, past 7 months |
| Last 30 runs, drawn | 5 failed, contiguous | 2 failed, weeks apart | 1 failed, most recent run |

The recommendation is the last one: headline the failure count, draw the
outcomes as a strip, and name the real time span underneath. It is
cadence-independent by construction, it fits `?limit=30` inside the endpoint's
default, and it separates one incident from a pattern -- which a percentage
cannot express. When every run is clean the tile reads "All 30 clean" over a
solid green strip.

A percentage is a fleet statistic that ended up on a single-agent page; it
belongs on the Dashboard, where every agent shares one window.

**This corrects a claim above.** "Every number is derivable from what the view
already fetches" was too strong: the 30-day rate as first drawn is not
derivable for a busy agent. Option D still needs no new endpoint, but it does
need an explicit `?limit=`.

Still open: whether 30 is the right count. Twenty covers three weeks of daily
backups and renders comfortably; fifty is the endpoint's free ceiling but
draws as hairlines. The strip should probably size itself to the tile.

## What shipped with it

The implementation carried the usual companions: a `docs/agents.md` rewrite
of the Agent Detail View section, a `.devcontainer/demo/seed-demo.sh` step
seeding three consecutive failures on `db-server-01` so the run strip's
Incident chip is covered by the demo, component tests for each new unit, and
`frontend/e2e/agent-detail.spec.ts`. See `skills/documentation/SKILL.md` and `skills/testing/SKILL.md` in the
repository root.
