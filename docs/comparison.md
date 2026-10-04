<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Comparison

This page compares Assimilate with other tools built on [BorgBackup](https://borgbackup.readthedocs.io). Use it to decide whether Assimilate fits your setup, or which tool to pair it with. The comparison reflects the state of each project in October 2026; open an issue if an entry is out of date.

## At a glance

| | Assimilate | Borg Backup Server (BBS) | borgmatic | Vorta |
|---|---|---|---|---|
| Kind | Central server + agents | Central server + agents | Per-host CLI and config file | Desktop GUI |
| Manages many hosts from one UI | Yes | Yes | No | No |
| Web UI | Yes | Yes | No | No (desktop app) |
| Server stack | Rust, PostgreSQL | PHP, MySQL/MariaDB, ClickHouse | n/a | n/a |
| Agent / client platforms | Linux | Linux, Windows, macOS, BSD, NAS | Linux, macOS, BSD | Linux, macOS |
| License | Apache-2.0 | MIT | GPL-3.0 | GPL-3.0 |

## Assimilate and BBS in detail

BBS is the closest tool to Assimilate: both run a central server with agents on each machine. The entries below are based on the BBS documentation and source code.

### Security model

| | Assimilate | BBS |
|---|---|---|
| Where repository SSH keys live | On the server; agents sign through an ssh-agent relay | Per-client key for repositories on the BBS server; for remote providers the server sends the private key to the agent with each job |
| Inbound connections to clients | None; agents dial out over WebSocket | None; agents poll over HTTPS |
| Append-only enforcement | Manual, via `authorized_keys` on the repository host | Enforced for repositories stored on the BBS server; not for remote providers |
| Passphrases at rest | AES-256-GCM | AES-256-GCM |
| Two-factor login | TOTP with recovery codes | TOTP with recovery codes |
| Single sign-on (OIDC) | No | Yes |
| Permissions | Built-in and custom roles, groups, per-repository | Admin and user roles, per-client, per-action |
| Audit log | Dedicated, filterable, exportable | Server log |

!!! note
    If the ssh-agent relay is unavailable, the Assimilate agent falls back to SSH keys present on the agent machine. Leave no keys on agent machines to make the relay the only way in. See [SSH Agent Forwarding](ssh-agent-forwarding.md).

### Backups and scheduling

| | Assimilate | BBS |
|---|---|---|
| One schedule across many hosts | Yes | No (plans per client) |
| Several target repositories per schedule | Yes, each required or best effort | One repository per plan, plus offsite copies |
| Hosts that are not always online | Catch-up runs with re-check interval and give-up window | Queued while Wake-on-LAN wakes the client |
| Wake-on-LAN before a backup | Clients and repository hosts | Clients |
| Shutdown after a backup | Yes | No |
| Filesystem snapshots (ZFS, btrfs, LVM) | No | Yes |
| libvirt VM snapshots | Yes | No |
| Database dumps | Hook commands | MySQL/MariaDB, PostgreSQL, MongoDB plugins |
| Storage quotas with enforcement | Yes, per repository and per host | No |
| Hosted-provider presets (BorgBase, Hetzner, rsync.net) | No | Yes |
| Repositories on the management server itself | No | Yes |
| Offsite copy to S3-compatible storage | No | Yes |

### Restore

| | Assimilate | BBS |
|---|---|---|
| File index and search across archives | Yes | Yes |
| Archive diff | Yes | No |
| Restore to the original location | Yes | Yes |
| Restore to another path | No | Yes |
| Restore to another client | No | No |
| Browser download | Yes (tar.lz4 stream) | Yes (tar.gz) |
| Mount an archive (FUSE) | No | No |

### Operations

| | Assimilate | BBS |
|---|---|---|
| Notifications | Email, webhook, browser push | Email, Apprise, iOS app push |
| REST API | Yes, with OpenAPI specification | Yes |
| Prometheus metrics | No | Yes |
| Self-backup of the management server | Configuration export only | Yes, with restore scripts |
| Deployment | Docker (amd64, arm64) | Docker, installer for Ubuntu, Unraid template |

## Which tool fits

- **One machine, desktop use:** Vorta.
- **A few servers, configuration as code:** borgmatic.
- **Mixed Windows, macOS, and Linux fleet:** BBS.
- **Linux fleet where repository keys must stay off clients, or homelabs with VMs, NAS wake-up, and laptops:** Assimilate.

borgmatic and Assimilate do not conflict: borgmatic manages borg on one host, Assimilate orchestrates borg across many.

## Related pages

- [Architecture](architecture.md)
- [Security & Authentication](security.md)
- [SSH Agent Forwarding](ssh-agent-forwarding.md)
- [How It's Built](how-its-built.md)
