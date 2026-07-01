<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# API Reference & WebSocket Protocol

## Interactive API Explorer

The server ships with a built-in interactive API explorer and a downloadable OpenAPI specification:

- **Scalar UI**: `http://<server>/api/docs` — a modern API explorer powered by [Scalar](https://scalar.com/)
- **OpenAPI spec**: `http://<server>/api/openapi.json` — download for use with Postman, Insomnia, code generators, etc.

The Scalar UI lets you browse every endpoint, inspect request/response schemas, and execute requests directly from the browser. The OpenAPI document is generated with [utoipa](https://github.com/juhaku/utoipa) and always reflects the routes compiled into the running server.

!!! note
    This page serves the built MkDocs documentation site (`/docs`), which is a different route from the Scalar API explorer (`/api/docs`).

## Authentication

All API endpoints (except `/api/health` and `/api/auth/login`) require authentication.

### Bearer Token

Include an API token in the `Authorization` header:

```http
Authorization: Bearer <token>
```

Tokens are created and managed via the `/api/tokens` endpoints or the web UI.

### Session Cookie

Browser-based access uses a session cookie set after a successful login. The web UI handles this automatically. Cookies are `HttpOnly; SameSite=Lax` and last 24 hours (7 days when *remember me* is selected). Set `ASSIMILATE_SECURE_COOKIES=true` behind TLS to add the `Secure` flag.

### Login

```http
POST /api/auth/login
Content-Type: application/json

{"username": "admin", "password": "secret", "remember_me": false}
```

A successful response sets a session cookie and returns the authenticated user object.

## REST API Overview

| Property | Value |
|----------|-------|
| Base URL | `/api/` |
| Versioning | None (single version) |
| Request format | `application/json` |
| Response format | `application/json` |
| Error format | `{"error": "human-readable message"}` |

All timestamps are ISO 8601 strings in UTC. Numeric IDs are integers; **agents are addressed by hostname**, not by a numeric ID.

## API Endpoints Summary

For full request/response schemas, use the [interactive explorer](#interactive-api-explorer). Path parameters below use `{name}` placeholders matching the OpenAPI document.

### Auth

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/auth/login` | Authenticate and start a session |
| `POST` | `/api/auth/logout` | End the current session |
| `POST` | `/api/auth/refresh` | Refresh the current session cookie |
| `GET` | `/api/auth/me` | Return the currently authenticated user |
| `POST` | `/api/auth/change-password` | Change the current user's password |
| `GET` / `PUT` | `/api/auth/preferences` | Get or update the current user's UI preferences |

### Users

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/users` | List all users |
| `POST` | `/api/users` | Create a user |
| `PUT` | `/api/users/{id}/role` | Change a user's built-in role |
| `PUT` | `/api/users/{id}/password` | Reset a user's password |
| `DELETE` | `/api/users/{id}` | Delete a user |

### API Tokens

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tokens` | List all API tokens |
| `POST` | `/api/tokens` | Create a new API token (plaintext returned once) |
| `DELETE` | `/api/tokens/{id}` | Revoke a token |

### Agents

Agents are keyed by **hostname**.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/agents` | List all registered agents |
| `POST` | `/api/agents` | Register a new agent |
| `GET` / `PUT` / `DELETE` | `/api/agents/{hostname}` | Get, update, or delete an agent |
| `POST` | `/api/agents/{hostname}/regenerate-token` | Issue a new agent token |
| `POST` | `/api/agents/{hostname}/restart` | Send a restart command to the agent |
| `POST` | `/api/agents/{hostname}/deploy` | Push the agent binary and install a systemd unit over SSH |
| `POST` | `/api/agents/{hostname}/service-unit` | Generate the systemd unit file for the agent |
| `GET` / `POST` | `/api/agents/{hostname}/hostname-patterns` | List or add hostname alias patterns |
| `DELETE` | `/api/agents/{hostname}/hostname-patterns/{pattern_id}` | Delete a hostname alias pattern |
| `POST` | `/api/agents/{hostname}/merge-from/{source_id}` | Merge an imported placeholder agent into this one |
| `PUT` | `/api/agents/{hostname}/hide` / `/unhide` | Hide or reveal an imported placeholder agent |
| `POST` | `/api/agents/{hostname}/delete-archives` | Delete selected archives owned by the agent |
| `GET` | `/api/agents/{hostname}/repos` | List repositories assigned to the agent |
| `GET` | `/api/agents/{hostname}/reports` | List backup reports for the agent |
| `GET` | `/api/agents/{hostname}/tunnel` | Get the agent's SSH tunnel configuration |
| `GET` / `PUT` | `/api/agents/{hostname}/tags` | Get or set the agent's tags |

See [Agent Management](agents.md) for setup and configuration details.

### Repositories

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/repos` | List all repositories |
| `POST` | `/api/repos` | Create a repository record |
| `GET` | `/api/repos/stats` | List repositories with storage statistics |
| `POST` | `/api/repos/init` | Initialize a new borg repository on the agent |
| `GET` / `PUT` / `DELETE` | `/api/repos/{repo_id}` | Get, update, or delete a repository record |
| `POST` | `/api/repos/{repo_id}/destroy` | Run `borg delete` to destroy the remote repository |
| `POST` | `/api/repos/{repo_id}/sync` | Sync archive metadata from the repository |
| `POST` | `/api/repos/{repo_id}/rescan` | Rebuild the searchable archive file index |
| `POST` | `/api/repos/{repo_id}/reset-and-sync` | Clear cached metadata and re-sync |
| `POST` | `/api/repos/{repo_id}/reset-import` | Reset an imported repository to unmatched state |
| `POST` | `/api/repos/{repo_id}/break-lock` | Run `borg break-lock` |
| `POST` | `/api/repos/{repo_id}/confirm-relocation` | Acknowledge a repository relocation warning |
| `POST` | `/api/repos/{repo_id}/exec` | Execute an allow-listed borg maintenance command |
| `POST` | `/api/repos/{repo_id}/dry-run` | Preview which files a schedule would back up |
| `GET` | `/api/repos/{repo_id}/passphrase` | Retrieve the stored passphrase (admin only) |
| `POST` | `/api/repos/{repo_id}/key/export` | Export the borg repository key |
| `POST` | `/api/repos/{repo_id}/key/import` | Import a borg repository key |
| `POST` | `/api/repos/{repo_id}/key/change-passphrase` | Change the repository passphrase |
| `POST` | `/api/repos/{repo_id}/migrate-encryption` | Migrate the repository to a new encryption mode |
| `POST` | `/api/repos/{repo_id}/ssh-host-key/scan` | Scan the repository host's SSH host key |
| `POST` | `/api/repos/{repo_id}/ssh-host-key` | Accept and pin the repository host's SSH host key |
| `GET` | `/api/repos/{repo_id}/schedules` | List schedules for the repository |
| `GET` / `PUT` | `/api/repos/{repo_id}/tags` | Get or set repository tags |
| `GET` / `PUT` | `/api/repos/{id}/quota` | Get or set the repository storage quota |
| `GET` | `/api/repos/{repo_id}/permissions` | List per-repo user permissions |
| `PUT` | `/api/repos/{repo_id}/permissions/{user_id}` | Grant or update a user's repo permission |

See [Repositories](repositories.md) for full details.

### Archives

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/repos/{repo_id}/archives` | List archives in a repository |
| `GET` | `/api/repos/{repo_id}/archives/diff` | Diff two archives |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}` | Get details for a specific archive |
| `DELETE` | `/api/repos/{repo_id}/archives/{archive_name}` | Delete an archive |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}/contents` | Browse the archive file tree |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}/index-status` | Check the file-index build status |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}/extract` | Stream a single file from the archive |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}/export` | Export the whole archive as a tarball |
| `POST` | `/api/repos/{repo_id}/archives/{archive_name}/download` | Download selected paths as an archive |
| `POST` | `/api/repos/{repo_id}/archives/{archive_name}/restore` | Restore selected paths to a target on the agent |
| `GET` | `/api/repos/{repo_id}/archives/{archive_name}/search` | Search files within a single archive |
| `GET` | `/api/repos/{repo_id}/search` | Search files across all archives in a repo |
| `GET` / `POST` | `/api/repos/{repo_id}/archives/{archive_name}/tags` | List or add archive tags |
| `DELETE` | `/api/repos/{repo_id}/archives/{archive_name}/tags/{tag}` | Remove an archive tag |

See [Archives](archives.md) and [Restoring Files](restore.md) for browsing and restore workflows.

### Schedules

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/schedules` | List all schedules |
| `POST` | `/api/schedules` | Create a schedule |
| `GET` / `PUT` / `DELETE` | `/api/schedules/{id}` | Get, update, or delete a schedule |
| `POST` | `/api/schedules/{id}/run` | Trigger an immediate run for this schedule |
| `POST` | `/api/schedules/{id}/cancel` | Cancel a running backup for this schedule |
| `GET` | `/api/schedules/{id}/reports` | List reports produced by this schedule |
| `GET` | `/api/schedules/{id}/sources` | List the schedule's backup sources |
| `GET` | `/api/schedules/{id}/targets` | List the schedule's target repositories |

