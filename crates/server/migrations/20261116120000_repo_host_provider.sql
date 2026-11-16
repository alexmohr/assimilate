-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- The hosting service a repository host belongs to. A provider fixes the
-- defaults a host form would otherwise have the user type by hand: the SSH
-- port, the directory repositories live in, and the borg binary the remote
-- side runs (`--remote-path`). Every host that exists today was entered by
-- hand, so it is generic.
ALTER TABLE repo_hosts
    ADD COLUMN provider TEXT NOT NULL DEFAULT 'generic'
    CONSTRAINT repo_hosts_provider_check
    CHECK (provider IN ('generic', 'hetzner', 'rsync_net', 'borgbase'));
