// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  RUNTIME_LABELS,
  hostPortProblem,
  runtimeForm,
  runtimeSummaries,
  runtimesFromForm,
  sourceKey,
  sourceName,
  sourcePlace,
} from './databaseRuntimes'
import type { AgentDatabaseSourceResponse, DatabaseRuntimes } from '../types/generated'

const NONE: DatabaseRuntimes = {
  kubernetes: false,
  kubeconfig_path: null,
  docker: false,
  docker_socket: null,
  host: false,
  host_os_user: null,
  host_socket_dir: null,
  host_port: null,
  host_bin_dir: null,
}

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

  it('names every runtime', () => {
    expect(Object.values(RUNTIME_LABELS)).toEqual(['Kubernetes', 'Docker', 'Host'])
  })
})
