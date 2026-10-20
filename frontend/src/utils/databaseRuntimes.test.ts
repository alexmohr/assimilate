// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  ENGINE_LABELS,
  NO_RUNTIMES,
  RUNTIME_LABELS,
  formPortProblem,
  hostPortProblem,
  runtimeForm,
  runtimeSummaries,
  runtimesFromForm,
  sourceKey,
  sourceName,
  sourcePlace,
} from './databaseRuntimes'
import type { AgentDatabaseSourceResponse, DatabaseRuntimes } from '../types/generated'

const NONE: DatabaseRuntimes = NO_RUNTIMES

function source(overrides: Partial<AgentDatabaseSourceResponse>): AgentDatabaseSourceResponse {
  return {
    runtime: 'kubernetes',
    namespace: 'shop',
    name: 'orders-db',
    included: true,
    kind: 'cnpg_cluster',
    engine: 'postgresql',
    pod: 'orders-db-2',
    container: 'postgres',
    databases: [],
    last_error: null,
    last_scanned_at: null,
    ...overrides,
  }
}

describe('runtime settings', () => {
  it('round-trip through the form, empty text becoming the defaults', () => {
    const saved: DatabaseRuntimes = {
      kubernetes: true,
      kubeconfig_path: '/etc/assimilate/kubeconfig',
      docker: true,
      docker_socket: null,
      host: true,
      host_os_user: 'pgadmin',
      host_socket_dir: null,
      host_port: 5433,
      host_bin_dir: '/usr/lib/postgresql/16/bin',
      host_mysql: {
        enabled: true,
        engine: 'mariadb',
        os_user: 'mysql',
        socket: '/run/mysqld/mysqld.sock',
        port: 3307,
        bin_dir: null,
      },
      host_mongodb: {
        enabled: true,
        os_user: null,
        port: 27018,
        config_file: '/etc/assimilate/mongodump.yaml',
        bin_dir: '/opt/mongodb/bin',
      },
    }
    expect(runtimesFromForm(runtimeForm(saved))).toEqual(saved)
    expect(runtimeForm(NONE).hostPort).toBe('')

    const typed = { ...runtimeForm(NONE), dockerSocket: '  ', hostPort: ' 70000 ' }
    expect(runtimesFromForm(typed)).toMatchObject({ docker_socket: null, host_port: 70000 })
  })

  it('take a port only as a whole number in range, or empty for the default', () => {
    expect(hostPortProblem('')).toBeNull()
    expect(hostPortProblem(' 5433 ')).toBeNull()
    expect(hostPortProblem('65535')).toBeNull()
    for (const typed of ['abc', '54.3', '0', '65536', '-1', '5e3']) {
      expect(hostPortProblem(typed)).toContain('whole number from 1 to 65535')
    }
  })

  it('summarize what each runtime does', () => {
    expect(runtimeSummaries(NONE).map((s) => s.value)).toEqual([
      'Not searched',
      'Not searched',
      'Not dumped',
    ])

    const on = runtimeSummaries({
      ...NONE,
      kubernetes: true,
      docker: true,
      docker_socket: '/run/podman/podman.sock',
      host: true,
    })
    expect(on[0]).toEqual({
      label: 'Kubernetes cluster',
      value: 'Searched with the agent default kubeconfig',
      mono: false,
    })
    expect(on[1]).toEqual({
      label: 'Docker or Podman containers',
      value: '/run/podman/podman.sock',
      mono: true,
    })
    expect(on[2]?.value).toBe('Dumped as postgres')

    const host = runtimeSummaries({
      ...NONE,
      host: true,
      host_os_user: 'pgadmin',
      host_socket_dir: '/var/run/postgresql',
      host_port: 5433,
      host_bin_dir: '/opt/pg/bin',
    })
    expect(host[2]?.value).toBe(
      'Dumped as pgadmin through /var/run/postgresql on port 5433 with /opt/pg/bin',
    )
  })
})

