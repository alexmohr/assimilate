# Getting Started

This guide walks you through installing and running Assimilate for the first time, from prerequisites to your first completed backup.

## Prerequisites

Install the following before proceeding:

- **Rust nightly** — install via [rustup](https://rustup.rs/): `rustup toolchain install nightly`
- **Node.js 20+** — required to build the frontend
- **PostgreSQL** — the server stores all state in a PostgreSQL database
- **BorgBackup** — must be installed on every machine running the agent

!!! tip
    The [Devcontainer Setup](#devcontainer-setup) section provides a fully self-contained environment with all dependencies pre-installed — recommended for contributors and evaluation.

## Docker Compose Quickstart

The included `docker-compose.yml` runs the full stack: PostgreSQL, the Assimilate server, and one agent.

### Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `ASSIMILATE_SECRET_KEY` | **Yes** | — | 32-byte hex key used to encrypt repository passphrases at rest (AES-256-GCM) |
| `POSTGRES_PASSWORD` | No | `borg_secret` | Password for the PostgreSQL `borg` user |
| `BORG_AGENT_TOKEN` | Agent only | — | Token copied from the server UI after creating a host |

!!! warning "Protect `ASSIMILATE_SECRET_KEY`"
    This key derives the AES-256-GCM encryption key that protects all repository passphrases stored in the database. If you lose or rotate this value, every encrypted passphrase becomes **permanently unrecoverable**. Store it in a secrets manager or a secure `.env` file that is never committed to version control.

### `docker-compose.yml`

```yaml
services:
  postgres:
    image: postgres:latest
    environment:
      POSTGRES_DB: borg
      POSTGRES_USER: borg
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-borg_secret}
    volumes:
      - pgdata:/var/lib/postgresql
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U borg -d borg"]
      interval: 5s
      timeout: 3s
      retries: 5

  server:
    build:
      context: .
      dockerfile: Dockerfile.server
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://borg:${POSTGRES_PASSWORD:-borg_secret}@postgres:5432/borg
      ASSIMILATE_SECRET_KEY: ${ASSIMILATE_SECRET_KEY:?ASSIMILATE_SECRET_KEY must be set}
      ASSIMILATE_BIND_ADDR: "0.0.0.0:8080"
      ASSIMILATE_STATIC_DIR: /app/static
    volumes:
      - ssh_keys:/app/ssh
    depends_on:
      postgres:
        condition: service_healthy

volumes:
  pgdata:
  ssh_keys:
```

### First run

Generate a secret key and start the server:

```bash
export ASSIMILATE_SECRET_KEY=$(openssl rand -hex 32)
docker compose up -d postgres server
```

The server creates a default admin account and generates an Ed25519 SSH key pair on first start. The SSH key pair is persisted in the `ssh_keys` volume and the public key is visible under **System** in the admin UI.

Once the server is running, create a host in the UI, copy its token, then start the agent:

```bash
export BORG_AGENT_TOKEN=<token from server UI>
docker compose up -d agent
```

To run multiple agents for different machines, pass the token per instance:

```bash
docker compose run -e BORG_AGENT_TOKEN=<other-token> agent
```

## Manual Build

Build the server and agent binaries:

```bash
cargo build --workspace
```

Build the frontend and place the output where the server can serve it:

```bash
cd frontend && npm install && npm run build
```

The server looks for static files in the directory specified by `ASSIMILATE_STATIC_DIR` (default: `./static`). Copy the built frontend there:

```bash
cp -r frontend/dist/* static/
```

Start the server:

```bash
export DATABASE_URL=postgres://borg:borg_secret@localhost:5432/borg
export ASSIMILATE_SECRET_KEY=$(openssl rand -hex 32)
cargo run -p server
```

Start the agent on each backup machine (after creating a host in the UI):

```bash
export BORG_SERVER_URL=http://<server-address>:8080
export BORG_AGENT_TOKEN=<token from server UI>
cargo run -p agent
```

The agent accepts `http://`, `https://`, `ws://`, or `wss://` URLs — it normalises them automatically.

See [Configuration](configuration.md) for the full list of server environment variables.

## First Login

Open `http://localhost:8080` in your browser.

![Login](assets/screenshots/login.png)

!!! warning "Change the default password immediately"
    The server creates a default admin account with credentials `admin` / `admin`. You are required to change the password on first login. Do not skip this step on any internet-facing deployment.

After changing the password, the dashboard loads and shows no hosts yet.

![Dashboard](assets/screenshots/dashboard.png)

## Adding Your First Host

A *host* represents a machine running the Assimilate agent.

1. Navigate to **Clients** and click **Add Host**.
2. Enter a display name for the machine.
3. Click **Save** — the server generates a unique agent token.
4. Copy the token shown on the host detail page.
5. On the backup machine, start the agent with that token:

```bash
BORG_SERVER_URL=http://<server-address>:8080 \
BORG_AGENT_TOKEN=<copied-token> \
./assimilate-agent
```

The host status changes to **Online** in the dashboard once the agent connects.

See [Host & Agent Management](hosts.md) for systemd unit examples and advanced options.

## Adding Your First Repository

A *repository* is a Borg backup repository, typically accessed over SSH.

### SSH key setup

The server holds an Ed25519 key pair used to authenticate to borg repository hosts. Retrieve the public key from **System** in the admin UI, then add it to `~/.ssh/authorized_keys` on the repository host:

```bash
# On the repository host:
echo "<public key from System page>" >> ~/.ssh/authorized_keys
```

!!! tip
    Use [SSH Agent Forwarding](ssh-agent-forwarding.md) so agent machines authenticate using the server's key — no SSH keys need to be distributed to agent machines.

### Create the repository

1. Navigate to **Repos** and click **Add Repository**.
2. Fill in the connection details:

    | Field | Example |
    |---|---|
    | SSH Host | `backup.example.com` |
    | SSH User | `borg` |
    | SSH Port | `22` |
    | Repo Path | `/backup/repos/myhost` |
    | Passphrase | a strong random passphrase |

3. Click **Test Connection** to verify SSH access.
4. Click **Initialize** to run `borg init` and create the repository.

See [Repository Management](repositories.md) for pruning policies and passphrase rotation.

## Your First Backup

1. Navigate to **Scheduling** and click **Add Schedule**.
2. Select the host and repository created above.
3. Set the paths to back up (e.g., `/home`, `/etc`).
4. Choose a cron expression or interval (e.g., `0 2 * * *` for 2 AM daily).
5. Click **Save**.
6. Click **Run Now** to trigger an immediate backup.

The dashboard shows the backup progress in real time. Once complete, the archive appears under **Archives** for the repository.

See [Scheduling & Retention](scheduling.md) for retention policies and pre/post hooks.

## Devcontainer Setup

The project includes a devcontainer for contributors. It provides a fully self-contained environment — no local PostgreSQL, SSH keys, or borg installation required.

### Services

| Service | Purpose |
|---|---|
| `dev` | Rust nightly + Node.js + borg — your workspace |
| `postgres` | PostgreSQL database |
| `borg-repo` | SSH server + borg acting as the repository target |

### Start the environment

Open the project in VS Code and select **Reopen in Container** (or run `Dev Containers: Reopen in Container` from the command palette).

On first start, an SSH key pair is generated and shared with the `borg-repo` container automatically.

Start the server and Vite dev server with a single command:

```bash
.devcontainer/start.sh
```

After creating a host in the UI and copying its token, start the agent:

```bash
source /tmp/ssh-agent-env.sh
BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN=<token> cargo run -p agent
```

When creating a repository in the devcontainer, use these settings:

| Field | Value |
|---|---|
| SSH Host | `localhost` or `borg-repo` |
| SSH User | `borg` |
| SSH Port | `22` |
| Repo Path | `/backup/repos/<name>` |
| Passphrase | any value (e.g. `devpass`) |

!!! tip
    Without a devcontainer, start PostgreSQL manually and set `DATABASE_URL` and `ASSIMILATE_SECRET_KEY` before running `cargo run -p server`. Run `cd frontend && npm run dev` in a separate terminal for hot-reload.

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
