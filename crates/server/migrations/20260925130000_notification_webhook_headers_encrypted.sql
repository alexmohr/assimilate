-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A webhook channel's custom HTTP headers used to live in plaintext inside
-- notification_channels.config->'headers', and every read of that JSONB handed
-- their values back - typically an `Authorization: Bearer ...` token. Each
-- header now lives in its own row: the name in plaintext (it is shown in the
-- edit dialog), the value AES-256-GCM encrypted (nonce || ciphertext) under the
-- same key as repos.passphrase_encrypted. A NULL value is a header sent empty.
--
-- Existing plaintext headers cannot be encrypted from SQL: the key is derived
-- from ASSIMILATE_SECRET_KEY and never reaches the database. The server moves
-- them over at startup, right after this migration runs - see
-- notifications::webhook_header_migration::encrypt_plaintext_webhook_headers.
CREATE TABLE notification_channel_headers (
    channel_id BIGINT NOT NULL REFERENCES notification_channels(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value_encrypted BYTEA,
    PRIMARY KEY (channel_id, name)
);

-- HTTP header names are case-insensitive, so `Authorization` and `authorization`
-- are the same header and may only be stored once per channel.
CREATE UNIQUE INDEX notification_channel_headers_name_ci
    ON notification_channel_headers (channel_id, lower(name));
