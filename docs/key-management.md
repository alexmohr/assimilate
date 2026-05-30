<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Key Management

Assimilate encrypts repository passphrases at rest using AES-256-GCM. The key management page lets you export the server's encryption key for safekeeping, import a previously exported key after a migration, or change the passphrase for an individual repository.

!!! warning "Critical — back up your keys"
    If the server's encryption key is lost, all stored passphrases become unrecoverable and you will lose access to encrypted borg repositories. Export the key immediately after initial setup and store it in a secure offline location.

## Accessing Key Management

Navigate to **Settings → Key Management**. This page is available to users with the **admin** role only.

<!-- screenshot: key-management -->

## Exporting the Server Key

The server encryption key is a randomly generated AES-256 key stored on the server. Export it to create a backup.

1. Click **Export Key**.
2. Enter your admin password to confirm.
3. The key is downloaded as a JSON file: `assimilate-key-<timestamp>.json`.

Store the exported file in an encrypted password manager or offline secure storage. Do not store it in the same location as the server.

!!! warning
    The exported key file contains the master secret in plaintext (within the JSON envelope). Treat it with the same sensitivity as a private SSH key.

## Importing a Server Key

Use import to restore a previously exported key, for example after a server migration or disaster recovery.

1. Click **Import Key**.
2. Select the previously exported JSON file.
3. Enter your admin password to confirm.
4. The server replaces the active key with the imported key and re-encrypts all stored passphrases.

!!! warning "Irreversible"
    Importing a key overwrites the current key. If the import file is wrong or corrupted, passphrases encrypted with the previous key become unrecoverable. Export the current key before importing a replacement.

```mermaid
flowchart LR
    ExportedKey["Exported key file"] --> Import["Import Key"]
    Import --> ReEncrypt["Re-encrypt all passphrases"]
    ReEncrypt --> Active["New key active"]
```

## Changing a Repository Passphrase

Change the passphrase stored for a specific borg repository without affecting other repositories.

1. Navigate to **Repos** and select the repository.
2. Open the **Settings** tab.
3. Click **Change Passphrase**.
4. Enter the new passphrase and confirm it.
5. Click **Save**.

Assimilate updates the stored (encrypted) passphrase and uses it on the next backup run. The change does not re-key the borg repository itself — it only updates what Assimilate stores. To change the actual borg repository encryption key, run `borg key change-passphrase` directly on the repository host.

!!! note
    Assimilate does not verify the new passphrase against the borg repository. If you enter an incorrect passphrase, the next backup attempt will fail with a borg authentication error. The old passphrase remains recoverable via the audit log (action: `repository.passphrase_changed`) if you need to roll back.

## Related Pages

- [Repository Management](repositories.md) — add and configure borg repositories
- [Security](security.md) — authentication, session management, and encryption overview
- [Audit Log](audit-log.md) — track key management actions
