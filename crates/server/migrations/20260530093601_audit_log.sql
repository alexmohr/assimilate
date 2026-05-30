-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
--
-- See the NOTICE file(s) distributed with this work for additional
-- information regarding copyright ownership.
--
-- This program and the accompanying materials are made available under the
-- terms of the Apache License Version 2.0 which is available at
-- https://www.apache.org/licenses/LICENSE-2.0

CREATE TABLE audit_log (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT,
  username TEXT NOT NULL DEFAULT 'system',
  action TEXT NOT NULL,
  target_type TEXT,
  target_id BIGINT,
  details JSONB,
  ip_address TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_log_created_at ON audit_log (created_at DESC);
CREATE INDEX idx_audit_log_user_id ON audit_log (user_id);
CREATE INDEX idx_audit_log_target ON audit_log (target_type, target_id);
