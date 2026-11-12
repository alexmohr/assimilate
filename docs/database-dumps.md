<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Database Dumps

An agent can dump PostgreSQL, MySQL, MariaDB, MongoDB, SQLite, Redis, Valkey and InfluxDB databases into the same archive as the rest of a backup. It finds them in three places:

- **a Kubernetes cluster**: `CloudNativePG` clusters and annotated pods of any engine, dumped inside their own pod;
- **Docker or Podman containers**: containers of any engine carrying a label, dumped inside the container;
- **the host itself**: PostgreSQL, MySQL, MariaDB or MongoDB servers installed on the agent's host, and SQLite files on it, dumped by running their own tools locally. Redis, Valkey and InfluxDB are dumped in pods and containers only.

Every dump is streamed straight into `borg create`: nothing is written to the agent's disk first, so a 200 GB database needs no 200 GB of scratch space.

Like [VM snapshots](vm-snapshots.md), the settings belong to the **host**, because the credentials and the dump directory live there. A schedule only opts in.

## Prerequisites

- A Linux [agent](agents.md), running as root like any agent that backs up a whole system.
- For a cluster: credentials that may list pods and exec into them. See [Grant the agent access](#grant-the-agent-access).
- For containers: the engine's API socket on the agent's host (Docker's `/var/run/docker.sock`, or Podman's `/run/podman/podman.sock` with `podman.socket` enabled).
- The engine's own client programs where the dump runs: inside the database container for a cluster or a container, or on the host for a server installed there. Every official image of the engine has them, and so does every `CloudNativePG` image:

    | Engine | To dump | To restore from Assimilate |
    |--------|---------|----------------------------|
    | PostgreSQL | `pg_dump`, `psql` | `pg_restore`, `createdb`, `dropdb` |
    | MySQL | `mysqldump`, `mysql` | `mysql` |
    | MariaDB | `mariadb-dump`, `mariadb` | `mariadb` |
    | MongoDB | `mongodump`, `mongosh` | `mongorestore`, `mongosh` |
    | SQLite | `sqlite3` | `sqlite3`, `sh` |
    | Redis | `redis-cli` | `redis-cli`, `redis-server`, `sh`, `mktemp` |
    | Valkey | `valkey-cli` | `valkey-cli`, `valkey-server`, `sh`, `mktemp` |
    | InfluxDB 2 | `influx`, `tar`, `mktemp` | `influx`, `tar`, `mktemp` |
    | InfluxDB 1 | `influx`, `influxd`, `tar`, `mktemp` | `influx`, `influxd`, `tar`, `mktemp` |

    Not every image that keeps its data in SQLite ships `sqlite3`: check with `docker exec <container> sqlite3 -version`, and add it to the image (for example `apk add sqlite` or `apt-get install sqlite3`) where it is missing.

    A MySQL, MariaDB or MongoDB container also needs `sh`, `printenv`, `cat` and `sed`, which sign the client in (see [How MySQL, MariaDB and MongoDB sign in](#how-mysql-mariadb-and-mongodb-sign-in)). The official images have them.

## Configure a host

Open the agent, then its **Sources** tab and the **Databases** section. Only admins see this tab: its settings name the host's engine socket, OS user and directories.

![The Databases section, listing a host's database sources and their databases](assets/screenshots/agent-databases.png)

1. Turn on **Allow schedules to dump databases**. Like the VM setting, this is a permission rather than an action: nothing is dumped until a schedule asks for it.
2. Choose **which sources** to dump. **All except excluded** dumps every source the agent finds, including one deployed after the last scan; **Only selected** dumps nothing until you pick the sources you want.
3. Turn on each place the agent should look, and fill in how it reaches it:
    - **Search the Kubernetes cluster**, with the **kubeconfig**, a path to a file on the agent's host. Leave it empty to use the in-cluster service account, or `$KUBECONFIG` and `~/.kube/config`. The file never leaves the host.

        To reach the API server through an HTTP or HTTPS proxy, set the cluster's `proxy-url` in the kubeconfig, or `HTTPS_PROXY` in the agent's environment. `NO_PROXY` is not consulted, so leave `HTTPS_PROXY` unset for the agent if the cluster should be reached directly.
    - **Search Docker or Podman containers**, with the **engine socket**. Leave it empty for `$DOCKER_HOST`, or `/var/run/docker.sock`.
    - **Dump PostgreSQL on this host**, with the **OS user** the dump runs as (`postgres` when empty), and, when the server is not where the clients look by default, its **socket directory**, **port** and the **programs directory** holding the matching `pg_dump`. See [PostgreSQL on the host](#postgresql-on-the-host).
    - **Dump MySQL or MariaDB on this host**, with which **server** it is, the **OS user** the clients run as (`root` when empty) and, when needed, its **socket**, a TCP **port** instead, and the **programs directory**. See [MySQL and MariaDB on the host](#mysql-and-mariadb-on-the-host).
    - **Dump MongoDB on this host**, with the **OS user** (`root` when empty), a **config file** naming a server that wants a password, the **port** otherwise, and the **programs directory**. See [MongoDB on the host](#mongodb-on-the-host).
    - **Dump SQLite files on this host**, with the **database files**, one absolute path per line, the **OS user** that may read them (`root` when empty) and the **programs directory** holding `sqlite3`. See [SQLite files](#sqlite-files).
4. Set the **dump directory**, an absolute path on the agent's host. It holds one named pipe per database while a backup runs, and the archive stores the dumps under it. It may not be below `/dev`, `/proc`, `/sys` or `/run`.
5. Click **Scan for databases**. While the settings form is open, the scan searches with the settings in the form rather than the saved ones, so you can test them before saving. When one place cannot be searched but another can, the scan shows what it found together with why the other failed, and keeps the sources it had already found in the failed place rather than dropping them.

Every place you turn on must be reachable when a backup runs: a backup whose cluster or container engine cannot be reached fails with the reason, rather than leaving databases out.

Then turn on **Dump databases** in a schedule's **Settings → Advanced** section, or on the **Advanced** step of a new schedule.

Both switches have to be on. A schedule that asks for dumps on a host where **Allow schedules to dump databases** is off backs that host up without them, and says so: the schedule's form and its page name every such host with a link to its database settings, the host's database settings name the schedules asking, and each such run is reported as a **warning** in the Activity Log with the reason. The same applies to [virtual machines](vm-snapshots.md) a schedule asks for on a host with staging turned off.

## How sources are found

The agent groups what it finds into **sources**, one per database workload:

| Found as | Where | Recognised by | Dumped from |
|----------|-------|---------------|-------------|
| `CloudNativePG` | cluster | the `cnpg.io/cluster` label | a replica when the cluster has one, so a nightly dump does not load the primary; the primary otherwise |
| Annotated pod | cluster | the `assimilate.alexmohr.github.io/database` annotation, such as `postgresql` or `mariadb` | the first running pod of the workload |
| Labelled container | Docker or Podman | the `assimilate.alexmohr.github.io/database` label on a running container | the first container of the Compose service |
| Host server | the host | turning on **Dump PostgreSQL on this host**, **Dump MySQL or MariaDB on this host**, **Dump MongoDB on this host** or **Dump SQLite files on this host** | the server itself, or the files |

A source is named after the workload, not after a pod or container, so it keeps its name and your choice across rollouts:

- a cluster source by its namespace and workload (`shop/orders-db`); the pods of one Deployment are one source;
- a container by its Compose project and service (`gitea/db`), or by its name when it is not part of a Compose project; the containers of one scaled service are one source;
- a server installed on the host after its engine, as **PostgreSQL on this host**, **MariaDB on this host** and so on.

### Pods and containers

An annotated pod or a labelled container can say more about itself, with the same keys:

| Annotation or label | Meaning | Default |
|---------------------|---------|---------|
| `assimilate.alexmohr.github.io/database` | The engine: `postgresql`, `mysql`, `mariadb`, `mongodb`, `sqlite`, `redis`, `valkey`, `influxdb` (InfluxDB 2) or `influxdb1` (InfluxDB 1) | required |
| `assimilate.alexmohr.github.io/container` | (pods only) The container to run the dump in | the pod's first container |
| `assimilate.alexmohr.github.io/user` | The account to connect as | `postgres` for PostgreSQL, `root` for MySQL and MariaDB, the image's root account for MongoDB, InfluxDB 1 and Redis or Valkey ACL users: the image's admin account, or the default user |
| `assimilate.alexmohr.github.io/password-env` | (MySQL, MariaDB, MongoDB, Redis, Valkey and InfluxDB) The variable in the container's environment that holds the account's password, or InfluxDB 2's token | the variables the official image or chart reads its password from |
| `assimilate.alexmohr.github.io/databases` | A comma-separated list of databases to dump; for SQLite, the absolute paths of the database files. Ignored by Redis, Valkey and InfluxDB 2, which dump their whole server | every database the server holds, except its own system databases; required for SQLite |

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: orders-db
  namespace: shop
spec:
  template:
    metadata:
      annotations:
        assimilate.alexmohr.github.io/database: postgresql
        assimilate.alexmohr.github.io/container: postgres
        assimilate.alexmohr.github.io/user: orders
        assimilate.alexmohr.github.io/databases: orders,payments
```

```yaml
# compose.yaml
services:
  db:
    image: postgres:16
    labels:
      assimilate.alexmohr.github.io/database: postgresql
      assimilate.alexmohr.github.io/databases: gitea
```

For PostgreSQL, `pg_dump` connects over the pod's or container's local socket without a password, so the role must be allowed to connect locally. That holds for the `postgres` superuser in the official image and in `CloudNativePG`.

```yaml
# compose.yaml
services:
  wiki-db:
    image: mariadb:11
    environment:
      MARIADB_ROOT_PASSWORD_FILE: /run/secrets/db_root_password
    labels:
      assimilate.alexmohr.github.io/database: mariadb
  chat-db:
    image: mongo:7
    environment:
      MONGO_INITDB_ROOT_USERNAME: root
      MONGO_INITDB_ROOT_PASSWORD: ${CHAT_DB_PASSWORD}
    labels:
      assimilate.alexmohr.github.io/database: mongodb
```

### SQLite files

SQLite has no server to ask for its databases and no account to sign in as: the agent runs `sqlite3` on the database files themselves. Name the files by their absolute paths inside the container in the `databases` annotation or label, or on the host in **Database files**:

```yaml
# compose.yaml
services:
  vaultwarden:
    image: vaultwarden/server
    labels:
      assimilate.alexmohr.github.io/database: sqlite
      assimilate.alexmohr.github.io/databases: /data/db.sqlite3
```

A dump runs `sqlite3 -readonly <file> .dump`, which reads the whole file in one transaction: the dump is consistent while the application keeps writing, and it never writes to the file. A source that names no files, or a path that is not absolute or holds `..`, is listed with an error instead of being dumped.

On the host, `sqlite3` runs as the **OS user** in the settings, which must be allowed to read each file, and, to restore over one, to write it and create files in its directory.

#### Applications that keep their data in SQLite

Home Assistant, Grafana and Jellyfin all keep their state in SQLite files. Each is a container of its own, so each gets a label naming its files, and a dump of it is consistent while it keeps writing, write-ahead log included:

| Application | Files to name (inside the container) |
|-------------|--------------------------------------|
| Home Assistant | `/config/home-assistant_v2.db` (the recorder history) |
| Grafana | `/var/lib/grafana/grafana.db` |
| Jellyfin | `/config/data/jellyfin.db` and `/config/data/library.db` |

```yaml
# compose.yaml
services:
  grafana:
    image: grafana/grafana
    labels:
      assimilate.alexmohr.github.io/database: sqlite
      assimilate.alexmohr.github.io/databases: /var/lib/grafana/grafana.db
```

These images do not all ship `sqlite3`, and Assimilate runs the dump with the program inside the container, so the backup of a container without it fails with the reason. Check with `docker exec <container> sqlite3 -version` and add `sqlite3` to the image before turning the source on. The paths above are the images' defaults; check them against your own volume mounts.

### How MySQL, MariaDB and MongoDB sign in

These servers want a password, and the database's own container already holds it: the official images, and the Bitnami charts, are set up with it in their environment. The agent runs the client through a short `sh` script inside the container that reads the password from there and hands it to the client through a file descriptor (an options file for MySQL and MariaDB, a config file for the MongoDB tools). Assimilate never stores the password, and it never appears on a command line, where every process in the container could read it.

| Engine | Account | Password read from |
|--------|---------|--------------------|
| MySQL | `root` | `MYSQL_ROOT_PASSWORD` |
| MariaDB | `root` | `MARIADB_ROOT_PASSWORD`, then `MYSQL_ROOT_PASSWORD` |
| MongoDB | `MONGO_INITDB_ROOT_USERNAME` or `MONGODB_ROOT_USER`, `root` when neither is set | `MONGO_INITDB_ROOT_PASSWORD`, then `MONGODB_ROOT_PASSWORD` |

Each variable can also be given as a file, the way Docker and Kubernetes secrets are mounted: `MARIADB_ROOT_PASSWORD_FILE=/run/secrets/db_root_password` is read like `MARIADB_ROOT_PASSWORD`. A container that holds none of the variables is reached without a password, which suits a server that lets local connections in without one.

To dump as another account, name it with the `user` annotation or label and name the variable holding its password with `password-env`:

```yaml
annotations:
  assimilate.alexmohr.github.io/database: mysql
  assimilate.alexmohr.github.io/user: backup
  assimilate.alexmohr.github.io/password-env: BACKUP_DB_PASSWORD
```

`password-env` names exactly one variable: a letter or `_`, then letters, digits and `_`. Any other value is ignored, with a warning in the agent's log, and the source signs in with its image's own variables.

!!! note "The account's privileges"
    A MySQL or MariaDB dump reads routines, events and triggers too, so the account needs `SELECT`, `SHOW VIEW`, `TRIGGER`, `EVENT` and `LOCK TABLES`; `root` has them. A MongoDB account needs the `backup` role to dump and `restore` to restore; the root account has both.

### Redis and Valkey

A dump is the RDB file `redis-cli --rdb -` (or `valkey-cli --rdb -`) downloads from the server and writes to stdout, so it is streamed into the archive like any other dump and nothing is staged on disk. The server forks to produce it, the way it does for a background save. A server holds one dataset, so a source has a single database, `instance`; a `databases` label is ignored.

The client is signed in with the password from the container's environment, handed over in the client's own `REDISCLI_AUTH` variable rather than on a command line:

| Engine | Password read from |
|--------|--------------------|
| Redis | `REDIS_PASSWORD` |
| Valkey | `VALKEY_PASSWORD`, then `REDIS_PASSWORD` |

The official images set none, which leaves the server reachable without a password from inside its container. Use the `user` label for an ACL user and `password-env` for another variable, as for MySQL.

A restore cannot send an RDB to a running server, so the agent starts a throwaway server of the container's own image on a loopback port (`redis-server` or `valkey-server`), loads the dump into it, makes the real server its replica until the full sync has replaced its dataset, and then promotes it back to a master. Nothing restarts, and an append-only file is rewritten from the restored data. It needs:

- `redis-server` or `valkey-server` in the container, next to the client (the official images have both), and free space for the RDB in the container's temporary directory;
- an account allowed to run `ROLE`, `REPLICAOF`, and `CONFIG GET` and `CONFIG SET` for `masterauth`;
- a server that is a master: restoring into a replica is refused, since its master would overwrite the result.

The loopback of a pod is shared by all its containers, so the throwaway server is protected by a random password. It is written to a config file in a directory only the restore can read and removed with it, and the real server is handed it as its `masterauth` on the client's stdin, so it is on no command line. The real server gets its own `masterauth` back once it is a master again.

A restore replaces every key. A restore that fails while the sync runs can leave the dataset partly replaced, and a Redis that is only a cache is often better left to refill itself. If the server cannot be promoted back to a master, the restore retries and then fails with a message saying that it may be left as a replica: run `REPLICAOF NO ONE` on it and check its `masterauth`.

### InfluxDB

InfluxDB writes its backup to a directory, not to stdout. The agent therefore runs the backup into a scratch directory in the container's temporary directory, streams it out as a tar, and removes the directory again whatever happens. Plan for free space in the container roughly the size of the data. Neither the tar nor the scratch directory is written anywhere on the agent's host.

**InfluxDB 2** (`influxdb`) dumps the whole instance with `influx backup`, so a source has a single database, `instance`. The `influx` client reads the operator token from `INFLUX_TOKEN`, which the agent sets from the first of `DOCKER_INFLUXDB_INIT_ADMIN_TOKEN`, `INFLUXDB_TOKEN` and `INFLUX_TOKEN` in the container's environment, or the variable `password-env` names. Assimilate never stores it, and it is on no command line. A restore runs `influx restore --full`, which replaces everything, users and tokens included, and needs the operator token.

**InfluxDB 1** (`influxdb1`) dumps one database at a time with `influxd backup -portable -database <name>`, so a database can be restored next to the original. The databases are listed with `SHOW DATABASES`, without the server's own `_internal`. A server with authentication on is signed in as the account from the `user` label, or from `INFLUXDB_ADMIN_USER` and `INFLUX_USERNAME`, with the password from `INFLUXDB_ADMIN_PASSWORD`, `INFLUX_PASSWORD` or the variable `password-env` names. `influxd` backs up over its local RPC port (8088), which the official image binds to `127.0.0.1`.

### Servers installed on the host

For a server installed on the host, the agent runs the server's own client programs itself, as an OS user from the settings, with that user's home directory and none of the agent's environment. The OS user signs in on its own, the way it would from a shell: Assimilate stores no database password. Each engine is turned on separately, and a host can run all of them.

The programs run with all of the OS user's groups, as a login would, so a socket, a password file or a client certificate the user reaches through a supplementary group (such as `ssl-cert`) works. On macOS they run in the user's primary group only.

#### PostgreSQL on the host

The agent runs `psql` and `pg_dump` itself, as the **OS user** in the settings (`postgres` by default), with that user's home directory. Assimilate stores no database password:

- Over the server's Unix socket, the default `peer` authentication lets the OS user in as the database role of the same name. The Debian, Ubuntu and RHEL packages set this up for `postgres`.
- A server that asks for a password reads it from the OS user's `~/.pgpass`, which `psql` and `pg_dump` consult on their own.

Set the **socket directory** and **port** when the server is not where the clients look by default (`/var/run/postgresql` and 5432 on most distributions). On a host with several PostgreSQL versions installed side by side, set the **programs directory** to the `bin` directory of the server's version (for example `/usr/lib/postgresql/16/bin`), since an older `pg_dump` refuses to dump a newer server.

#### MySQL and MariaDB on the host

The agent runs `mysqldump` and `mysql`, or `mariadb-dump` and `mariadb` for MariaDB, as the **OS user** in the settings (`root` by default):

- Over the server's Unix socket, `root` is let in as the account of the same name without a password on Debian and Ubuntu (`unix_socket` for MariaDB, `auth_socket` for MySQL).
- A server that asks for a password reads it from the OS user's `~/.my.cnf`, which the clients consult on their own:

    ```ini
    # ~root/.my.cnf, readable by root only (chmod 600)
    [client]
    user = backup
    password = "s3cret"
    ```

Set the **socket** when the server's socket is not where the clients look by default (`/run/mysqld/mysqld.sock` on most distributions). A **port** connects over TCP to `127.0.0.1` instead of the socket; set one or the other, since the settings refuse both. Set the **programs directory** when the clients are not on the agent's `PATH`.

#### MongoDB on the host

The agent runs `mongodump`, `mongorestore` and `mongosh` as the **OS user** in the settings (`root` by default). A server without access control is reached on `127.0.0.1` at the **port** (27017 when empty), without signing in.

A server with access control on is named by a **config file** of the database tools, which holds its connection string, account and password included. The tools read it themselves with `--config`, and the agent's `mongosh` scripts read its `uri` line; the file never leaves the host and Assimilate never reads it:

```yaml
# /etc/assimilate/mongodump.yaml, readable by the OS user only (chmod 600)
uri: 'mongodb://backup:s3cret@127.0.0.1:27017/?authSource=admin'
```

Percent-encode any character of the password a connection string reserves, such as `@`, `:` or `/`. The `uri` names the server, so the **port** is not used with a config file. Name no database in the `uri`'s path: each dump names its own.

## What a backup stores

Each database becomes one file, left uncompressed so borg can deduplicate it against the previous night's dump and compress it itself:

| Engine | Format | File |
|--------|--------|------|
| PostgreSQL | `pg_dump --format=custom` | `<database>.pgdump` |
| MySQL and MariaDB | the SQL script `mysqldump` or `mariadb-dump --single-transaction --routines --events --triggers` writes | `<database>.sql` |
| MongoDB | `mongodump --archive` of the one database | `<database>.mongoarchive` |
| SQLite | the SQL script `sqlite3 .dump` writes for the one file | `<file>.sqlite.sql` |
| Redis and Valkey | the RDB file of the whole server | `instance.rdb` |
| InfluxDB 2 | a tar of `influx backup` of the whole instance | `instance.influxbackup.tar` |
| InfluxDB 1 | a tar of `influxd backup -portable` of the one database | `<database>.influxbackup.tar` |

A MySQL or MariaDB dump reads a consistent snapshot of InnoDB tables without locking them, and names no database itself, so it can be loaded into any database. The server's own schemas (`mysql`, `sys`, `information_schema`, `performance_schema`) and MongoDB's `admin`, `config` and `local` are not dumped: they hold the server's accounts and settings, not an application's data.

The dumps sit below a directory per place they came from:

```text
/var/lib/assimilate/database-dumps/schedule-7/
├── databases.json
├── kubernetes/
│   └── shop/
│       └── orders-db/
│           ├── orders.pgdump
│           └── payments.pgdump
├── docker/
│   ├── gitea/
│   │   └── db/
│   │       └── gitea.pgdump
│   ├── wiki-db/
│   │   └── wiki.sql
│   └── chat-db/
│       └── rocket.mongoarchive
└── host/
    ├── postgresql/
    │   └── inventory.pgdump
    └── mariadb/
        └── crm.sql
```

A SQLite dump is named after the file's whole path, percent-encoded like any name that is not a safe path component: `/data/db.sqlite3` becomes `%2Fdata%2Fdb%2Esqlite3.sqlite.sql`. `databases.json` records, for every dump, the source, where it ran, the database and its format. The path is the same on every run of a schedule, so the dumps line up in an archive diff.

A run that dumps databases makes borg read special files as regular ones (`--read-special`), which is how the pipes are archived. For those runs the agent also excludes `/dev`, `/proc`, `/sys` and `/run`, so a broad backup source such as `/` does not hang on a device.

!!! warning "A failed dump fails the backup"
    If a dump fails halfway (the pod or container restarts, the dump program exits with an error, the connection drops) the agent stops borg before it can commit, and deletes any checkpoint archive borg wrote while stopping. The run fails with the reason. An archive never holds a truncated dump while reporting success.

If borg never opened a dump's pipe, usually because an exclude pattern matches the dump directory, the run ends with a warning naming the missing dumps.

## Restore a database

Restore a dump from the archive that holds it. Open the archive in the archive browser (a repository's **Archives** tab, an agent's or a schedule's **Backups** tab or the **Archives** page) and switch it from **Files** to its **Databases** tab. The view lists every dump in the archive: the database, the source it came from, where that source ran and its runtime. **Restore** on a row opens the restore dialog; it is shown to the users who may restore files.

![Database dumps in an archive](assets/screenshots/archive-databases.png)

Choose where the dump goes:

| Restore into | What happens |
|--------------|--------------|
| **Into a new database** (the default) | The agent creates a database of the name you give, `<database>_restored` unless you change it, and loads the dump into it. A database of that name that already exists is never written into: the restore fails instead. The database the dump came from is left alone, so you can compare the two or copy single rows back. The name may use letters, digits, `_` and `-`, up to 63 characters. For SQLite, the new database is a file next to the original with its extension: restoring `/data/db.sqlite3` as `db_restored` creates `/data/db_restored.sqlite3`. |
| **Replace `<database>`** | The agent drops what the dump holds from the database it came from and recreates it from the dump. Type the database's name to confirm. |

Redis, Valkey and InfluxDB 2 dump their whole server, so their dump can only **replace** it: the dialog offers no new database for them, and the server refuses a request for one. InfluxDB 1 restores a database next to the original like the SQL engines (`influxd restore -portable -db <database> -newdb <name>`), while replacing it drops the database first, because InfluxDB 1 refuses to restore into one that exists. A restore that then fails leaves the database missing until you restore it again.

The restore runs on the host the archive came from unless you name another one under **Restore with host**; that host must run the same source. The agent finds the source again by its name, the same way a backup does, so a pod or container replaced since the backup is no problem; a source that no longer runs on the host cannot be restored into. The source's runtime must be turned on for that host in the **Databases** section of its **Sources** tab.

The agent streams the dump straight out of the archive into the engine's restore client, without writing it to disk, as the same database user the backup dumped as.

For PostgreSQL:

```bash
# Into a new database
createdb <new database>
pg_restore --single-transaction --exit-on-error --no-owner --dbname=<new database>

# Replacing the original database
pg_restore --single-transaction --exit-on-error --clean --if-exists --dbname=<database>
```

`--single-transaction` makes a restore all or nothing: if any statement fails, nothing the restore did is kept, and the original database is exactly as it was. A restore into a new database that fails drops the new database again. A replaced database gets its objects back with their original owners, so the roles your application connects as keep their access. A new database is a copy to inspect: `--no-owner` makes the restoring user the owner of everything in it.

For MySQL and MariaDB (with `mysql` in place of `mariadb`), and MongoDB:

```bash
# Into a new database
mariadb -e 'CREATE DATABASE `<new database>`'
mariadb --database=<new database> < dump.sql
mongorestore --archive --nsInclude='<database>.*' --nsFrom='<database>.*' --nsTo='<new database>.*'

# Replacing the original database
mariadb --database=<database> < dump.sql
mongorestore --archive --nsInclude='<database>.*' --drop
```

For SQLite:

```bash
# Into a new database, next to the original
sqlite3 -bail /data/<new database>.sqlite3 < dump.sqlite.sql

# Replacing the original database
sqlite3 -bail /data/.scratch.db < dump.sqlite.sql
sqlite3 -bail /data/db.sqlite3 ".restore /data/.scratch.db"
```

A SQLite restore over the original loads the dump into a scratch file next to it first, then copies that into the database with `.restore`, which takes SQLite's own locks, so an application holding the file open sees the restored content. A dump that does not load leaves the database exactly as it was, and the scratch file is removed either way.

MySQL, MariaDB and MongoDB have no transaction that covers creating tables, so a restore that fails partway is not undone the way a PostgreSQL one is. A failed restore into a new database still drops the new database again. Replacing the original drops and recreates each table (MySQL, MariaDB) or collection (MongoDB) the dump holds one after the other, so a restore that fails partway leaves some replaced and others not; the dialog says so before you confirm.

A database restore is a restore run like a file restore: it is listed on the **Restores** tab of the [Activity Log](activity.md#restores) and its status follows the same states (see [Restore Status](restore.md#restore-status)).

!!! warning "Replacing a database"
    Replacing the original database removes every change made to the objects in the dump since the backup was taken. Restore into a new database first when in doubt.

!!! note "Privileges"
    For PostgreSQL, the database user needs the `CREATEDB` privilege to restore into a new database. To replace a database it must own the dumped objects or be a superuser, since it hands them back to their owners. The default `postgres` user can do both.

!!! note "Kubernetes 1.30 or later"
    A restore into a pod sends the dump over the exec stream and then tells the cluster the input has ended, which needs the `v5.channel.k8s.io` exec protocol of Kubernetes 1.30 and later. An older cluster cannot be told that, so the restore is reported as cut off.

To restore by hand instead, restore the dump file like any other file (see [Restore](restore.md)) and load it with the engine's client where the database runs:

```bash
# A cluster source
kubectl -n shop exec -i orders-db-0 -c postgres -- \
  pg_restore --clean --if-exists --no-owner --dbname=orders < orders.pgdump

# A container
docker exec -i gitea-db-1 pg_restore --clean --if-exists --no-owner -U postgres --dbname=gitea < gitea.pgdump

# PostgreSQL on the host
sudo -u postgres pg_restore --clean --if-exists --no-owner --dbname=inventory < inventory.pgdump

# MariaDB or MySQL in a container
docker exec -i wiki-db sh -c 'MYSQL_PWD="$MARIADB_ROOT_PASSWORD" mariadb -uroot --database=wiki' < wiki.sql

# MongoDB in a container
docker exec -i chat-db sh -c 'mongorestore -u "$MONGO_INITDB_ROOT_USERNAME" -p "$MONGO_INITDB_ROOT_PASSWORD" \
  --authenticationDatabase=admin --archive --drop' < rocket.mongoarchive

# SQLite in a container, into a copy next to the original
docker exec -i vaultwarden sqlite3 -bail /data/db_restored.sqlite3 < %2Fdata%2Fdb%2Esqlite3.sqlite.sql

# Redis or Valkey: stop the server, put the file where it keeps its dump, start it again
# (with appendonly off, since the server loads an append-only file in preference)
docker stop cache && docker cp instance.rdb cache:/data/dump.rdb && docker start cache

# InfluxDB 2 in a container, replacing the whole instance
docker exec -i metrics sh -c 'dir=$(mktemp -d) && tar -C "$dir" -xf - && influx restore --full "$dir"' < instance.influxbackup.tar

# InfluxDB 1 in a container, into a copy next to the original
docker exec -i influx1 sh -c 'dir=$(mktemp -d) && tar -C "$dir" -xf - \
  && influxd restore -portable -db telegraf -newdb telegraf_copy "$dir"' < telegraf.influxbackup.tar
```

## Grant the agent access

Exec into a database pod can read every row in it, so give the agent its own service account rather than a cluster-admin kubeconfig. The agent needs to list pods across the cluster and create exec sessions:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: assimilate-agent
  namespace: assimilate
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: assimilate-database-dumps
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["list"]
  - apiGroups: [""]
    resources: ["pods/exec"]
    verbs: ["create", "get"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: assimilate-database-dumps
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: assimilate-database-dumps
subjects:
  - kind: ServiceAccount
    name: assimilate-agent
    namespace: assimilate
```

To limit the agent to some namespaces, bind the same rules with a `Role` and `RoleBinding` in each of them instead. The scan then lists only the pods it may see.

The container engine's socket has no such scoping: whoever can use it controls every container on the host, which the agent, running as root, already does. The agent only lists the labelled containers and runs the database client programs in them.

!!! note "Long dumps"
    The kubelet closes an exec stream that carries no data for its stream idle timeout, four hours by default. A dump that stalls that long fails with a message saying the connection was cut. Container and host dumps have no such limit.
