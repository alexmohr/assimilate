-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

UPDATE schedules SET execution_mode = 'sequential' WHERE execution_mode = 'parallel';
ALTER TABLE schedules ALTER COLUMN execution_mode SET DEFAULT 'sequential';
