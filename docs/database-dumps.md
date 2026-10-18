<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Database Dumps

An agent can dump PostgreSQL databases into the same archive as the rest of a backup. It finds them in three places:

- **a Kubernetes cluster**: `CloudNativePG` clusters and annotated pods, dumped inside their own pod;
- **Docker or Podman containers**: containers carrying a label, dumped inside the container;
- **the host itself**: a PostgreSQL server installed on the agent's host, dumped by running `pg_dump` locally.

Every dump is streamed straight into `borg create`: nothing is written to the agent's disk first, so a 200 GB database needs no 200 GB of scratch space.

Like [VM snapshots](vm-snapshots.md), the settings belong to the **host**, because the credentials and the dump directory live there. A schedule only opts in.

## Prerequisites

- An [agent](agents.md), running as root like any agent that backs up a whole system.
- For a cluster: credentials that may list pods and exec into them. See [Grant the agent access](#grant-the-agent-access).
- For containers: the engine's API socket on the agent's host (Docker's `/var/run/docker.sock`, or Podman's `/run/podman/podman.sock` with `podman.socket` enabled).
- `pg_dump` and `psql` where the dump runs, and `pg_restore`, `createdb` and `dropdb` there to restore one from Assimilate: inside the database container for a cluster or a container (every official PostgreSQL image and every `CloudNativePG` image has them), or on the host for the host's own server.

## Configure a host

Open the agent, then **Settings → Databases**. Only admins see this pane: its settings name the host's engine socket, OS user and directories.

![The Databases settings pane, listing a host's database sources and their databases](assets/screenshots/agent-databases.png)

1. Turn on **Allow schedules to dump databases**. Like the VM setting, this is a permission rather than an action: nothing is dumped until a schedule asks for it.
2. Choose **which sources** to dump. **All except excluded** dumps every source the agent finds, including one deployed after the last scan; **Only selected** dumps nothing until you pick the sources you want.
3. Turn on each place the agent should look, and fill in how it reaches it:
    - **Search the Kubernetes cluster**, with the **kubeconfig**, a path to a file on the agent's host. Leave it empty to use the in-cluster service account, or `$KUBECONFIG` and `~/.kube/config`. The file never leaves the host.

        To reach the API server through an HTTP or HTTPS proxy, set the cluster's `proxy-url` in the kubeconfig, or `HTTPS_PROXY` in the agent's environment. `NO_PROXY` is not consulted, so leave `HTTPS_PROXY` unset for the agent if the cluster should be reached directly.
    - **Search Docker or Podman containers**, with the **engine socket**. Leave it empty for `$DOCKER_HOST`, or `/var/run/docker.sock`.
    - **Dump PostgreSQL on this host**, with the **OS user** the dump runs as (`postgres` when empty), and, when the server is not where the clients look by default, its **socket directory**, **port** and the **programs directory** holding the matching `pg_dump`. See [The host's own server](#the-hosts-own-server).
4. Set the **dump directory**, an absolute path on the agent's host. It holds one named pipe per database while a backup runs, and the archive stores the dumps under it. It may not be below `/dev`, `/proc`, `/sys` or `/run`.
5. Click **Scan for databases**. While the settings form is open, the scan searches with the settings in the form rather than the saved ones, so you can test them before saving. When one place cannot be searched but another can, the scan shows what it found together with why the other failed, and keeps the sources it had already found in the failed place rather than dropping them.

Every place you turn on must be reachable when a backup runs: a backup whose cluster or container engine cannot be reached fails with the reason, rather than leaving databases out.

Then turn on **Dump databases** in a schedule's **Advanced** tab.

## How sources are found

The agent groups what it finds into **sources**, one per database workload:

| Found as | Where | Recognised by | Dumped from |
|----------|-------|---------------|-------------|
| `CloudNativePG` | cluster | the `cnpg.io/cluster` label | a replica when the cluster has one, so a nightly dump does not load the primary; the primary otherwise |
| Annotated pod | cluster | the `assimilate.alexmohr.github.io/database: postgresql` annotation | the first running pod of the workload |
| Labelled container | Docker or Podman | the `assimilate.alexmohr.github.io/database=postgresql` label on a running container | the first container of the Compose service |
| Host server | the host | turning on **Dump PostgreSQL on this host** | the server itself |

A source is named after the workload, not after a pod or container, so it keeps its name and your choice across rollouts:

- a cluster source by its namespace and workload (`shop/orders-db`); the pods of one Deployment are one source;
- a container by its Compose project and service (`gitea/db`), or by its name when it is not part of a Compose project; the containers of one scaled service are one source;
- the host's own server as **PostgreSQL on this host**.

### Pods and containers

An annotated pod or a labelled container can say more about itself, with the same keys:

| Annotation or label | Meaning | Default |
|---------------------|---------|---------|
| `assimilate.alexmohr.github.io/database` | The engine. Only `postgresql` is supported. | required |
| `assimilate.alexmohr.github.io/container` | (pods only) The container to run `pg_dump` in | the pod's first container |
| `assimilate.alexmohr.github.io/user` | The role to connect as | `postgres` |
| `assimilate.alexmohr.github.io/databases` | A comma-separated list of databases to dump | every database that accepts connections |

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

`pg_dump` connects over the pod's or container's local socket without a password, so the role must be allowed to connect locally. That holds for the `postgres` superuser in the official image and in `CloudNativePG`.

### The host's own server

For a server installed on the host, the agent runs `psql` and `pg_dump` itself, as the **OS user** in the settings (`postgres` by default), with that user's home directory. Assimilate stores no database password:

- Over the server's Unix socket, the default `peer` authentication lets the OS user in as the database role of the same name. The Debian, Ubuntu and RHEL packages set this up for `postgres`.
- A server that asks for a password reads it from the OS user's `~/.pgpass`, which `psql` and `pg_dump` consult on their own.

The programs run with the OS user's own user and primary group, without its supplementary groups. If the socket directory or `~/.pgpass` is readable only through a supplementary group, make it readable through the user's primary group instead, or pick an OS user whose primary group can read it.

Set the **socket directory** and **port** when the server is not where the clients look by default (`/var/run/postgresql` and 5432 on most distributions). On a host with several PostgreSQL versions installed side by side, set the **programs directory** to the `bin` directory of the server's version (for example `/usr/lib/postgresql/16/bin`), since an older `pg_dump` refuses to dump a newer server.

## What a backup stores

Each database becomes one file in custom format (`pg_dump --format=custom`), left uncompressed so borg can deduplicate it against the previous night's dump and compress it itself. The dumps sit below a directory per place they came from:

```text
/var/lib/assimilate/database-dumps/schedule-7/
├── databases.json
├── kubernetes/
│   └── shop/
│       └── orders-db/
│           ├── orders.pgdump
│           └── payments.pgdump
├── docker/
│   └── gitea/
│       └── db/
│           └── gitea.pgdump
└── host/
    └── postgresql/
        └── inventory.pgdump
```

`databases.json` records, for every dump, the source, where it ran, the database and its format. The path is the same on every run of a schedule, so the dumps line up in an archive diff.

A run that dumps databases makes borg read special files as regular ones (`--read-special`), which is how the pipes are archived. For those runs the agent also excludes `/dev`, `/proc`, `/sys` and `/run`, so a broad backup source such as `/` does not hang on a device.

!!! warning "A failed dump fails the backup"
    If a dump fails halfway (the pod or container restarts, `pg_dump` exits with an error, the connection drops) the agent stops borg before it can commit, and deletes any checkpoint archive borg wrote while stopping. The run fails with the reason. An archive never holds a truncated dump while reporting success.

If borg never opened a dump's pipe, usually because an exclude pattern matches the dump directory, the run ends with a warning naming the missing dumps.

## Restore a database

Restore a dump from the archive that holds it. Open the archive in the archive browser (a repository's **Archives** tab, a schedule's **Backups** tab or the **Archives** page) and switch it from **Files** to **Databases**. The view lists every dump in the archive: the database, the source it came from, where that source ran and its runtime. **Restore** on a row opens the restore dialog; it is shown to the users who may restore files.

![Database dumps in an archive](assets/screenshots/archive-databases.png)

Choose where the dump goes:

| Restore into | What happens |
|--------------|--------------|
| **Into a new database** (the default) | The agent creates a database of the name you give, `<database>_restored` unless you change it, and loads the dump into it. The database the dump came from is left alone, so you can compare the two or copy single rows back. The name may use letters, digits, `_` and `-`, up to 63 characters. |
| **Replace `<database>`** | The agent drops what the dump holds from the database it came from and recreates it from the dump. Type the database's name to confirm. |

The restore runs on the host the archive came from unless you name another one under **Restore with host**; that host must run the same source. The agent finds the source again by its name, the same way a backup does, so a pod or container replaced since the backup is no problem; a source that no longer runs on the host cannot be restored into. The source's runtime must be turned on for that host in **Settings → Databases**.

The agent streams the dump straight out of the archive into `pg_restore`, without writing it to disk, as the same database user the backup dumped as:

```bash
# Into a new database
createdb <new database>
pg_restore --single-transaction --exit-on-error --no-owner --dbname=<new database>

# Replacing the original database
pg_restore --single-transaction --exit-on-error --clean --if-exists --dbname=<database>
```

`--single-transaction` makes a restore all or nothing: if any statement fails, nothing the restore did is kept, and the original database is exactly as it was. A restore into a new database that fails drops the new database again. A replaced database gets its objects back with their original owners, so the roles your application connects as keep their access. A new database is a copy to inspect: `--no-owner` makes the restoring user the owner of everything in it.

A database restore is a restore run like a file restore: it is listed on the **Restores** tab of the [Activity Log](activity.md#restores) and its status follows the same states (see [Restore Status](restore.md#restore-status)).

!!! warning "Replacing a database"
    Replacing the original database removes every change made to the objects in the dump since the backup was taken. Restore into a new database first when in doubt.

!!! note "Privileges"
    The database user needs the `CREATEDB` privilege to restore into a new database. To replace a database it must own the dumped objects or be a superuser, since it hands them back to their owners. The default `postgres` user can do both.

!!! note "Kubernetes 1.30 or later"
    A restore into a pod sends the dump over the exec stream and then tells the cluster the input has ended, which needs the `v5.channel.k8s.io` exec protocol of Kubernetes 1.30 and later. An older cluster cannot be told that, so the restore is reported as cut off.

To restore by hand instead, restore the dump file like any other file (see [Restore](restore.md)) and load it with `pg_restore` where the database runs:

```bash
# A cluster source
kubectl -n shop exec -i orders-db-0 -c postgres -- \
  pg_restore --clean --if-exists --no-owner --dbname=orders < orders.pgdump

# A container
docker exec -i gitea-db-1 pg_restore --clean --if-exists --no-owner -U postgres --dbname=gitea < gitea.pgdump

# The host's own server
sudo -u postgres pg_restore --clean --if-exists --no-owner --dbname=inventory < inventory.pgdump
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

The container engine's socket has no such scoping: whoever can use it controls every container on the host, which the agent, running as root, already does. The agent only lists the labelled containers and runs the PostgreSQL client programs in them.

!!! note "Long dumps"
    The kubelet closes an exec stream that carries no data for its stream idle timeout, four hours by default. A dump that stalls that long fails with a message saying the connection was cut. Container and host dumps have no such limit.
