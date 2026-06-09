-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE repos
    ADD COLUMN last_op_kind TEXT,
    ADD COLUMN last_op_at TIMESTAMPTZ,
    ADD COLUMN last_op_by TEXT;
