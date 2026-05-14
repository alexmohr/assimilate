# Archive Browsing & Extraction

Archives are point-in-time snapshots created by each backup run. Assimilate lets you browse, inspect, and extract files from any archive directly in the web UI.

## Viewing Archives

Navigate to **Repos** in the sidebar, select a repository, then open the **Archives** tab. The list shows every archive stored in that repository, ordered by creation time (newest first).

Each row displays:

| Column | Description |
|--------|-------------|
| Name | Archive name (timestamp-based, see [Archive Naming](#archive-naming)) |
| Date | When the backup started |
| Hostname | Agent machine that created the archive |
| Duration | How long the backup took |

Click an archive name to open its detail view.

![Archives](assets/screenshots/archives.png)

## Archive Details

The detail view shows statistics reported by `borg info`:

| Stat | Description |
|------|-------------|
| Original size | Total uncompressed size of all backed-up files |
| Compressed size | Size after compression |
| Deduplicated size | Actual new data written to the repository (after deduplication across all archives) |
| File count | Number of files included |
| Duration | Elapsed time from start to end |
| Start / End | Timestamps for the backup window |

The deduplicated size is typically much smaller than the original size because borg shares identical chunks across archives. This is the number that matters for storage capacity planning.

## Browsing Archive Contents

From the archive detail view, click **Browse** to open the file tree browser.

The browser starts at the repository root (`/`). Each entry shows:

- **Type** — file (`-`) or directory (`d`)
- **Path** — full path within the archive
- **Size** — file size in bytes
- **Modified** — last-modified timestamp
- **Mode** — Unix permission bits (e.g. `rwxr-xr-x`)

Click a directory to navigate into it. Use the breadcrumb path at the top to jump back up the tree. The browser loads up to 100 entries per directory by default; very large directories may be truncated.

## Extracting Files

To download a file from an archive:

1. Browse to the file in the archive contents view.
2. Click the **Download** icon next to the file.
3. The server streams the file directly from borg and your browser saves it with the original filename.

The download uses the correct `Content-Type` for common file types (text, images, JSON, etc.) and falls back to `application/octet-stream` for unknown extensions.

!!! warning "Large file extractions"
    Extraction streams data live from the borg repository over SSH. Downloading very large files (multi-GB) will hold an SSH connection open for the duration of the transfer. The server enforces a 5-minute timeout — extractions that exceed this limit are cancelled. For large restores, consider running `borg extract` directly on the agent machine or repository host.

Only users with the **extract** permission on the repository can download files. Users with view-only access can browse archive contents but cannot download.

## Archive Naming

Borg names archives using a timestamp prefix by default. Assimilate passes the archive name to borg at backup time using the format:

```text
{hostname}-{schedule_type}-{YYYY-MM-DDTHH:MM:SS}
```

For example: `webserver-backup-2024-03-15T02:00:01`

The hostname comes from the agent machine. The schedule type is `backup`, `check`, or `verify`. You cannot rename archives after they are created — borg does not support in-place rename.

To use a custom prefix, configure the archive prefix in the repository's schedule settings. See [Scheduling](scheduling.md) for details.

## Pruning Archives

Old archives are removed automatically after each successful backup run according to the retention policy configured on the schedule. The policy controls how many daily, weekly, monthly, and yearly archives to keep.

Retention settings are per-schedule. See [Scheduling](scheduling.md) for how to configure `keep_daily`, `keep_weekly`, `keep_monthly`, and `keep_yearly`.

Manual pruning through the UI is not available. To prune outside of the normal schedule, run `borg prune` directly on the repository host or trigger a backup run (which includes pruning) from the [Repositories](repositories.md) page.

## Archive Integrity

Borg uses content-addressed, deduplicated chunk storage. Every chunk is identified by a cryptographic hash (BLAKE2b or SHA-256 depending on the encryption mode). This means:

- **Deduplication is automatic** — identical data across archives is stored once.
- **Corruption is detectable** — borg verifies chunk hashes on read. A corrupted chunk causes an error rather than silently returning bad data.

To actively verify repository integrity, run a **Check** schedule (see [Scheduling](scheduling.md)). A check reads all chunks and verifies their hashes without extracting files.

If the repository is corrupted beyond what borg can repair, the affected archives may become unreadable. Assimilate surfaces borg error output in the backup report. For recovery options, refer to the [BorgBackup documentation](https://borgbackup.readthedocs.io/en/stable/usage/check.html).

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
