-- Drop the unique constraint on (client_id, repo_id, cron_expression) to allow
-- schedule cloning with the same cron expression.
ALTER TABLE schedules DROP CONSTRAINT IF EXISTS schedules_client_id_repo_id_cron_expression_key;
