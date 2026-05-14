<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# API Reference & WebSocket Protocol

## Interactive API Explorer

The server ships with a built-in interactive API explorer powered by [Scalar](https://scalar.com/).

- **UI**: `http://<server>/api/docs`
- **OpenAPI spec**: `http://<server>/api/openapi.json` (download for use with Postman, Insomnia, etc.)

The explorer lets you authenticate, browse all endpoints, inspect request/response schemas, and execute requests directly from the browser.

## Authentication

All API endpoints (except `/api/health` and `/api/auth/login`) require authentication.

### Bearer Token

Include an API token in the `Authorization` header:

```http
Authorization: Bearer <token>
```

Tokens are created and managed via the `/api/tokens` endpoints or the web UI.

### Session Cookie

Browser-based access uses a session cookie set after a successful login. The web UI handles this automatically.

### Login

```http
POST /api/auth/login
Content-Type: application/json

{"username": "admin", "password": "secret"}
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

All timestamps are ISO 8601 strings in UTC. IDs are integers.

## API Endpoints Summary

For full request/response schemas, use the [interactive explorer](#interactive-api-explorer).

### Auth

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/auth/login` | Authenticate and start a session |
| `POST` | `/api/auth/logout` | End the current session |
| `GET` | `/api/auth/me` | Return the currently authenticated user |

### Users

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/users` | List all users |
| `POST` | `/api/users` | Create a user |
| `GET` | `/api/users/:id` | Get a user by ID |
| `PUT` | `/api/users/:id` | Update a user |
| `DELETE` | `/api/users/:id` | Delete a user |

### API Tokens

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tokens` | List all API tokens |
| `POST` | `/api/tokens` | Create a new API token |
| `DELETE` | `/api/tokens/:id` | Revoke a token |

### Clients (Agents / Hosts)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/clients` | List all registered clients |
| `POST` | `/api/clients` | Register a new client |
| `GET` | `/api/clients/:id` | Get a client by ID |
| `PUT` | `/api/clients/:id` | Update a client |
| `DELETE` | `/api/clients/:id` | Remove a client |
| `POST` | `/api/clients/:id/regenerate-token` | Issue a new agent token |
| `POST` | `/api/clients/:id/restart` | Send a restart command to the agent |

See [Hosts](hosts.md) for setup and configuration details.

### Repositories

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/repos` | List all repositories |
| `POST` | `/api/repos` | Create a repository record |
| `GET` | `/api/repos/:id` | Get a repository by ID |
| `PUT` | `/api/repos/:id` | Update a repository |
| `DELETE` | `/api/repos/:id` | Delete a repository record |
| `POST` | `/api/repos/init` | Initialize a new borg repository on the agent |
| `GET` | `/api/repos/:id/passphrase` | Retrieve the stored passphrase (admin only) |

See [Repositories](repositories.md) for full details.

### Archives

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/repos/:id/archives` | List archives in a repository |
| `GET` | `/api/repos/:id/archives/:name` | Get details for a specific archive |
| `GET` | `/api/repos/:id/archives/:name/extract` | Download extracted archive content |

See [Archives](archives.md) for browsing and restore workflows.

### Schedules

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/schedules` | List all backup schedules |
| `POST` | `/api/schedules` | Create a schedule |
| `GET` | `/api/schedules/:id` | Get a schedule by ID |
| `PUT` | `/api/schedules/:id` | Update a schedule |
| `DELETE` | `/api/schedules/:id` | Delete a schedule |
| `POST` | `/api/schedules/:id/run` | Trigger an immediate backup for this schedule |

See [Scheduling](scheduling.md) for cron expression syntax and examples.

### Global Excludes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/excludes` | List all global exclude patterns |
| `POST` | `/api/excludes` | Create a new exclude pattern |
| `PUT` | `/api/excludes/:id` | Update an exclude pattern |
| `DELETE` | `/api/excludes/:id` | Delete an exclude pattern |

See [Global Excludes](excludes.md) for pattern syntax and usage.

### SSH Tunnels

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/tunnels` | List all configured SSH tunnels |
| `POST` | `/api/tunnels` | Create a new SSH tunnel |
| `GET` | `/api/tunnels/:id` | Get a tunnel by ID |
| `PUT` | `/api/tunnels/:id` | Update a tunnel |
| `DELETE` | `/api/tunnels/:id` | Delete a tunnel |

See [SSH Tunnels](ssh-tunnels.md) for configuration details.

### Roles

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/roles` | List all roles |
| `POST` | `/api/roles` | Create a custom role |
| `PUT` | `/api/roles/:id` | Update a role |
| `DELETE` | `/api/roles/:id` | Delete a role |

### Groups

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/groups` | List all groups |
| `POST` | `/api/groups` | Create a group |
| `GET` | `/api/groups/:id` | Get a group by ID |
| `PUT` | `/api/groups/:id` | Update a group |
| `DELETE` | `/api/groups/:id` | Delete a group |

See [Access Control](access-control.md) for roles, groups, and permissions management.

### Activity & Statistics

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/stats/activity` | List backup activity entries |
| `GET` | `/api/stats/system-events` | List system events |
| `GET` | `/api/stats/dashboard` | Dashboard statistics (success rates, storage, overdue counts) |
| `GET` | `/api/clients/:hostname/reports` | List backup reports for a specific host |

See [Activity Log](activity.md) for the activity timeline UI.

### Server Logs

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/logs` | Retrieve server log entries (filterable by level) |

### System

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/system/info` | Server version, uptime, and runtime info |
| `GET` | `/api/system/ssh-key` | Public SSH key used for borg repository access |
| `GET` | `/api/system/settings` | Get system settings |
| `PUT` | `/api/system/settings` | Update system settings |

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Liveness check — returns `200 OK` when the server is up |

No authentication required for `/api/health`.

## WebSocket Protocol

Agents connect to the server over a persistent WebSocket connection. All messages are JSON-encoded.

### Endpoint

```text
ws://<server>/ws/agent
```

Authentication is performed via the `Hello` message immediately after connection. The agent sends its hostname and token in the `Hello` payload — these are no longer part of the URL path.

### Message Types

#### Agent → Server

| Type | Description |
|------|-------------|
| `register` | Sent immediately after connection; includes agent version and capabilities |
| `pong` | Response to a server `ping`; keeps the connection alive |
| `backup_result` | Reports the outcome of a completed backup run |
| `log` | Streams a log line from an in-progress backup |

#### Server → Agent

| Type | Description |
|------|-------------|
| `ping` | Heartbeat; agent must respond with `pong` |
| `trigger_backup` | Instructs the agent to start a backup for a given repository |
| `restart` | Instructs the agent process to restart |

### Connection Lifecycle

1. Agent opens the WebSocket to `/ws/agent` and immediately sends a `Hello` message containing its hostname and token.
2. Server validates the token, acknowledges the connection, and begins sending periodic `ping` messages.
3. Agent responds to each `ping` with `pong`.
4. Server sends `trigger_backup` when a scheduled or manual backup is due.
5. Agent streams `log` messages during the run, then sends `backup_result`.
6. Either side may close the connection; the agent reconnects automatically.

## SSH Agent WebSocket

The server relays the SSH agent protocol over a dedicated WebSocket endpoint, giving agents transparent access to the server's SSH keys without distributing private key material.

### Endpoint

```text
ws://<server>/ws/ssh-agent/:hostname/:token
```

Authentication uses the same agent token as the main WebSocket.

### Protocol

The relay is a **binary** byte-stream bridge between the agent's local Unix domain socket and the server's `SSH_AUTH_SOCK`. No framing or JSON is used — raw SSH agent protocol bytes flow in both directions.

For setup instructions and Docker configuration, see [SSH Agent Forwarding](ssh-agent-forwarding.md).

## Rate Limiting & Security

The server does **not** implement built-in rate limiting. For production deployments:

- Place the server behind a reverse proxy (nginx, Caddy, Traefik) and configure rate limiting there.
- Restrict access to the admin API by IP allowlist at the proxy or firewall level.
- CORS is **not** enabled by default; the API is intended to be served from the same origin as the frontend.
- All agent tokens are cryptographically random (32+ bytes). Rotate them via `/api/clients/:id/regenerate-token` if compromised.

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
| `500 Internal Server Error` | Unexpected server error; check server logs |

All error responses use the same envelope:

```json
{"error": "human-readable description of the problem"}
```
