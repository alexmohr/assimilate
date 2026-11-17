<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Provider Guides

Hosted borg storage needs a few settings that a plain SSH server does not: another SSH port, a directory repositories must live in, or a borg binary under another name. Choosing the provider in the [create or import dialog](repositories.md) fills these in and locks them. This page lists what each provider does and what remains for you to do.

## Prerequisites

- An account with the provider, and its login for the first SSH connection, used once to deploy the server's key.
- Admin access in Assimilate. See [Repository Hosts](repository-hosts.md#provider) for what a provider fixes on the host.

## Hetzner Storage Box

A Storage Box serves SSH on port 23 and offers no shell, so the key is installed over SFTP. Borg runs as `borg-1.4` on the box.

1. Enable **SSH support** for the box in the Hetzner console.
2. In **Repositories**, select **Import** or **Create** and choose **Hetzner Storage Box** as the **Provider**.
3. Enter the account name, for example `u123456`, in **SSH host**. The field completes to `u123456.your-storagebox.de` when you leave it. Set **SSH user** to the same account name, or to a sub-account.
4. The port is 23 and **Repo path** starts as `./backups/`. Append the repository name, or the sub-account's own path.
5. Select **+ Deploy Key**, enter the account password and select **Deploy**. SFTP is already selected.
6. Enter the passphrase and select **Create Repo** (or **Import Repo**).

!!! note
    **Test Connection** asks the box for the provider's borg (`borg-1.4` here) with `--version`. A storage box that restricts which commands may run can answer "borg not found" even though backups work; the repository path check and a real backup are the reliable test.

!!! note
    A sub-account only sees its own directory. Use the path from the Hetzner console rather than `./backups` when the sub-account is restricted to a sub-directory.

## rsync.net

rsync.net runs several borg versions side by side and selects one by the name of the binary. Assimilate uses `borg14`, sent as `--remote-path` on every command.

1. In **Repositories**, select **Import** or **Create** and choose **rsync.net** as the **Provider**.
2. Enter the account host, for example `ab123.rsync.net`, as **SSH host**, and the account name as **SSH user**. The port is 22.
3. Enter the repository path relative to the account's home directory.
4. Select **+ Deploy Key** to install the server's key, then enter the passphrase and save. **Test Connection** asks for `borg14`, the binary Assimilate uses on this provider.

## BorgBase

BorgBase gives each repository its own host name and path, and its key is the one added to the repository in the BorgBase console. Choosing the provider fixes the port and runs plain `borg`.

1. Create the repository in the BorgBase console and add the server's public key from the **System** page, with **Full access** or **Append-only access**.
2. In **Repositories**, select **Import** and choose **BorgBase** as the **Provider**.
3. Enter the repository's host name as **SSH host**, its user as **SSH user** and its path as **Repo path**.
4. Enter the passphrase and select **Import Repo**.

## Generic SSH

Any other server running borg. Nothing is filled in or locked, and every setting is entered by hand.

## Configuration Reference

| Setting | Generic SSH | Hetzner Storage Box | rsync.net | BorgBase |
|---------|-------------|---------------------|-----------|----------|
| SSH port | You choose | 23 | 22 | 22 |
| Repository path | You choose | `./backups` | You choose | You choose |
| `BORG_REMOTE_PATH` | Not set | `borg-1.4` | `borg14` | Not set |
| Key deploy | SFTP offered first | SFTP | SFTP | Shell |

## Related Pages

- [Repository Hosts](repository-hosts.md#provider)
- [Repository Management](repositories.md)
- [Append-only Repositories](append-only.md)
