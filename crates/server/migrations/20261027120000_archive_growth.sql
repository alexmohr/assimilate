-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Where an archive's deduplicated size sits: per folder and per file, the
-- data no other archive in the repository holds. Worked out in the
-- background from `borg list`, which reads every file's chunks against the
-- repository's chunk index, so it is kept rather than recomputed per view.
--
-- The breakdown is a snapshot: a later archive that keeps the same data
-- shares those chunks, so the same archive read again shows less. `finished_at`
-- says when it was taken, and recomputing replaces it.

CREATE TABLE archive_growth (
    archive_id    BIGINT PRIMARY KEY REFERENCES archives(id) ON DELETE CASCADE,
    status        TEXT NOT NULL CHECK (status IN ('pending', 'computing', 'done', 'failed')),
    -- The breakdown as JSON (`shared::archive_growth::ArchiveGrowth`), set when done.
    result        TEXT,
    error_message TEXT,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at   TIMESTAMPTZ
);
