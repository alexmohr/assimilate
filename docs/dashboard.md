# Dashboard

The Dashboard is the landing page after login. It provides a comprehensive overview of the backup fleet's health, storage usage, trends, and recent activity — all updated in real time via WebSocket.

![Dashboard](assets/screenshots/dashboard.png)

The dashboard is organized into sections that scroll vertically. Each section focuses on a different aspect of your backup infrastructure.

## System Status Banner

The top row shows eight summary statistics at a glance:

| Stat | Description |
|------|-------------|
| **Online Clients** | Number of currently connected agents out of total registered hosts (e.g. "3/5"). |
| **Repositories** | Total number of registered repositories. |
| **Overdue** | Number of schedules that missed their expected run window. Highlighted when > 0. |
| **Last Backup** | Relative time since the most recent completed backup across all hosts. |
| **Next Backup** | Relative time until the next scheduled backup fires. |
| **Total Storage** | Sum of deduplicated storage across all repositories. |
| **Last Failure** | Relative time since the most recent failed backup. Click to open a detail popup showing the error message, with links to the affected repository and schedule. |
| **Last Warning** | Relative time since the most recent backup warning. Click to open a detail popup showing the warning message, with links to the affected repository and schedule. |

## Success Rate

A donut ring showing the percentage of successful backups. The ring colour reflects overall health:

| Rate | Colour |
|------|--------|
| ≥ 90% | Green |
| 70–89% | Yellow |
| < 70% | Red |

Below the ring, a legend shows the absolute count of passed and failed runs (e.g. "125/128 OK").

**Controls:**

- **Repository filter** — dropdown to view success rate for a specific repository or all repos.
- **Time range** — toggle between 7D, 14D, 30D, and 90D windows.

## Storage Breakdown

A donut chart showing how total storage is distributed. Each segment shows the name, percentage, and absolute size.

**Toggle buttons** change the grouping:

| View | Groups by |
|------|-----------|
| **Repo** | Individual repositories |
| **Client** | Host machines |
| **Server** | Borg repository servers |

The total deduplicated size and repository count are shown above the chart.

## Repository Health (Cards)

A grid of status cards — one per repository-host combination. Each card shows:

- **Hostname** — the agent machine
- **Repository name** — the backup target
- **Overdue badge** — shown in red when the schedule has missed its expected window
- **Last backup time** — relative timestamp (e.g. "just now", "1m ago", or "Never")

Cards with "Never" for last backup indicate hosts assigned to a repository that haven't run a backup yet. Use this section to spot hosts that need attention at a glance.

## Activity Timeline

A scatter plot showing backup activity over a configurable time window. Each dot represents one backup run:

- **X-axis** — date
- **Y-axis** — time of day (0h–24h)
- **Colour** — green (success), yellow (warning), red (failed)

**Controls:**

- **Repository filter** — dropdown to filter by specific repository.
- **Time range** — toggle between 7D, 14D, 30D, and 90D.

This visualization helps identify patterns: regular schedules appear as horizontal bands, failures cluster visibly, and gaps indicate missed runs.

## Repository Health (List)

A detailed list view showing every repository-host combination with:

- **Client / Repository** — combined label (e.g. "db-server-01 / database-hourly")
- **Last backup time** — relative timestamp
- **Overdue badge** — shown when the schedule has missed its window

This supplements the card view above with a more compact, scannable format.

## Backup Stats

Summary statistics for backup activity across a selectable time range:

| Stat | Description |
|------|-------------|
| **Total** | Total number of backup runs in the selected period |
| **Success** | Percentage of successful runs |
| **Failed** | Number of failed runs |
| **Avg Duration** | Average time per backup run |

**Controls:**

- **Repository filter** — dropdown to filter by specific repository.
- **Time range** — toggle between 7D, 14D, 30D, and 90D.

## Storage Trend

A line chart showing how deduplicated repository storage has changed over time. Use it to project future storage needs and detect unexpected growth.

- **X-axis** — date
- **Y-axis** — total deduplicated storage
- **Current size** and **change over period** are displayed as summary stats.

**Controls:**

- **Repository filter** — dropdown to view trends for a specific repository or all.
- **View mode** — toggle between **Total** (single line) and **Stacked** (area chart per repository).
- **Time range** — toggle between 14D, 30D, 90D, and 1Y.

## Backup Size Trends (Deduplicated)

A chart showing how individual backup sizes (deduplicated) change over time. This helps identify backups that are growing unexpectedly or repositories with poor deduplication ratios.

**Controls:**

- **Repository filter** — dropdown to filter by specific repository.
- **Time range** — toggle between 14D, 30D, 90D, and 1Y.
- **Dedup Ratio** — shown as a summary metric.

## Backup Calendar

The calendar view shows backup activity for the current month. Each day cell is coloured based on the worst backup result for that day:

| Colour | Meaning |
|--------|---------|
| Green | All backups succeeded |
| Yellow | At least one backup produced a warning |
| Red | At least one backup failed |
| Empty | No backups scheduled or run |

Days with multiple backups show a count badge (e.g. "+5", "+22") indicating the number of backup runs on that day. Click any day to see a list of backup runs for that date with their result, duration, and archive size.

**Controls:**

- **Repository filter** — dropdown to filter by specific repository.
- **Navigation arrows** — move between months.

When clicking a failed or warning event, a popup appears showing the full error or warning message with links to the affected repository and schedule.

!!! tip
    Use the calendar to quickly identify days when backups failed or were skipped, especially after holidays or maintenance windows.

## Recent Activity

A table showing the most recent backup runs across all hosts:

| Column | Description |
|--------|-------------|
| **Client** | Hostname of the agent |
| **Repository** | Target repository name |
| **Time** | Relative timestamp |
| **Duration** | How long the backup took |

Rows are colour-coded by status: green for success, yellow for warning, red for failure.

## Next Scheduled

A list of upcoming scheduled backup runs showing:

- **Repository name** — which schedule will fire
- **Absolute time** — when the backup is scheduled
- **Relative time** — countdown (e.g. "in 47m")

This section shows the next 5 scheduled runs, giving you visibility into what's coming up.

## Real-Time Updates

The dashboard refreshes automatically via WebSocket when:

- A backup starts or completes
- An agent connects or disconnects
- The WebSocket connection is re-established after a drop

No manual page refresh is needed to see current status.

## Related Pages

- [Activity Log](activity.md) — detailed backup history and server logs
- [Scheduling & Retention](scheduling.md) — configure backup schedules
- [Host & Agent Management](hosts.md) — manage registered hosts
- [Repository Management](repositories.md) — manage backup repositories

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