See [Scheduling](scheduling.md) for cron expression syntax and examples.

### Global Excludes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/excludes` | List all global exclude patterns |
| `PUT` | `/api/excludes` | Replace the full set of global exclude patterns |

See [Global Excludes](excludes.md) for pattern syntax and usage.

### Tags

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tags` | List all tags |
| `POST` | `/api/tags` | Create a tag |
| `DELETE` | `/api/tags/{id}` | Delete a tag |
| `GET` | `/api/agent-tags` | List agent→tag associations |
| `GET` | `/api/repo-tags` | List repo→tag associations |

### Storage Quotas

| Method | Path | Description |
|--------|------|-------------|
| `GET` / `PUT` | `/api/repos/{id}/quota` | Get or set a repository's quota thresholds |

See [Storage Quotas](quotas.md) for details.

### SSH Tunnels

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tunnels` | List all configured SSH tunnels |
| `POST` | `/api/tunnels` | Create a new SSH tunnel |
| `GET` / `PUT` / `DELETE` | `/api/tunnels/{id}` | Get, update, or delete a tunnel |
| `POST` | `/api/tunnels/{id}/enable` / `/disable` | Enable or disable a tunnel |
| `POST` | `/api/tunnels/{id}/reconnect` | Force a tunnel to reconnect |

