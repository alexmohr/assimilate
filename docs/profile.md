# Profile & Preferences

The Profile page lets you manage your account settings, API tokens, and UI preferences. Access it from **Settings → Profile** in the sidebar.

The page shows your username, then its sections as collapsible rows stacked in one card: **Change password**, **API tokens**, **Two-factor auth**, **Sessions** and **Appearance**. Click a section's title to open or fold it; several can be open at once, and **Expand all** opens every one. The page opens with **Change password** open.

![Profile](assets/screenshots/profile.png)

## Change Password

The **Change password** section lets you update your login password.

1. Enter your new password (minimum 8 characters).
2. Confirm the password.
3. Click **Update Password**.

Passwords are hashed with bcrypt before storage. The server enforces a minimum length of 8 characters.

!!! note
    Admins can also reset other users' passwords from **Settings → Access Control → Users**. See [Access Control](access-control.md) for details.

## API Tokens

The **API tokens** section lets you create and manage personal API tokens for programmatic access to the REST API.

### Creating a Token

1. Click **New token**.
2. Enter a descriptive name (e.g., "CI pipeline", "monitoring script").
3. Choose when it **Expires**: **Never** (the default), after **30 days**, **90 days** or **1 year**, or **On a date** you pick, at the end of that day; **Create** waits until a day is picked. The dialog says when that is.
4. Click **Create**.
5. **Copy the token immediately** — it is shown only once and cannot be retrieved later.

The token list shows each token's expiry in its **Expires** column: amber within two weeks of the date, and red once it has passed. An expired token is refused like an unknown one, on the API and on `/metrics`; delete it and create a new one. Through the API, `POST /api/tokens` takes an optional `expires_at` timestamp, which must lie in the future.

### Using a Token

Include the token in the `Authorization` header of API requests:

```http
Authorization: Bearer <token>
```

Tokens authenticate with the same permissions as your user account.

### Managing Tokens

The token list shows:

| Column | Description |
|--------|-------------|
| Name | The label you assigned at creation |
| Created | When the token was created |
| Last used | When the token was last used to authenticate a request (or "Never") |
| Expires | When the token expires (or "Never") |

Click the delete button in its **Actions** column to revoke a token. Any integrations using that token will immediately stop working.

!!! warning
    Deleted tokens cannot be recovered. Create a new token if you need to replace a revoked one.

### Admin Token Management

Admins can view and delete all users' tokens from **Settings → Access Control → Users**. Regular users can only manage their own tokens. See [Security](security.md) for more details.

## Appearance

The **Appearance** section lets you choose the theme. Your preference is saved server-side and persists across browsers and sessions.

| Theme | Description |
|-------|-------------|
| Auto | Follows your operating system's light or dark setting |
| Light | Light background, dark text |
| Dark | Dark background, light text |

The theme applies immediately without a page reload.

## Two-Factor Authentication (TOTP)

The **Two-factor auth** section enrolls and manages TOTP-based two-factor authentication for your account. Once enabled, every login requires a one-time code from an authenticator app (e.g. Google Authenticator, Authy) in addition to your password.

!!! note
    The desktop app doesn't offer two-factor authentication. It signs in automatically with only a password, so it hides this section and the server refuses to set up TOTP (see [`ASSIMILATE_DEPLOYMENT_MODE`](configuration.md)). An account that already has TOTP still sees the section, so it can turn it off.

### Enabling 2FA

1. Click **Set up two-factor auth**. The server generates a new TOTP secret and a set of recovery codes.
2. Scan the displayed QR code with your authenticator app, or enter the manual key.
3. Enter the current 6-digit code from your authenticator app and click **Verify & Enable**.
4. **Save the recovery codes** shown on completion in a secure place. Each code can be used once to regain access if you lose your authenticator device.

TOTP is only activated after the verification code confirms you can generate valid codes, so a mistyped secret never locks you out.

### Logging in without your authenticator device

If you lose access to your authenticator app, click **Use a recovery code instead** on the two-factor step of the login page and enter one of the recovery codes you saved during enrollment in place of the 6-digit code. Each recovery code works once; to get a fresh full set, disable and re-enable 2FA (this generates 10 new codes and invalidates any unused old ones).

### Disabling 2FA

To turn 2FA off, enter your current account password in the **Two-factor auth** section and click **Disable two-factor auth**. Disabling removes the stored secret and any remaining recovery codes.

!!! warning
    Disabling 2FA reduces account security. Prefer rotating your authenticator device over disabling 2FA whenever possible.

See [Security & Authentication](security.md) for the security properties of TOTP (replay protection, recovery-code hashing, and secret encryption).

## Sessions

The **Sessions** section lists every active session for your account. Use it to review where your account is signed in and to sign out other devices.

| Column | Description |
|--------|-------------|
| Created | When the session was established |
| Last active | The most recent time the session made an API request |
| Expires | When the session expires (remember-me sessions last 7 days) |
| Type | `Remember Me` (7-day) or `Session` (24-hour) |
| Status | `Current` for the session used by this browser, otherwise `Active` |

Click the revoke button in the **Actions** column of any non-current session and confirm to revoke it immediately. Your current session cannot be revoked from here — sign out normally instead.

Sessions that are inactive longer than the configured **Session Idle Timeout** (see [Configuration](configuration.md)) are revoked automatically on the next request.

## Related Pages

- [Security & Authentication](security.md) — authentication mechanisms and token security
- [API Reference](api-reference.md) — using tokens with the REST API
- [Access Control](access-control.md) — roles, groups, and permissions

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
