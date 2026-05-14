-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user',
    must_change_password BOOLEAN NOT NULL DEFAULT false,
    preferences JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE login_attempts (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL,
    ip TEXT NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE api_tokens (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ
);

CREATE TABLE clients (
    id BIGSERIAL PRIMARY KEY,
    hostname TEXT NOT NULL UNIQUE,
    display_name TEXT,
    agent_token_hash TEXT NOT NULL,
    agent_version TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ,
    owner_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    visibility TEXT NOT NULL DEFAULT 'shared' CHECK (visibility IN ('private', 'shared')),
    default_backup_paths TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    default_exclude_patterns TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[]
);

CREATE TABLE repos (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    repo_path TEXT NOT NULL,
    ssh_user TEXT NOT NULL DEFAULT 'root',
    ssh_host TEXT NOT NULL,
    ssh_port INTEGER NOT NULL DEFAULT 22,
    passphrase_encrypted BYTEA NOT NULL,
    compression TEXT NOT NULL DEFAULT 'lz4',
    encryption TEXT NOT NULL DEFAULT 'repokey',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    owner_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    visibility TEXT NOT NULL DEFAULT 'shared' CHECK (visibility IN ('private', 'shared'))
);

CREATE TABLE schedules (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT REFERENCES clients(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    schedule_type TEXT NOT NULL DEFAULT 'backup',
    cron_expression TEXT NOT NULL DEFAULT '0 2 * * *',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    canary_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    last_run_at TIMESTAMPTZ,
    next_run_at TIMESTAMPTZ,
    exclude_patterns TEXT[] NOT NULL DEFAULT '{}',
    ignore_global_excludes BOOLEAN NOT NULL DEFAULT FALSE,
    keep_daily INTEGER NOT NULL DEFAULT 7,
    keep_weekly INTEGER NOT NULL DEFAULT 4,
    keep_monthly INTEGER NOT NULL DEFAULT 6,
    keep_yearly INTEGER NOT NULL DEFAULT 0,
    compact_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    pre_backup_commands TEXT NOT NULL DEFAULT '[]',
    post_backup_commands TEXT NOT NULL DEFAULT '[]',
    owner_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    visibility TEXT NOT NULL DEFAULT 'shared' CHECK (visibility IN ('private', 'shared')),
    UNIQUE (client_id, repo_id, cron_expression)
);

CREATE TABLE backup_sources (
    id BIGSERIAL PRIMARY KEY,
    repo_id BIGINT REFERENCES repos(id) ON DELETE CASCADE,
    schedule_id BIGINT REFERENCES schedules(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE (schedule_id, path)
);

CREATE TABLE backup_reports (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL,
    original_size BIGINT NOT NULL DEFAULT 0,
    compressed_size BIGINT NOT NULL DEFAULT 0,
    deduplicated_size BIGINT NOT NULL DEFAULT 0,
    files_processed BIGINT NOT NULL DEFAULT 0,
    duration_secs BIGINT NOT NULL DEFAULT 0,
    error_message TEXT,
    warnings TEXT[] NOT NULL DEFAULT '{}',
    borg_version TEXT
);

CREATE TABLE excludes_global (
    id BIGSERIAL PRIMARY KEY,
    pattern TEXT NOT NULL UNIQUE,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE canary_results (
    id BIGSERIAL PRIMARY KEY,
    repo_id BIGINT REFERENCES repos(id) ON DELETE CASCADE,
    schedule_id BIGINT REFERENCES schedules(id) ON DELETE CASCADE,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL,
    canary_filename TEXT,
    error_message TEXT,
    archive_name TEXT
);

CREATE TABLE repo_permissions (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    can_view BOOLEAN NOT NULL DEFAULT false,
    can_backup BOOLEAN NOT NULL DEFAULT false,
    can_modify_schedules BOOLEAN NOT NULL DEFAULT false,
    can_extract BOOLEAN NOT NULL DEFAULT false,
    can_delete BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (user_id, repo_id)
);

CREATE TABLE system_events (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    event_type TEXT NOT NULL,
    hostname TEXT,
    message TEXT NOT NULL
);

CREATE TABLE system_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tags (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#6b7280',
    scope TEXT NOT NULL CHECK (scope IN ('repo', 'host')),
    UNIQUE (name, scope)
);

CREATE TABLE repo_tags (
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    tag_id BIGINT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (repo_id, tag_id)
);

CREATE TABLE host_tags (
    client_id BIGINT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    tag_id BIGINT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (client_id, tag_id)
);

CREATE TABLE ssh_tunnels (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL UNIQUE REFERENCES clients(id) ON DELETE CASCADE,
    ssh_host TEXT NOT NULL,
    ssh_user TEXT NOT NULL DEFAULT 'root',
    ssh_port INTEGER NOT NULL DEFAULT 22,
    tunnel_port INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE groups (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE user_groups (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id BIGINT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_id)
);

CREATE TABLE roles (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    can_create_client BOOLEAN NOT NULL DEFAULT false,
    can_delete_client BOOLEAN NOT NULL DEFAULT false,
    can_delete_own_client BOOLEAN NOT NULL DEFAULT false,
    can_create_repo BOOLEAN NOT NULL DEFAULT false,
    can_delete_repo BOOLEAN NOT NULL DEFAULT false,
    can_delete_own_repo BOOLEAN NOT NULL DEFAULT false,
    can_create_schedule BOOLEAN NOT NULL DEFAULT false,
    can_delete_schedule BOOLEAN NOT NULL DEFAULT false,
    can_delete_own_schedule BOOLEAN NOT NULL DEFAULT false,
    can_manage_tags BOOLEAN NOT NULL DEFAULT false,
    can_view_all_repos BOOLEAN NOT NULL DEFAULT false,
    can_manage_tunnels BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE user_roles (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id BIGINT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
);

CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX idx_login_attempts_lookup ON login_attempts(username, ip, attempted_at);
CREATE INDEX idx_api_tokens_user_id ON api_tokens(user_id);
CREATE UNIQUE INDEX idx_api_tokens_hash ON api_tokens(token_hash);
CREATE INDEX idx_backup_reports_client_id ON backup_reports(client_id);
CREATE INDEX idx_backup_reports_repo_id ON backup_reports(repo_id);
CREATE INDEX idx_backup_reports_started_at ON backup_reports(started_at DESC);
CREATE INDEX idx_schedules_next_run_at ON schedules(next_run_at);
CREATE INDEX idx_system_events_created_at ON system_events(created_at DESC);
CREATE INDEX idx_system_events_event_type ON system_events(event_type);
CREATE INDEX idx_canary_results_repo_id ON canary_results(repo_id);
CREATE INDEX idx_canary_results_verified_at ON canary_results(verified_at DESC);
CREATE INDEX idx_repo_tags_tag_id ON repo_tags(tag_id);
CREATE INDEX idx_host_tags_tag_id ON host_tags(tag_id);
CREATE INDEX idx_user_groups_group_id ON user_groups(group_id);
CREATE INDEX idx_user_roles_role_id ON user_roles(role_id);

INSERT INTO system_settings (key, value) VALUES ('retention_days', '7');

INSERT INTO roles (name, can_create_client, can_delete_client, can_delete_own_client, can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, can_manage_tunnels)
VALUES
  ('admin', true, true, true, true, true, true, true, true, true, true, true, true),
  ('operator', true, false, true, true, false, true, true, false, true, true, true, false),
  ('viewer', false, false, false, false, false, false, false, false, false, false, false, false);