See [SSH Tunnels](ssh-tunnels.md) for configuration details.

### Notifications

| Method | Path | Description |
|--------|------|-------------|
| `GET` / `POST` | `/api/notifications/channels` | List or create notification channels |
| `PUT` / `DELETE` | `/api/notifications/channels/{id}` | Update or delete a channel |
| `POST` | `/api/notifications/channels/{id}/test` | Send a test notification |
| `GET` / `POST` | `/api/notifications/rules` | List or create notification rules |
| `DELETE` | `/api/notifications/rules/{id}` | Delete a rule |
| `GET` | `/api/notifications/deliveries` | List recent notification deliveries |
| `POST` | `/api/notifications/validate-smtp` | Validate SMTP settings |
| `GET` / `PUT` | `/api/notifications/push/vapid-key` | Get or set the Web Push VAPID keys |
| `POST` | `/api/notifications/push/subscribe` / `/unsubscribe` | Manage this browser's Web Push subscription |
| `GET` | `/api/notifications/push/subscriptions` | List Web Push subscriptions |

See [Notifications](notifications.md) for channel and rule configuration.

### Access Control (RBAC)

| Method | Path | Description |
|--------|------|-------------|
| `GET` / `POST` | `/api/roles` | List or create roles |
| `PUT` / `DELETE` | `/api/roles/{id}` | Update or delete a custom role |
| `GET` / `POST` | `/api/groups` | List or create groups |
| `PUT` / `DELETE` | `/api/groups/{id}` | Update or delete a group |
| `GET` / `PUT` | `/api/groups/{id}/members` | List or set group members |
| `GET` / `PUT` | `/api/users/{id}/roles` | List or set a user's roles |
| `GET` | `/api/users/{id}/groups` | List a user's groups |
| `GET` | `/api/users/{id}/permissions` | List a user's directly assigned repo permissions |
| `GET` | `/api/users/{id}/effective-permissions` | List a user's effective (resolved) permissions |

See [Access Control](access-control.md) for roles, groups, and permissions management.

