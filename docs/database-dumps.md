<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Database Dumps

An agent can dump the PostgreSQL databases running in its Kubernetes cluster into the same archive as the rest of a backup. Each dump runs inside the database's own pod and is streamed straight into `borg create`: nothing is written to the agent's disk first, so a 200 GB database needs no 200 GB of scratch space.

Like [VM snapshots](vm-snapshots.md), the settings belong to the **host**, because the cluster credentials and the dump directory live there. A schedule only opts in.

## Prerequisites

- An [agent](agents.md) that can reach the cluster's API server, either running inside the cluster with a service account or on a host with a kubeconfig.
- Credentials that may list pods and exec into them. See [Grant the agent access](#grant-the-agent-access).
- `pg_dump` and `psql` inside the database container. Every official PostgreSQL image and every `CloudNativePG` image has them.

## Configure a host

Open the agent, then **Settings → Databases**.

![The Databases settings pane, listing a cluster's database sources and their databases](assets/screenshots/agent-databases.png)

1. Turn on **Allow schedules to dump databases**. Like the VM setting, this is a permission rather than an action: nothing is dumped until a schedule asks for it.
2. Choose **which sources** to dump. **All except excluded** dumps every source the agent finds, including one deployed after the last scan; **Only selected** dumps nothing until you pick the sources you want.
3. Set the **kubeconfig**, a path to a file on the agent's host. Leave it empty to use the in-cluster service account, or `$KUBECONFIG` and `~/.kube/config`. The file never leaves the host.

   To reach the API server through an HTTP or HTTPS proxy, set the cluster's `proxy-url` in the kubeconfig, or `HTTPS_PROXY` in the agent's environment. `NO_PROXY` is not consulted, so leave `HTTPS_PROXY` unset for the agent if the cluster should be reached directly.
4. Set the **dump directory**, an absolute path on the agent's host. It holds one named pipe per database while a backup runs, and the archive stores the dumps under it. It may not be below `/dev`, `/proc`, `/sys` or `/run`.
5. Click **Scan cluster**. While the settings form is open, the scan tries the kubeconfig in the form rather than the saved one, so you can test a path before saving it.

Then turn on **Dump databases** in a schedule's **Advanced** tab.

## How sources are found

The agent lists every pod its credentials can see and groups them into **sources**, one per database workload:

| Found as | Recognised by | Dumped from |
|----------|---------------|-------------|
| `CloudNativePG` | the `cnpg.io/cluster` label | a replica when the cluster has one, so a nightly dump does not load the primary; the primary otherwise |
| Annotated pod | the `assimilate.alexmohr.github.io/database: postgresql` annotation | the first running pod of the workload |

A source is named after its namespace and its workload (`shop/orders-db`), not after a pod, so it keeps its name and your choice across rollouts. The pods of one Deployment are one source.

An annotated pod can say more about itself:

| Annotation | Meaning | Default |
|------------|---------|---------|
| `assimilate.alexmohr.github.io/database` | The engine. Only `postgresql` is supported. | required |
| `assimilate.alexmohr.github.io/container` | The container to run `pg_dump` in | the pod's first container |
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

`pg_dump` connects over the pod's local socket without a password, so the role must be allowed to connect locally. That holds for the `postgres` superuser in the official image and in `CloudNativePG`.

## What a backup stores

Each database becomes one file in custom format (`pg_dump --format=custom`), left uncompressed so borg can deduplicate it against the previous night's dump and compress it itself:

```text
/var/lib/assimilate/database-dumps/schedule-7/
├── databases.json
└── shop/
    └── orders-db/
        ├── orders.pgdump
        └── payments.pgdump
```

`databases.json` records, for every dump, the source, the pod it ran in, the database and its format. The path is the same on every run of a schedule, so the dumps line up in an archive diff.

A run that dumps databases makes borg read special files as regular ones (`--read-special`), which is how the pipes are archived. For those runs the agent also excludes `/dev`, `/proc`, `/sys` and `/run`, so a broad backup source such as `/` does not hang on a device.

!!! warning "A failed dump fails the backup"
    If a dump fails halfway (the pod restarts, `pg_dump` exits with an error, the connection drops) the agent stops borg before it can commit, and deletes any checkpoint archive borg wrote while stopping. The run fails with the reason. An archive never holds a truncated dump while reporting success.

If borg never opened a dump's pipe, usually because an exclude pattern matches the dump directory, the run ends with a warning naming the missing dumps.

## Restore a database

Restore the dump file from the archive like any other file (see [Restore](restore.md)), then load it with `pg_restore`:

```bash
kubectl -n shop exec -i orders-db-0 -c postgres -- \
  pg_restore --clean --if-exists --no-owner --dbname=orders < orders.pgdump
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

!!! note "Long dumps"
    The kubelet closes an exec stream that carries no data for its stream idle timeout, four hours by default. A dump that stalls that long fails with a message saying the connection was cut.
