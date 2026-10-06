-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Whether a repository is held until an admin sets its passphrase.
--
-- A config import creates repositories without their passphrases (they are
-- never exported) and marks them `importing` so the scheduler and "Sync now"
-- leave them alone. `importing` alone also reads as "a sync was interrupted",
-- which startup resumes - with the placeholder passphrase, so it always failed
-- and left an import error behind. This flag tells the two apart: startup does
-- not resume a held repository, and setting the passphrase releases exactly
-- this hold rather than whatever sync owns `importing` at the time.
ALTER TABLE repo_import_state
    ADD COLUMN awaiting_passphrase BOOLEAN NOT NULL DEFAULT FALSE;
