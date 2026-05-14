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
