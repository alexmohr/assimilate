// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { apiClient } from '../api/client'
import AgentDatabasesCard from './AgentDatabasesCard.vue'
import { NO_RUNTIMES } from '../utils/databaseRuntimes'
import BaseSegmented from './BaseSegmented.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'
import type {
  AgentDatabaseSourceResponse,
  AgentDatabasesResponse,
  DatabaseRuntimes,
} from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const AGENT = { hostname: 'k3s-node-01', domain: null } as unknown as AgentRow

const CLUSTER: DatabaseRuntimes = {
  ...NO_RUNTIMES,
  kubernetes: true,
  kubeconfig_path: '/etc/assimilate/kubeconfig',
}

function source(overrides: Partial<AgentDatabaseSourceResponse> = {}): AgentDatabaseSourceResponse {
  return {
    runtime: 'kubernetes',
    namespace: 'shop',
    name: 'orders-db',
    included: true,
    kind: 'cnpg_cluster',
    engine: 'postgresql',
    pod: 'orders-db-2',
    container: 'postgres',
    databases: ['orders', 'payments'],
    last_error: null,
    last_scanned_at: '2026-10-14T02:00:00Z',
    ...overrides,
  }
}

function response(
  sources: AgentDatabaseSourceResponse[] = [source()],
  overrides: Partial<AgentDatabasesResponse['settings']> = {},
): AgentDatabasesResponse {
  return {
    settings: {
      enabled: true,
      dump_dir: '/var/lib/assimilate/database-dumps',
      runtimes: CLUSTER,
      selection: 'all',
      ...overrides,
    },
    sources,
  }
}

