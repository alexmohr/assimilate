-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Tracks how many times an account has been locked out consecutively
-- (reset to 0 on a successful login). Deriving the escalation tier from
-- the raw cumulative failure count doesn't work: an active lockout blocks
-- further login attempts from ever recording a new failure, so in normal
-- usage the count only grows by 1 per lockout cycle and the tier would
-- advance once every `max_account_failures` cycles instead of once per
-- cycle. This counter tracks lockout cycles directly instead.
ALTER TABLE users ADD COLUMN lockout_escalation_level INTEGER NOT NULL DEFAULT 0;
