-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Store the archive content index as one compressed blob per (archive, directory)
-- instead of one row per file per archive.
--
-- Before: archive_files held a row for every file in every archive - a 24-byte tuple
-- header, four BIGINTs and three TEXT columns (measured at 117 B of heap plus 92 B of
-- index entries per row), and archive_paths held a row for every distinct file path.
-- On a production instance the two tables were 11.3 GB, 99.8% of the database. Of the
-- four indexes on archive_files, the surrogate `id` primary key alone accounted for a
-- quarter of the index bytes while never being referenced by any query.
--
-- The only reader of that data is the directory listing (`archive_index::query_dir`):
-- archive search shells out to borg and never touches these tables. So the index is now
-- keyed the way it is actually read - one row per (archive, directory), holding that
-- directory's children as a single LZ4-compressed blob, chunked so that a directory with
-- millions of entries stays paginatable and no single row grows unbounded.
--
-- archive_paths now holds only *directory* paths; file names live inside the blobs.
--
-- The content index is derived data that is rebuilt lazily from borg on the next browse
-- (the same approach taken by 20260611100000_archive_index_normalize.sql), so the old
-- rows are dropped rather than migrated, and the job rows are cleared to force a rebuild.

DROP TABLE IF EXISTS archive_files;
DROP TABLE IF EXISTS archive_paths;

DELETE FROM archive_index_jobs;

-- Directory paths referenced by the content index. Only directories are stored here:
-- a file's name is held inside its parent directory's entry blob.
CREATE TABLE archive_paths (
    id      BIGSERIAL PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    path    TEXT NOT NULL,
    UNIQUE (repo_id, path)
);

-- One row per (archive, directory, chunk). `entries` is an LZ4-compressed, escaped-TSV
-- listing of the directory's children, pre-sorted into listing order so that a limited
-- listing only has to read the leading chunks.
CREATE TABLE archive_dirs (
    archive_id  BIGINT  NOT NULL REFERENCES archives(id) ON DELETE CASCADE,
    dir_path_id BIGINT  NOT NULL REFERENCES archive_paths(id),
    chunk_no    INTEGER NOT NULL,
    entries     BYTEA   NOT NULL,
    PRIMARY KEY (archive_id, dir_path_id, chunk_no)
);

-- Serves the orphaned-path GC that runs after an archive is deleted; the primary key
-- leads with archive_id and so cannot satisfy a dir_path_id-leading lookup.
CREATE INDEX idx_archive_dirs_path_id ON archive_dirs (dir_path_id);