### Activity & Statistics

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/stats/summary` | Top-level dashboard summary |
| `GET` | `/api/stats/dashboard-overview` | Dashboard overview (coverage, success rates, findings) |
| `GET` | `/api/stats/health` | Aggregate backup health |
| `GET` | `/api/stats/activity` | Backup activity entries |
| `GET` | `/api/stats/system-events` | System event log |
| `GET` | `/api/stats/trends` | Backup success/failure trends |
| `GET` | `/api/stats/storage` | Storage usage per repository |
| `GET` | `/api/stats/storage-breakdown` | Storage breakdown by category |
| `GET` | `/api/stats/storage-trends` | Storage growth over time |
| `GET` | `/api/stats/storage-trends/by-repo` | Storage growth per repository |
| `GET` | `/api/stats/calendar` | Calendar heatmap data |
| `GET` | `/api/stats/schedule-counts` | Counts of schedules by type/state |
| `POST` / `DELETE` | `/api/stats/findings/{finding_id}/dismiss` | Dismiss or undismiss a health finding |

See [Activity Log](activity.md) for the activity timeline UI.

### Configuration Import / Export

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/config/export` | Export server configuration as JSON |
| `POST` | `/api/config/import` | Import server configuration from JSON |

### Server Logs

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/logs` | Retrieve buffered server log entries (filterable by level) |

### SSH Helpers

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/ssh/test-connection` | Test SSH connectivity to a repository host |
| `POST` | `/api/ssh/deploy-key` | Deploy the server public key to a host |
| `POST` | `/api/ssh/list-dir` | List a directory on a remote host over SSH |
| `POST` | `/api/ssh/mkdir` | Create a directory on a remote host over SSH |

### Audit Log

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/audit-log` | Retrieve audit log entries |

See [Audit Log](audit-log.md) for details.

### System

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/system/version` | Server version and build info |
| `GET` / `PUT` | `/api/system/settings` | Get or update system settings |
| `GET` | `/api/system/ssh-public-key` | Public SSH key used for borg repository access |
| `POST` | `/api/system/ssh-regenerate-key` | Regenerate the server's SSH key pair |
| `GET` | `/api/system/database-storage` | PostgreSQL storage usage per table |
| `POST` | `/api/system/reset` | Reset the system (destructive; admin only) |

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Liveness check — returns `200 OK` when the server is up |

No authentication required for `/api/health`.

## WebSocket Protocol

Agents connect to the server over a persistent WebSocket connection. All messages are JSON-encoded using a tagged envelope:

```json
{"type": "<VariantName>", "payload": { ... }}
```

The `type` is the message variant name (PascalCase). Messages with no data (for example `Ping`, `Pong`, `RestartAgent`, `ShuttingDown`) omit the `payload` field.

### Endpoint

```text
ws://<server>/ws/agent
```

Authentication is performed via the `Hello` message immediately after connection — the hostname and token are **not** part of the URL. The agent sends `Hello { hostname, token, agent_version, ... }`; the server validates the token against the bcrypt hash stored for that hostname.

### Message Types

#### Agent → Server (`AgentToServer`)

| Type | Description |
|------|-------------|
| `Hello` | Sent immediately after connect; carries hostname, token, and agent version/capabilities |
| `Pong` | Response to a server `Ping` |
| `BackupStarted` / `BackupCompleted` / `BackupRejected` / `BackupCancelled` | Backup lifecycle events |
| `BackupLog` | Streams a log line from an in-progress backup |
| `CheckCompleted` / `VerifyCompleted` / `CanaryVerified` | Integrity operation results |
| `InitRepoCompleted` | Result of a repository initialization |
| `StatusUpdate` | Per-repo status change |
| `SearchResult` / `RestoreCompleted` / `DryRunResult` / `ExportReady` | Results of on-demand operations |
| `KeyExportResult` / `KeyImportResult` / `PassphraseChanged` / `MigrateEncryptionCompleted` | Key-management results |
| `DeleteArchivesResult` | Result of an archive deletion request |
| `OperationProgress` / `OperationFailed` | Progress and failure reporting for long operations |
| `RestartFailed` | Sent when the agent cannot honor a restart request |

