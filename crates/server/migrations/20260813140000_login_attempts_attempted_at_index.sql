-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- delete_login_attempts_before() prunes by `attempted_at < $1` alone, but the
-- only existing indexes lead with (username, ip, ...) / (username, ...), so
-- the retention sweep had to scan the whole table. Add a supporting index.
CREATE INDEX idx_login_attempts_attempted_at ON login_attempts(attempted_at);
