<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Assimilate live demo

This codespace runs a complete Assimilate setup: the server, PostgreSQL, three hosts running the agent, and real borg repositories, filled with a few weeks of backup history. Everything runs inside your own codespace. Nothing you do here reaches any other machine.

## Open the UI

The demo seeds itself on first start. The terminal says **Assimilate demo ready** when it is done, which takes a few minutes. Then:

1. Open the **Ports** tab and click the globe icon next to **Assimilate (8080)**. The browser usually opens it on its own.
2. Log in as `admin` / `admin`.

If the page doesn't load yet, the seed is still running; reload it after a minute.

## Things to try

- **Dashboard:** what needs attention right now (a failed run, an overdue schedule, a repository over its quota warning), and live status of every host.
- **Hosts:** `web-server-01`, `db-server-01` and `media-store-01` are connected agents. Open one to see its schedules, its backups across repositories and its activity. `old-webserver (imported)` and `legacy-db-prod (imported)` are hosts found in imported archives that no agent matches yet.
- **Schedules:** daily, hourly and weekly schedules with retention policies. **Web server dual-target** writes every run to two repositories. **Media share nightly** depends on a NAS being online and has a skipped run waiting to catch up. Click **Run now** on any schedule and watch the run live.
- **Repositories:** quotas with warning and critical thresholds, compression and encryption settings, and the archives in each repository. `offsite-imported` came from a config import and waits for its passphrase: use **Set passphrase** with `demo-passphrase-123`.
- **Archives:** browse any archive's files, search them, download a file, or restore one to a host.
- **Notifications:** webhook and email channels with rules for failures, warnings and agent events.
- **Audit log:** every change made in the demo, including the ones you make now.
- **Users and roles:** log in as `operator1` or `viewer1` (the password is the user name) to see what each built-in role can do.

The documentation is served by the demo itself at `/docs/` on the same address.

## Costs and cleanup

The codespace runs on your own GitHub account's Codespaces quota. Personal accounts get a free monthly allowance, which this demo uses at the 2-core machine rate. A codespace stops on its own after 30 minutes without activity (the default); you can lower that under **Settings → Codespaces → Default idle timeout** on GitHub.

When you are done:

- **Stop** the codespace to keep your demo for later. Opening the demo link again resumes it with your changes.
- **Delete** it from [github.com/codespaces](https://github.com/codespaces) so it no longer uses your storage quota.

To run Assimilate for real, follow the [Getting Started guide](docs/getting-started.md).
