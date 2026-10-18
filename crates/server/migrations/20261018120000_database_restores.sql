-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A restore can now write a database dump from an archive back into the
-- database server its source runs on, instead of extracting files onto the
-- agent's filesystem. `database` holds what the agent needs for that
-- (`shared::database::DatabaseRestore`: the source, the database, the dump's
-- path in the archive, its format and where it is restored to), read from the
-- archive's manifest when the restore was requested. A database restore has
-- no target directory, so `target_path` becomes optional; every restore is
-- exactly one of the two kinds.

ALTER TABLE restore_runs
    ADD COLUMN database JSONB,
    ALTER COLUMN target_path DROP NOT NULL,
    ADD CONSTRAINT restore_runs_files_or_database
        CHECK ((database IS NULL) <> (target_path IS NULL));