describe('host MySQL, MariaDB and MongoDB', () => {
  it('get a summary line once they are turned on', () => {
    const lines = runtimeSummaries({
      ...NONE,
      host_mysql: {
        ...NONE.host_mysql,
        enabled: true,
        engine: 'mariadb',
        socket: '/run/mysqld/mysqld.sock',
        port: 3307,
        bin_dir: '/opt/mariadb/bin',
      },
      host_mongodb: {
        ...NONE.host_mongodb,
        enabled: true,
        os_user: 'mongodb',
        port: 27018,
        bin_dir: '/opt/mongodb/bin',
      },
    })
    expect(lines.map((line) => line.label).slice(3)).toEqual([
      'MariaDB on this host',
      'MongoDB on this host',
    ])
    expect(lines[3]?.value).toBe(
      'Dumped as root through /run/mysqld/mysqld.sock on port 3307 with /opt/mariadb/bin',
    )
    expect(lines[4]?.value).toBe(
      'Dumped as mongodb on port 27018 with the tools in /opt/mongodb/bin',
    )

    const named = runtimeSummaries({
      ...NONE,
      host_mongodb: {
        ...NONE.host_mongodb,
        enabled: true,
        port: 27018,
        config_file: '/etc/assimilate/mongodump.yaml',
      },
    })
    expect(named[3]?.value).toBe(
      'Dumped as root with the server named in /etc/assimilate/mongodump.yaml',
    )
  })

  it('have their ports checked only while they are on', () => {
    const form = {
      ...runtimeForm(NONE),
      hostPort: 'x',
      hostMysqlPort: '99999',
      hostMongodbPort: 'abc',
    }
    expect(formPortProblem(form)).toBeNull()
    expect(formPortProblem({ ...form, hostMysql: true, hostMysqlEngine: 'mariadb' })).toBe(
      'The MariaDB port must be a whole number from 1 to 65535, not "99999".',
    )
    expect(formPortProblem({ ...form, hostMongodb: true })).toContain('The MongoDB port')
    expect(formPortProblem({ ...form, host: true })).toContain('The PostgreSQL port')
    expect(formPortProblem({ ...form, hostMongodb: true, hostMongodbPort: '27018' })).toBeNull()
  })

  it('are sources named after their engine', () => {
    const host = (name: string) => sourceName({ runtime: 'host', namespace: '', name })
    expect(host('mariadb')).toBe('MariaDB on this host')
    expect(host('mongodb')).toBe('MongoDB on this host')
    expect(host('unknown')).toBe('PostgreSQL on this host')
  })
})

describe('sources', () => {
  it('are named and placed the way their runtime calls them', () => {
    const pod = source({})
    expect([sourceKey(pod), sourceName(pod), sourcePlace(pod)]).toEqual([
      'kubernetes:shop/orders-db',
      'shop/orders-db',
      'orders-db-2 (postgres)',
    ])

    const loose = source({ runtime: 'docker', namespace: '', name: 'wiki-db', pod: 'wiki-db' })
    expect([sourceName(loose), sourcePlace(loose)]).toEqual(['wiki-db', 'container wiki-db'])
    expect(sourceName(source({ runtime: 'docker', namespace: 'shop', name: 'db' }))).toBe('shop/db')

    const host = source({ runtime: 'host', namespace: '', name: 'postgresql', pod: '' })
    expect([sourceName(host), sourcePlace(host)]).toEqual([
      'PostgreSQL on this host',
      'as postgres',
    ])
    expect(sourcePlace({ ...host, pod: '/run/pg' })).toBe('as postgres through /run/pg')
  })

  it('have no place before any scan has seen them', () => {
    expect(sourcePlace(source({ pod: null }))).toBeNull()
  })

  it('names every engine', () => {
    expect(Object.values(ENGINE_LABELS)).toEqual(['PostgreSQL', 'MySQL', 'MariaDB', 'MongoDB'])
  })

  it('names every runtime', () => {
    expect(Object.values(RUNTIME_LABELS)).toEqual(['Kubernetes', 'Docker', 'Host'])
  })
})
