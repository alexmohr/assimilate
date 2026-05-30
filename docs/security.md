<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)

See the NOTICE file(s) distributed with this work for additional
information regarding copyright ownership.

This program and the accompanying materials are made available under the
terms of the Apache License Version 2.0 which is available at
https://www.apache.org/licenses/LICENSE-2.0
-->

# Security & Authentication

This page describes how Assimilate authenticates users and agents, protects credentials at rest, and enforces access control.

## Authentication Mechanisms

Assimilate supports three authentication methods:

| Method | Transport | Use case |
|--------|-----------|----------|
| Session cookie | `Cookie: session=<id>` | Browser UI |
| API token | `Authorization: Bearer <token>` | Scripts, CI, REST API clients |
| Agent token | WebSocket `Hello` message | Agent binary connecting to server |

All three methods resolve to an authenticated identity before any handler runs. Bearer tokens and session cookies are interchangeable for API access.

## Session Security

When a user logs in via the UI, the server creates a session record in the database and sets a cookie:

```text
Set-Cookie: session=<uuid>; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400
```

Key properties:

- **HttpOnly** — the cookie is not accessible from JavaScript.
- **SameSite=Lax** — mitigates CSRF for cross-site navigations.
- **Max-Age=86400** — sessions expire after 24 hours.
- The session ID is a random UUID stored in the database. Logging out deletes the session record immediately.

## API Tokens

API tokens allow programmatic access without a browser session.

- Tokens are created through the UI or the API (`POST /api/tokens`).
- The plaintext token is shown **once** at creation time and never stored.
- The server stores only the SHA-256 hash of the token.
- Each use updates a `last_used` timestamp on the token record.
- Admins can view and delete all tokens. Regular users can only manage their own.

To authenticate with a token:

```http
Authorization: Bearer <plaintext-token>
```

## Agent Tokens

Each host registered in Assimilate has a unique agent token.

- Tokens are generated with 32 bytes of cryptographic randomness.
- The server stores the token as a bcrypt hash.
- The agent presents its token in the WebSocket `Hello` handshake when connecting.
- The same token is used to authenticate the SSH agent forwarding WebSocket endpoint.

Agent tokens are scoped to a single host. Revoking a host removes its token.

## Role-Based Access Control

Assimilate has two roles: **Admin** and **User**.

| Capability | Admin | User |
|------------|-------|------|
| Manage users and roles | ✓ | ✗ |
| Create and delete hosts | ✓ | ✗ |
| View and manage all repositories | ✓ | per-repo permission |
| Manage API tokens (all users) | ✓ | own tokens only |
| Configure SSH tunnels | ✓ | ✗ |
| View system information | ✓ | ✗ |
| Trigger backups | ✓ | per-repo permission |
| Browse archives | ✓ | per-repo permission |

### Per-Repository Permissions

Admins can grant non-admin users fine-grained access to individual repositories:

| Permission | Effect |
|------------|--------|
| `can_view` | User can see the repository and its archives |
| `can_backup` | User can trigger a backup run |
| `can_modify_schedules` | User can create and edit backup schedules |
| `can_extract` | User can browse and extract archive contents |
| `can_delete` | User can delete archives |

Permissions are managed by admins under **Settings → Users**.

## Brute-Force Protection

The login endpoint tracks failed attempts per username and client IP address.

- **5 failed attempts** within a **15-minute window** trigger a lockout.
- Subsequent login attempts return `429 Too Many Requests` until the window expires.
- All login attempts (successful and failed) are recorded in the database.

This applies to the password login flow only. API token and agent token authentication is not subject to the same rate limiting, but invalid tokens are rejected immediately.

## Passphrase Encryption

Borg repository passphrases are encrypted at rest using **AES-256-GCM**.

The encryption key is derived from `ASSIMILATE_SECRET_KEY` using HKDF-SHA256. A random 96-bit nonce is generated for each encryption operation, so identical passphrases produce different ciphertexts.

!!! warning "Security"
    If you change or lose `ASSIMILATE_SECRET_KEY`, all stored passphrases become unrecoverable. Back up this value and keep it stable for the lifetime of your deployment.

Generate a strong key before first run:

```bash
openssl rand -hex 32
```

See [Configuration](configuration.md) for how to set this environment variable.

## Password Policy

- Passwords must be **at least 8 characters**.
- Passwords are hashed with bcrypt before storage. The plaintext is never persisted.
- The API rejects change-password requests that do not meet the minimum length.

## Repository Relocation Protection

Borg verifies that a repository has not been moved or swapped since the last access. If a malicious actor replaces the remote repository with a different one, borg refuses to operate on it unless explicitly told to accept the relocation.

Assimilate enforces a **one-shot** relocation acceptance model:

- The `BORG_RELOCATED_REPO_ACCESS_IS_OK` environment variable is **only** set when an admin has explicitly changed the repository's path, SSH host, or SSH port.
- The acceptance applies to a single backup run. Once that backup succeeds, the flag is cleared.
- At all other times, borg will reject unexpected repository relocations, protecting against silent data-store substitution.

See [Repositories — Relocation Safety](repositories.md#repository-relocation-safety) for operational details.

## Sensitive Data Handling

- Repository passphrases are never logged or transmitted in plaintext. They are decrypted in memory only when passed to the borg subprocess.
- API tokens and agent tokens are stored as hashes. The plaintext is never written to the database or logs.
- Debug and trace output uses `[REDACTED]` placeholders wherever sensitive values would otherwise appear.

## First-Run Security Checklist

When you start Assimilate for the first time:

1. **Default credentials** — the server creates an `admin` account with password `admin`.
2. **Forced password change** — the UI requires you to set a new password before you can use the application.
3. **Set `ASSIMILATE_SECRET_KEY`** — generate a strong random value and keep it stable. Without it, passphrase encryption cannot function.
4. **Review user accounts** — create per-user accounts with the least privilege needed. Avoid sharing the admin account.
5. **Rotate agent tokens** — if an agent token is ever exposed, delete the host and recreate it to issue a new token.

See [Getting Started](getting-started.md) for the full setup walkthrough.
