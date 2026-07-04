# Assimilate

Self-hosted Borg backup orchestration — central dashboard, per-agent tokens, schedules, and real-time status across all your machines.

Assimilate is a self-hosted orchestration layer for [BorgBackup](https://borgbackup.readthedocs.io). A lightweight agent runs on each machine; everything is managed from one dashboard.

## Quick start

```yaml
# docker-compose.yml
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_DB: borg
      POSTGRES_USER: borg
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-borg_secret}
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U borg -d borg"]
      interval: 5s
      timeout: 3s
      retries: 5

  server:
    image: ghcr.io/alexmohr/assimilate:latest
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://borg:${POSTGRES_PASSWORD:-borg_secret}@db:5432/borg
      ASSIMILATE_SECRET_KEY: ${ASSIMILATE_SECRET_KEY:?must be set}
    volumes:
      - ssh_keys:/app/ssh
    depends_on:
      db:
        condition: service_healthy

volumes:
  pgdata:
  ssh_keys:
```

```bash
export ASSIMILATE_SECRET_KEY=$(openssl rand -hex 32) && docker compose up -d
```

Open [http://localhost:8080](http://localhost:8080) — login: `admin` / `admin`.

## Next steps

- [Getting Started](getting-started.md) — full setup walkthrough
- [Architecture](architecture.md) — how the components fit together
- [Configuration](configuration.md) — environment variables reference
- [Security & Authentication](security.md) — auth model, encryption, RBAC
- [Agent Management](agents.md) — add machines, deploy agents
- [Repository Management](repositories.md) — init and import repos
- [API Reference](api-reference.md) — REST API documentation

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
