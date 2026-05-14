# Assimilate

[![CI](https://github.com/alexmohr/assimilate/actions/workflows/ci.yml/badge.svg)](https://github.com/alexmohr/assimilate/actions/workflows/ci.yml)

A self-hosted BorgBackup management server — web UI, multi-host agent orchestration, and scheduled backups with a REST API.

> **Alpha Software** — This project is under heavy development and not yet recommended for production backups. Expect breaking changes, incomplete features, and data-format migrations between releases.

## Key Features

- **Web UI** — light/dark Vue 3 SPA with live agent status and backup history
- **Agent-based** — lightweight agent binary runs on each backup machine, connects over WebSocket
- **SSH agent forwarding** — server holds SSH keys; no keys distributed to agent machines
- **AES-256-GCM encryption** — repository passphrases encrypted at rest
- **RBAC + API tokens** — session auth, role-based access control, brute-force protection
- **REST API + OpenAPI** — full programmatic access
- **Scheduled backups with retention** — cron-based scheduling, pruning policies, pre/post hooks

## Quick Start

```bash
export ASSIMILATE_SECRET_KEY=$(openssl rand -hex 32)
docker compose up -d postgres server
```

Open `http://localhost:8080` and log in with `admin` / `admin` (password change required on first login).

See the full [Getting Started guide](docs/getting-started.md) for adding hosts, repositories, and scheduling your first backup.

## Documentation

The docs are served by the app at `/docs/` when running. Source files:

| Topic | File |
|---|---|
| Getting Started | [docs/getting-started.md](docs/getting-started.md) |
| Configuration | [docs/configuration.md](docs/configuration.md) |
| Hosts & Agent Management | [docs/hosts.md](docs/hosts.md) |
| Repository Management | [docs/repositories.md](docs/repositories.md) |
| Scheduling & Retention | [docs/scheduling.md](docs/scheduling.md) |
| Archives | [docs/archives.md](docs/archives.md) |
| SSH Agent Forwarding | [docs/ssh-agent-forwarding.md](docs/ssh-agent-forwarding.md) |
| SSH Reverse Tunnels | [docs/ssh-tunnels.md](docs/ssh-tunnels.md) |
| Security | [docs/security.md](docs/security.md) |
| API Reference | [docs/api-reference.md](docs/api-reference.md) |
| Architecture | [docs/architecture.md](docs/architecture.md) |
| Contributing | [docs/contributing/](docs/contributing/) |

## Development

```bash
cargo build --workspace
cd frontend && npm install && npm run dev
```

The project includes a devcontainer with PostgreSQL and a borg repository server pre-configured — see [Getting Started → Devcontainer Setup](docs/getting-started.md#devcontainer-setup).

## License

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

Licensed under the [Apache License, Version 2.0](LICENSE).
