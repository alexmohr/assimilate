-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- An email channel's SMTP password used to live in plaintext inside
-- notification_channels.config, and every read of that JSONB handed it back.
-- It now lives here instead, AES-256-GCM encrypted (nonce || ciphertext) under
-- the same key as repos.passphrase_encrypted. NULL means no password is stored.
--
-- Existing plaintext passwords cannot be encrypted from SQL: the key is derived
-- from ASSIMILATE_SECRET_KEY and never reaches the database. The server moves
-- them over at startup, right after this migration runs - see
-- notifications::smtp_migration::encrypt_plaintext_smtp_passwords.
ALTER TABLE notification_channels ADD COLUMN smtp_password_encrypted BYTEA;
