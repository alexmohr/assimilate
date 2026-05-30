<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Restoring Files

Assimilate supports two restore paths: downloading files directly to your browser, or restoring files to the agent machine's filesystem. Choose based on the destination you need.

## Prerequisites

- The target repository must be accessible (agent connected or repository reachable via SSH).
- You need the **extract** permission on the repository.

## Browser Download

Use browser download to retrieve individual files or a directory tree directly to your local machine.

1. Navigate to **Repos**, select a repository, and open the **Archives** tab.
2. Click the archive you want to restore from.
3. Click **Browse** to open the file tree.
4. Navigate to the file or directory you want.
5. Click the **Download** icon next to a file to download it, or click **Export** on the archive detail view to download an entire subtree as `tar.lz4`.

The server streams data live from the borg repository. For details on the export format, see [Exporting as tar.lz4](archives.md#exporting-as-tarlz4).

<!-- screenshot: restore-browser-download -->

!!! warning "Streaming timeout"
    Large downloads stream data live over SSH. The server enforces a 5-minute per-file timeout and a 10-minute archive-export timeout. For multi-GB restores, use agent-side restore instead.

## Agent-Side Restore

Agent-side restore extracts files directly on the agent machine — no data passes through the Assimilate server or your browser. This is the right approach for large restores or when the destination is the agent's own filesystem.

### How to Trigger

1. Navigate to **Repos**, select a repository, and open the **Archives** tab.
2. Open the archive detail view.
3. Click **Restore to Agent**.
4. Configure the restore options:

| Option | Description |
|--------|-------------|
| **Archive** | Pre-selected archive to restore from |
| **Target path** | Absolute path on the agent machine to extract files into |
| **Sub-path** | Optional path prefix inside the archive to limit what is extracted (e.g. `/home/user/documents`) |
| **Strip components** | Number of leading path components to strip from extracted paths |

5. Click **Start Restore**. The server sends a `RestoreArchive` message to the connected agent.

<!-- screenshot: restore-agent-side -->

### Restore Status

While the restore runs, the detail view shows a live progress indicator. On completion the view reports:

- Exit code from `borg extract`
- Number of files extracted
- Any warnings or errors from borg output

!!! note
    The agent must be connected when you trigger a restore. If the agent is offline, the restore request is queued and delivered when the agent reconnects.

### Overwriting Existing Files

By default, `borg extract` overwrites existing files at the target path. Ensure the target path is correct before starting — there is no undo.

!!! warning "Data loss risk"
    Restoring to a non-empty directory will overwrite existing files without prompting. Set the target path to an empty staging directory if you want to inspect files before replacing production data.

## Restore Flow

```mermaid
sequenceDiagram
    participant User
    participant Server
    participant Agent
    participant Borg

    User->>Server: POST /api/archives/{id}/restore
    Server->>Agent: RestoreArchive (WebSocket)
    Agent->>Borg: borg extract <archive> <path>
    Borg-->>Agent: exit code + stats
    Agent->>Server: RestoreResult (WebSocket)
    Server->>User: restore complete notification
```

## Related Pages

- [Archive Browsing & Extraction](archives.md) — browse archive contents and download individual files
- [Scheduling & Retention](scheduling.md) — configure backup schedules and retention policies
- [Host & Agent Management](hosts.md) — manage connected agents
