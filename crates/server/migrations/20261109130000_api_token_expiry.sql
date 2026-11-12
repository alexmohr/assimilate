-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- When an API token stops working. NULL, as every existing token has, means
-- it works until it is deleted. An expired token is rejected like an unknown
-- one; the row stays so the token list can show that it expired.
ALTER TABLE api_tokens ADD COLUMN expires_at TIMESTAMPTZ;
