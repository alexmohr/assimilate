-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Backfill user_roles from users.role before dropping the column.
-- Users with role='admin' get the 'admin' role (by name).
-- Users with role='user' get no default role (admin must assign via RBAC UI).
-- Users with any other role value that matches an existing RBAC role name get that role.

DO $$
DECLARE
    r record;
    role_id_val bigint;
BEGIN
    FOR r IN SELECT id, role FROM users WHERE role IS NOT NULL LOOP
        IF r.role = 'admin' THEN
            SELECT id INTO role_id_val FROM roles WHERE name = 'admin';
            IF role_id_val IS NOT NULL THEN
                INSERT INTO user_roles (user_id, role_id) VALUES (r.id, role_id_val)
                ON CONFLICT DO NOTHING;
            END IF;
        ELSIF r.role IN ('operator', 'viewer') THEN
            SELECT id INTO role_id_val FROM roles WHERE name = r.role;
            IF role_id_val IS NOT NULL THEN
                INSERT INTO user_roles (user_id, role_id) VALUES (r.id, role_id_val)
                ON CONFLICT DO NOTHING;
            END IF;
        END IF;
    END LOOP;
END $$;

ALTER TABLE users DROP COLUMN role;
