<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
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

Each agent registered in Assimilate has a unique agent token.

- Tokens are generated with 32 bytes of cryptographic randomness.
- The server stores the token as a bcrypt hash.
- The agent presents its token in the WebSocket `Hello` handshake when connecting.
- The same token is used to authenticate the SSH agent forwarding WebSocket endpoint.

Agent tokens are scoped to a single agent. Revoking an agent removes its token.

## Role-Based Access Control

Assimilate uses a single RBAC layer: users are assigned one or more **roles** via the `user_roles` table. Each role bundles system-wide capabilities (e.g. create agents, manage schedules). A user's effective permissions are the union of all capabilities across every role they hold. See [Access Control](access-control.md) for the full capability matrix.

The **admin bypass** is determined by the `can_delete_repo` capability — a user with this permission (from any role they hold) passes all permission checks. The built-in `admin` role grants this capability; custom roles can also include it.

Beyond roles, per-repository permissions control what a user can do on individual repositories.

| Capability | With `can_delete_repo` | Without `can_delete_repo` |
|------------|------------------------|---------------------------|
| Manage users and roles | ✓ | ✗ |
| Create and delete agents | ✓ | depends on other RBAC capabilities |
| View and manage all repositories | ✓ | per-repo permission |
| Manage API tokens (all users) | ✓ | own tokens only |
| Configure SSH tunnels | ✓ | depends on other RBAC capabilities |
| View system information | ✓ | ✗ |
| Trigger backups | ✓ | per-repo permission |
| Browse archives | ✓ | per-repo permission |

### Per-Repository Permissions

Users with `can_view_all_repos` can see all repositories. For others, fine-grained access to individual repositories is granted by admins:

| Permission | Effect |
|------------|--------|
| `can_view` | User can see the repository and its archives |
| `can_backup` | User can trigger a backup run |
| `can_modify_schedules` | User can create and edit backup schedules |
| `can_extract` | User can browse and extract archive contents |
| `can_delete` | User can delete archives |

Permissions are managed by admins under **Settings → Access Control**.

## Session Idle Timeout

An idle timeout can be configured under **System Settings**. Sessions that have no API activity for longer than the configured duration are automatically revoked on the next request.

- **Default:** 480 minutes (8 hours).
- When idle timeout is exceeded, the session is deleted from the database and the user receives a `401 Unauthorized` response with the message "session expired due to inactivity".
- The idle window is reset on every API call that reads the session (including `/api/auth/me` and `/api/auth/refresh`).
- This is enforced in `AuthUser::from_request_parts`, which runs before every protected handler, so there is no gap between the timeout elapsing and access being denied.

## Two-Factor Authentication (TOTP)

Users can enable TOTP-based two-factor authentication from their **Profile** page. Once enabled, logging in requires two steps:

1. **Password step** — the user authenticates with their username and password. The server returns a `totp_required: true` flag and a short-lived `temp_token`.
2. **TOTP step** — the user provides the 6-digit code from their authenticator app, along with the `temp_token`. The server verifies the code and issues a real session cookie.

Security properties:

- **Temp tokens** are stored in the same `sessions` table but with `pending_totp = true`. The `AuthUser` middleware **rejects** pending sessions, so a temp token cannot be used to access any API endpoint except TOTP verification/recovery.
- **TOTP codes** use RFC 6238 (SHA-1, 30-second period, 6 digits).
- **Replay protection** — each TOTP code is rejected if the user's `totp_last_verified_at` is within the current 30-second window.
- **Recovery codes** — 10 codes are generated at enrollment using 8 bytes of cryptographic randomness, formatted as hex groups. They are hashed with **bcrypt** before storage. Each code is single-use: after verification it is removed from the stored set.
- **Secret encryption** — the TOTP secret is encrypted with AES-256-GCM using the same key derivation as repository passphrases.
- Enrollment can be cancelled; TOTP is only activated after a successful verification code confirms the user can generate valid codes.

## Session Management

Users can view and manage their active sessions from the **Sessions** tab in their Profile page.

- Each session displays creation time, expiration, last activity, and the "Remember Me" flag.
- Users can revoke any session **except** their current one.
- Session revocation checks ownership at the database layer (`DELETE FROM sessions WHERE id = $1 AND user_id = $2`), preventing a user from revoking another user's session.
- Session IDs are stored as SHA-256 hashes; the plaintext UUID is never persisted.

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
4. **Enable TOTP/2FA** — enroll two-factor authentication on the **Profile** page for all administrative accounts.
5. **Configure idle timeout** — set a session idle timeout under **System Settings** to automatically revoke inactive sessions.
6. **Review user accounts** — create per-user accounts with the least privilege needed. Avoid sharing the admin account.
7. **Rotate agent tokens** — if an agent token is ever exposed, delete the agent and recreate it to issue a new token.

See [Getting Started](getting-started.md) for the full setup walkthrough.
