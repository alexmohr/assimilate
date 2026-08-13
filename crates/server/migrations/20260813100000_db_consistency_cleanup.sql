-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- 1. dismissed_dashboard_findings.user_id was INT (int4) while users.id is
--    BIGSERIAL (int8) and every other FK to users(id) uses BIGINT. Widen it
--    for consistency and to remove the latent overflow risk.
ALTER TABLE dismissed_dashboard_findings ALTER COLUMN user_id TYPE BIGINT;

-- 2. audit_log.user_id intentionally has no REFERENCES users(id): audit rows
--    must survive user deletion so the trail isn't lost, and username is
--    denormalized onto the row for exactly that reason. Document the intent
--    so it doesn't read as an oversight.
COMMENT ON COLUMN audit_log.user_id IS
    'Intentionally not a foreign key: audit rows must survive deletion of the '
    'user that produced them. See the denormalized username column, which is '
    'the source of truth once the user is gone.';

-- 3. notification_channels.updated_at, system_settings.updated_at, and
--    repo_quotas.updated_at defaulted to NOW() on insert but relied on
--    application code to bump the value on every UPDATE, with no enforcement.
--    Add a shared trigger so the column is always correct regardless of the
--    calling code path.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_notification_channels_updated_at
    BEFORE UPDATE ON notification_channels
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER trg_system_settings_updated_at
    BEFORE UPDATE ON system_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER trg_repo_quotas_updated_at
    BEFORE UPDATE ON repo_quotas
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