async function mount(props: Record<string, unknown> = {}) {
  const wrapper = renderWithPlugins(AgentDatabasesCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
  await flushPromises()
  return wrapper
}

describe('AgentDatabasesCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.put).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.post).mockResolvedValue({ data: response() } as never)
  })

  it('summarizes the host settings and lists each source with its databases', async () => {
    const text = (await mount()).text()
    expect(text).toContain('Allowed')
    expect(text).toContain('/etc/assimilate/kubeconfig')
    expect(text).toContain('/var/lib/assimilate/database-dumps')
    expect(text).toContain('shop/orders-db')
    expect(text).toContain('orders-db-2 (postgres)')
    expect(text).toContain('CloudNativePG')
    expect(text).toContain('orders, payments')
    expect(apiClient.get).toHaveBeenCalledWith('/agents/k3s-node-01/databases', expect.anything())
  })

  it('says when no kubeconfig is set and when a source was never scanned', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response(
        [
          source({
            kind: null,
            engine: null,
            pod: null,
            container: null,
            databases: [],
            last_scanned_at: null,
            last_error: 'not found in the last scan',
          }),
        ],
        { runtimes: { ...CLUSTER, kubeconfig_path: null }, selection: 'selected' },
      ),
    } as never)
    const text = (await mount()).text()
    expect(text).toContain('Searched with the agent default kubeconfig')
    expect(text).toContain('Only the sources selected below')
    expect(text).toContain('Not scanned')
    expect(text).toContain('None found')
    expect(text).toContain('Never')
    expect(text).toContain('not found in the last scan')
    expect(text).toContain('Turn a source on to dump its databases')
  })

  it('shows an empty state before the first scan', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: response([]) } as never)
    expect((await mount()).text()).toContain('No database sources found')
  })

  it('reports a failed load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    expect((await mount()).text()).toContain('boom')
  })

  it('saves the settings, sending an empty kubeconfig as null', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find('#db-kubeconfig').setValue('   ')
    await wrapper.find('#db-dump-dir').setValue(' /srv/dumps ')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      {
        enabled: true,
        selection: 'all',
        dump_dir: '/srv/dumps',
        runtimes: { ...CLUSTER, kubeconfig_path: null },
      },
      expect.anything(),
    )
  })

  it('asks how to reach containers and the host server once they are turned on', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#db-docker-socket').exists()).toBe(false)
    expect(wrapper.find('#db-host-user').exists()).toBe(false)

    const toggle = (label: string): void => {
      const found = wrapper.findAllComponents(ToggleSwitch).find((t) => t.props('label') === label)
      found?.vm.$emit('update:modelValue', true)
    }
    toggle('Search Docker or Podman containers')
    toggle('Dump PostgreSQL on this host')
    await flushPromises()
    await wrapper.find('#db-docker-socket').setValue('/run/podman/podman.sock')
    await wrapper.find('#db-host-user').setValue('pgadmin')
    await wrapper.find('#db-host-socket').setValue('/var/run/postgresql')
    await wrapper.find('#db-host-port').setValue('5433')
    await wrapper.find('#db-host-bin').setValue('/usr/lib/postgresql/16/bin')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      expect.objectContaining({
        runtimes: {
          ...CLUSTER,
          docker: true,
          docker_socket: '/run/podman/podman.sock',
          host: true,
          host_os_user: 'pgadmin',
          host_socket_dir: '/var/run/postgresql',
          host_port: 5433,
          host_bin_dir: '/usr/lib/postgresql/16/bin',
        },
      }),
      expect.anything(),
    )
  })

  it('asks how to reach MySQL, MariaDB and MongoDB on the host once they are turned on', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#db-host-mysql-socket').exists()).toBe(false)
    expect(wrapper.find('#db-host-mongodb-config').exists()).toBe(false)

    const toggle = (label: string): void => {
      const found = wrapper.findAllComponents(ToggleSwitch).find((t) => t.props('label') === label)
      found?.vm.$emit('update:modelValue', true)
    }
    toggle('Dump MySQL or MariaDB on this host')
    toggle('Dump MongoDB on this host')
    await flushPromises()
    wrapper
      .findAllComponents(BaseSegmented)
      .find((s) => s.props('label') === 'Which server is installed')
      ?.vm.$emit('update:modelValue', 'mariadb')
    await wrapper.find('#db-host-mysql-user').setValue('mysql')
    await wrapper.find('#db-host-mysql-socket').setValue('/run/mysqld/mysqld.sock')
    await wrapper.find('#db-host-mysql-port').setValue('3307')
    await wrapper.find('#db-host-mysql-bin').setValue('/opt/mariadb/bin')
    await wrapper.find('#db-host-mongodb-user').setValue('mongodb')
    await wrapper.find('#db-host-mongodb-config').setValue('/etc/assimilate/mongodump.yaml')
    await wrapper.find('#db-host-mongodb-port').setValue('banana')

    await clickSectionButton(wrapper, 'Save')
    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('The MongoDB port must be a whole number')

    await wrapper.find('#db-host-mongodb-port').setValue('')
    await clickSectionButton(wrapper, 'Save')
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      expect.objectContaining({
        runtimes: {
          ...CLUSTER,
          host_mysql: {
            ...CLUSTER.host_mysql,
            enabled: true,
            engine: 'mariadb',
            os_user: 'mysql',
            socket: '/run/mysqld/mysqld.sock',
            port: 3307,
            bin_dir: '/opt/mariadb/bin',
          },
          host_mongodb: {
            ...CLUSTER.host_mongodb,
            enabled: true,
            os_user: 'mongodb',
            config_file: '/etc/assimilate/mongodump.yaml',
          },
        },
      }),
      expect.anything(),
    )
  })

  it('stops searching the cluster once its switch is turned off', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#db-kubeconfig').exists()).toBe(true)

    wrapper
      .findAllComponents(ToggleSwitch)
      .find((t) => t.props('label') === 'Search the Kubernetes cluster')
      ?.vm.$emit('update:modelValue', false)
    await flushPromises()
    expect(wrapper.find('#db-kubeconfig').exists()).toBe(false)
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      expect.objectContaining({
        runtimes: expect.objectContaining({ kubernetes: false }),
      }),
      expect.anything(),
    )
  })

  it('names container and host sources the way their runtime does', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response(
        [
          source({
            runtime: 'docker',
            namespace: '',
            name: 'wiki-db',
            kind: 'container',
            pod: 'wiki-db',
            container: '',
          }),
          source({
            runtime: 'host',
            namespace: '',
            name: 'postgresql',
            kind: 'host',
            pod: '',
            container: 'postgres',
          }),
        ],
        { runtimes: { ...CLUSTER, docker: true, host: true } },
      ),
    } as never)
    const text = (await mount()).text()
    expect(text).toContain('container wiki-db')
    expect(text).toContain('Labelled container')
    expect(text).toContain('PostgreSQL on this host')
    expect(text).toContain('as postgres')
    expect(text).toContain('Host server')
    expect(text).toContain('Dumped as postgres')
  })

  it('says which engine each source runs', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response(
        [
          source({}),
          source({
            runtime: 'docker',
            namespace: 'wiki',
            name: 'db',
            kind: 'container',
            engine: 'mariadb',
            pod: 'wiki-db-1',
            container: '',
          }),
          source({ name: 'chat', kind: 'annotated', engine: 'mongodb' }),
          source({ name: 'fresh', kind: null, engine: null }),
        ],
        { runtimes: { ...CLUSTER, docker: true } },
      ),
    } as never)
    const rows = (await mount()).findAll('tbody tr').map((row) => row.text())
    expect(rows[0]).toContain('PostgreSQL')
    expect(rows[1]).toContain('MariaDB')
    expect(rows[2]).toContain('MongoDB')
    expect(rows[3]).toContain('Not scanned')
    expect(rows[3]).not.toMatch(/PostgreSQL|MariaDB|MongoDB|MySQL/)
  })

  it('keeps the form open and says why when saving fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('the dump directory must not be below'))
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(wrapper.text()).toContain('the dump directory must not be below')
    expect(wrapper.find('#db-dump-dir').exists()).toBe(true)
  })

  it('refuses a port that is not a number instead of sending the default', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    wrapper
      .findAllComponents(ToggleSwitch)
      .find((t) => t.props('label') === 'Dump PostgreSQL on this host')
      ?.vm.$emit('update:modelValue', true)
    await flushPromises()
    await wrapper.find('#db-host-port').setValue('abc')

    await clickSectionButton(wrapper, 'Save')
    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('whole number from 1 to 65535')

    vi.mocked(apiClient.post).mockClear()
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('ignores the port of a host search that was turned off again', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    const hostToggle = () =>
      wrapper
        .findAllComponents(ToggleSwitch)
        .find((t) => t.props('label') === 'Dump PostgreSQL on this host')
    hostToggle()?.vm.$emit('update:modelValue', true)
    await flushPromises()
    await wrapper.find('#db-host-port').setValue('abc')
    hostToggle()?.vm.$emit('update:modelValue', false)
    await flushPromises()

    await clickSectionButton(wrapper, 'Scan for databases')
    expect(apiClient.post).toHaveBeenCalled()
    await clickSectionButton(wrapper, 'Save')
    expect(apiClient.put).toHaveBeenCalled()
    expect(wrapper.text()).not.toContain('whole number from 1 to 65535')
  })

  it('throws away an abandoned edit', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find('#db-dump-dir').setValue('/elsewhere')
    await clickSectionButton(wrapper, 'Cancel')
    expect(wrapper.text()).toContain('/var/lib/assimilate/database-dumps')
    expect(wrapper.text()).not.toContain('/elsewhere')
  })

  it('scans with the saved settings, or the ones in the open form', async () => {
    const wrapper = await mount()
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(apiClient.post).toHaveBeenLastCalledWith(
      '/agents/k3s-node-01/databases/scan',
      { runtimes: null },
      expect.anything(),
    )

    await startEditingSection(wrapper)
    await wrapper.find('#db-kubeconfig').setValue('/tmp/try-this')
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(apiClient.post).toHaveBeenLastCalledWith(
      '/agents/k3s-node-01/databases/scan',
      { runtimes: { ...CLUSTER, kubeconfig_path: '/tmp/try-this' } },
      expect.anything(),
    )
    // The form keeps what was typed.
    expect((wrapper.find('#db-kubeconfig').element as HTMLInputElement).value).toBe('/tmp/try-this')
  })

  it('says which runtime a partial scan could not search', async () => {
    const wrapper = await mount()
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: { ...response(), scan_error: 'the Docker socket is gone' },
    } as never)
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(wrapper.text()).toContain('the Docker socket is gone')

    vi.mocked(apiClient.post).mockResolvedValueOnce({ data: response() } as never)
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(wrapper.text()).not.toContain('the Docker socket is gone')
  })

  it('reports a failed scan', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('could not reach the cluster'))
    const wrapper = await mount()
    await clickSectionButton(wrapper, 'Scan for databases')
    expect(wrapper.text()).toContain('could not reach the cluster')
  })

  it('turns a source off, and says when that fails', async () => {
    const wrapper = await mount()
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', false)
    await flushPromises()
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/databases/sources',
      { runtime: 'kubernetes', namespace: 'shop', name: 'orders-db', included: false },
      expect.anything(),
    )

    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', true)
    await flushPromises()
    expect(wrapper.text()).toContain('agent offline')
  })

  it('saves a change to the permission and the selection mode', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', false)
    wrapper.findComponent({ name: 'BaseSegmented' }).vm.$emit('update:modelValue', 'selected')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      expect.objectContaining({ enabled: false, selection: 'selected' }),
      expect.anything(),
    )
  })

  it.each([
    ['allow database dumps', 'Each schedule decides whether its own runs include them'],
    ['how the agent reaches the cluster', 'running commands in a database pod is a privileged'],
    ['how the agent finds containers', 'assimilate.alexmohr.github.io/database=postgresql'],
    ["how the agent signs in to the host's server", 'stores no database password'],
    [
      "how the agent signs in to the host's MySQL or MariaDB",
      "reads it from that user's ~/.my.cnf",
    ],
    ["how the agent signs in to the host's MongoDB", 'only its path is stored here'],
    ['where the dumps are streamed from', 'holds one named pipe per database'],
  ])('explains %s when asked', async (label, text) => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find(`[aria-label="Help: ${label}"]`).trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain(text)
  })

  it('offers no scan for an imported host', async () => {
    const wrapper = await mount({ canEdit: false })
    expect(wrapper.text()).not.toContain('Scan for databases')
  })
})