#### Server → Agent (`ServerToAgent`)

| Type | Description |
|------|-------------|
| `Ping` | Heartbeat; agent responds with `Pong` |
| `ConfigUpdate` | Full agent configuration (repos, schedules, excludes) |
| `RunBackupNow` / `RunCheckNow` / `RunVerifyNow` | Trigger a backup, check, or verify for a repo |
| `CancelBackup` | Cancel a running backup |
| `InitRepo` | Initialize a new borg repository |
| `SearchArchive` / `RestoreFiles` / `DryRun` / `ExportArchive` | On-demand operations |
| `KeyExport` / `KeyImport` / `ChangePassphrase` | Key-management operations |
| `DeleteArchives` | Delete named archives |
| `RestartAgent` | Instruct the agent process to restart |
| `ShuttingDown` | Sent when the server is shutting down |

### Connection Lifecycle

1. Agent opens the WebSocket to `/ws/agent` and immediately sends a `Hello` message with its hostname and token.
2. Server validates the token, sends a `ConfigUpdate` with the agent's full configuration, and begins sending periodic `Ping` messages.
3. Agent responds to each `Ping` with `Pong`.
4. Server sends `RunBackupNow` (or `RunCheckNow` / `RunVerifyNow`) when a scheduled or manual operation is due.
5. Agent streams `BackupLog` messages during the run, then sends `BackupCompleted`.
6. Either side may close the connection; the agent reconnects automatically.

### UI WebSocket

Browsers open a separate, server-push-only WebSocket at `/ws/ui` to receive live events (`AgentConnected`, `AgentDisconnected`, `BackupStarted`, `BackupCompleted`, `CheckCompleted`, `VerifyCompleted`, `ConfigUpdated`, and more — the `ServerToUi` enum). It carries no client→server commands.

## SSH Agent WebSocket

The server relays the SSH agent protocol over a dedicated WebSocket endpoint, giving agents transparent access to the server's SSH keys without distributing private key material.

### Endpoint

```text
ws://<server>/ws/ssh-agent/{hostname}
```

The token is **not** part of the URL. The agent sends its token as the first WebSocket message; the server verifies it against the bcrypt hash stored for that hostname before opening the relay.

### Protocol

The relay is a **binary** byte-stream bridge between the agent's local Unix domain socket and the server's `SSH_AUTH_SOCK`. After the initial token message, raw SSH agent protocol bytes flow in both directions with no additional framing or JSON.

For setup instructions and Docker configuration, see [SSH Agent Forwarding](ssh-agent-forwarding.md).

## Rate Limiting & Security

The server applies two brute-force protections to the login endpoint:

- **IP rate limiting** — the `/api/auth/login` route is limited to 10 requests per 60 seconds per client; excess requests receive `429 Too Many Requests`.
- **Account lockout** — after 5 failed login attempts for a username within a 15-minute rolling window, further attempts are rejected until the window elapses.

Other API routes are not rate-limited by the server. For production deployments:

- Place the server behind a reverse proxy (nginx, Caddy, Traefik) and configure additional rate limiting there.
- Restrict access to the admin API by IP allowlist at the proxy or firewall level.
- CORS is **not** enabled by default; the API is intended to be served from the same origin as the frontend.
- All agent tokens are cryptographically random (32+ bytes). Rotate them via `/api/agents/{hostname}/regenerate-token` if compromised.

See [Security](security.md) for hardening recommendations.

## Error Codes

| HTTP Status | Meaning |
|-------------|---------|
| `200 OK` | Request succeeded |
| `201 Created` | Resource created successfully |
| `204 No Content` | Request succeeded; no body returned (e.g., DELETE) |
| `400 Bad Request` | Invalid input; body contains `{"error": "..."}` |
| `401 Unauthorized` | Missing or invalid credentials |
| `403 Forbidden` | Authenticated but not permitted to perform this action |
| `404 Not Found` | Resource does not exist |
| `409 Conflict` | Resource already exists or state conflict |
| `429 Too Many Requests` | Login rate limit or account lockout triggered |
| `500 Internal Server Error` | Unexpected server error; check server logs |

All error responses use the same envelope:

```json
{"error": "human-readable description of the problem"}
```
