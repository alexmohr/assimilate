-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- The BorgBase account the server manages repositories on. BorgBase gives every
-- repository its own host name, so the API token belongs to the account, not to
-- one repository host: one row at most. The token is AES-256-GCM
-- `nonce || ciphertext` under the server's encryption key, the same scheme as a
-- repository passphrase.
CREATE TABLE borgbase_account (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    api_token_encrypted BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
