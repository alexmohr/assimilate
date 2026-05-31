# Dashboard

The Dashboard is the landing page after login. It provides a high-level overview of the backup fleet's health, storage usage, and recent activity.

![Dashboard](assets/screenshots/dashboard.png)

## System Status Banner

The top row shows six summary statistics:

| Stat | Description |
|------|-------------|
| **Online Clients** | Number of currently connected agents out of total registered hosts. A green dot indicates all agents are connected; yellow indicates some are offline. |
| **Repositories** | Total number of registered repositories |
| **Overdue** | Number of schedules that missed their expected run window. Highlighted red when > 0. |
| **Last Backup** | Relative time since the most recent completed backup across all hosts |
| **Next Backup** | Relative time until the next scheduled backup fires |
| **Total Storage** | Sum of deduplicated storage across all repositories |
| **Last Failure** | Relative time since the most recent failed backup. Click to open a detail popup showing the error message, with links to the affected repository and schedule. |
| **Last Warning** | Relative time since the most recent backup warning. Click to open a detail popup showing the warning message, with links to the affected repository and schedule. |

## 30-Day Success Rate

A donut ring showing the percentage of successful backups over the last 30 days. The ring colour changes based on the rate:

| Rate | Colour |
|------|--------|
| ≥ 90% | Green |
| 70–89% | Yellow |
| < 70% | Red |

Below the ring, a legend shows the absolute count of passed and failed runs.

## Storage Breakdown

A donut chart showing how total storage is distributed. Use the toggle buttons to group by:

| View | Groups by |
|------|-----------|
| **Repo** | Individual repositories |
| **Client** | Host machines |
| **Server** | Borg repository servers |

Each segment shows the repository/host/server name, its percentage of total storage, and absolute size.

## Repository Health

A grid of cards — one per repository-host combination. Each card shows:

- **Status dot** — green (last backup succeeded), yellow (warning), red (failed or overdue)
- **Hostname** — the agent machine
- **Repository name** — the backup target
- **Overdue badge** — shown when the schedule has missed its expected window
- **Last backup time** — relative timestamp

Use this section to spot hosts that need attention at a glance.

## Activity Timeline (14 Days)

A scatter plot showing backup activity over the last 14 days. Each dot represents one backup run:

- **X-axis** — date (day)
- **Y-axis** — time of day (0h–24h)
- **Colour** — green (success), yellow (warning), red (failed)

This visualization helps identify patterns: regular schedules appear as horizontal bands, failures cluster visibly, and gaps indicate missed runs.

## Storage Trends Chart

The trends chart shows how deduplicated repository storage has changed over time. Use it to project future storage needs and detect unexpected growth.

- **X-axis** — date (last 30 days by default)
- **Y-axis** — total deduplicated storage across all repositories (or per-repository when filtered)
- **Tooltip** — hover over a data point to see the exact size and date

Use the **Group by** toggle to view trends per repository, per host, or as a fleet-wide total. Use the **Range** selector to switch between 7-day, 30-day, and 90-day views.

<!-- screenshot: dashboard-trends -->

## Backup Calendar

The calendar view shows backup activity for the current month. Each day cell is coloured based on the worst backup result for that day:

| Colour | Meaning |
|--------|---------|
| Green | All backups succeeded |
| Yellow | At least one backup produced a warning |
| Red | At least one backup failed |
| Empty | No backups scheduled or run |

Click any day to see a list of backup runs for that date with their result, duration, and archive size. Repository names in the event list are clickable links to the repository detail page.

When clicking a failed or warning event, a popup appears showing the full error or warning message. The popup includes clickable links to the affected repository and schedule for quick navigation.

<!-- screenshot: dashboard-calendar -->

!!! tip
    Use the calendar to quickly identify days when backups failed or were skipped, especially after holidays or maintenance windows.

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
