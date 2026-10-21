// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { DatabaseSourceRef } from '../api/databases'
import type {
  AgentDatabaseSourceResponse,
  DatabaseEngine,
  DatabaseRuntime,
  DatabaseRuntimes,
  DatabaseSourceKind,
  MysqlEngine,
} from '../types/generated'

/**
 * A host's runtime settings as the form edits them: every text field stays
 * text until it is saved, where an empty one becomes the agent's default.
 */
export interface RuntimeForm {
  kubernetes: boolean
  kubeconfigPath: string
  docker: boolean
  dockerSocket: string
  host: boolean
  hostOsUser: string
  hostSocketDir: string
  hostPort: string
  hostBinDir: string
  hostMysql: boolean
  hostMysqlEngine: MysqlEngine
  hostMysqlOsUser: string
  hostMysqlSocket: string
  hostMysqlPort: string
  hostMysqlBinDir: string
  hostMongodb: boolean
  hostMongodbOsUser: string
  hostMongodbPort: string
  hostMongodbConfigFile: string
  hostMongodbBinDir: string
}

/** One line of the settings summary. */
export interface RuntimeSummary {
  label: string
  value: string
  /** Whether the value is a path, shown in the monospace face. */
  mono: boolean
}

/** The OS user host dumps run as when the settings name none. */
export const DEFAULT_HOST_OS_USER = 'postgres'

/** The OS user host MySQL, MariaDB and MongoDB dumps run as by default. */
export const DEFAULT_HOST_ROOT_USER = 'root'

export const MYSQL_ENGINE_LABELS: Record<MysqlEngine, string> = {
  mysql: 'MySQL',
  mariadb: 'MariaDB',
}

export const KIND_LABELS: Record<DatabaseSourceKind, string> = {
  cnpg_cluster: 'CloudNativePG',
  annotated: 'Annotated pod',
  container: 'Labelled container',
  host: 'Host server',
}

export const ENGINE_LABELS: Record<DatabaseEngine, string> = {
  postgresql: 'PostgreSQL',
  mysql: 'MySQL',
  mariadb: 'MariaDB',
  mongodb: 'MongoDB',
}

export const RUNTIME_LABELS: Record<DatabaseRuntime, string> = {
  kubernetes: 'Kubernetes',
  docker: 'Docker',
  host: 'Host',
}

/** A host that looks for databases nowhere, as a form starts out. */
export const NO_RUNTIMES: DatabaseRuntimes = {
  kubernetes: false,
  kubeconfig_path: null,
  docker: false,
  docker_socket: null,
  host: false,
  host_os_user: null,
  host_socket_dir: null,
  host_port: null,
  host_bin_dir: null,
  host_mysql: {
    enabled: false,
    engine: 'mysql',
    os_user: null,
    socket: null,
    port: null,
    bin_dir: null,
  },
  host_mongodb: { enabled: false, os_user: null, port: null, config_file: null, bin_dir: null },
}

export function runtimeForm(runtimes: DatabaseRuntimes): RuntimeForm {
  return {
    kubernetes: runtimes.kubernetes,
    kubeconfigPath: runtimes.kubeconfig_path ?? '',
    docker: runtimes.docker,
    dockerSocket: runtimes.docker_socket ?? '',
    host: runtimes.host,
    hostOsUser: runtimes.host_os_user ?? '',
    hostSocketDir: runtimes.host_socket_dir ?? '',
    hostPort: portText(runtimes.host_port),
    hostBinDir: runtimes.host_bin_dir ?? '',
    hostMysql: runtimes.host_mysql.enabled,
    hostMysqlEngine: runtimes.host_mysql.engine,
    hostMysqlOsUser: runtimes.host_mysql.os_user ?? '',
    hostMysqlSocket: runtimes.host_mysql.socket ?? '',
    hostMysqlPort: portText(runtimes.host_mysql.port),
    hostMysqlBinDir: runtimes.host_mysql.bin_dir ?? '',
    hostMongodb: runtimes.host_mongodb.enabled,
    hostMongodbOsUser: runtimes.host_mongodb.os_user ?? '',
    hostMongodbPort: portText(runtimes.host_mongodb.port),
    hostMongodbConfigFile: runtimes.host_mongodb.config_file ?? '',
    hostMongodbBinDir: runtimes.host_mongodb.bin_dir ?? '',
  }
}

function portText(port: number | null): string {
  return port === null ? '' : String(port)
}

function textOrNull(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

/**
 * The port typed into the form, or null for the default. The form checks it
 * with `hostPortProblem` first: anything that is not a number would otherwise
 * be sent as `null` and quietly fall back to the default.
 */
function portOrNull(value: string): number | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : Number(trimmed)
}

/**
 * Why the port typed into the form for `server` cannot be sent, or null when
 * it can: it is left empty for the default, or a whole number from 1 to 65535.
 */
export function hostPortProblem(value: string, server = 'PostgreSQL'): string | null {
  const trimmed = value.trim()
  if (trimmed === '') return null
  const port = Number(trimmed)
  return /^\d+$/.test(trimmed) && port >= 1 && port <= 65535
    ? null
    : `The ${server} port must be a whole number from 1 to 65535, not "${trimmed}".`
}

/**
 * What is wrong with the connection settings of a host server the form turns
 * on, if anything: a port that is not one, or a MySQL or MariaDB server given
 * both a socket and a port, which its client cannot use together.
 */
export function formProblem(form: RuntimeForm): string | null {
  const mysql = MYSQL_ENGINE_LABELS[form.hostMysqlEngine]
  if (form.hostMysql && form.hostMysqlSocket.trim() !== '' && form.hostMysqlPort.trim() !== '') {
    return `Give the ${mysql} server a socket or a port, not both.`
  }
  const checks: [boolean, string, string][] = [
    [form.host, form.hostPort, 'PostgreSQL'],
    [form.hostMysql, form.hostMysqlPort, mysql],
    [form.hostMongodb, form.hostMongodbPort, 'MongoDB'],
  ]
  return (
    checks
      .filter(([on]) => on)
      .map(([, value, server]) => hostPortProblem(value, server))
      .find((problem) => problem !== null) ?? null
  )
}

export function runtimesFromForm(form: RuntimeForm): DatabaseRuntimes {
  return {
    kubernetes: form.kubernetes,
    kubeconfig_path: textOrNull(form.kubeconfigPath),
    docker: form.docker,
    docker_socket: textOrNull(form.dockerSocket),
    host: form.host,
    host_os_user: textOrNull(form.hostOsUser),
    host_socket_dir: textOrNull(form.hostSocketDir),
    host_port: portOrNull(form.hostPort),
    host_bin_dir: textOrNull(form.hostBinDir),
    host_mysql: {
      enabled: form.hostMysql,
      engine: form.hostMysqlEngine,
      os_user: textOrNull(form.hostMysqlOsUser),
      socket: textOrNull(form.hostMysqlSocket),
      port: portOrNull(form.hostMysqlPort),
      bin_dir: textOrNull(form.hostMysqlBinDir),
    },
    host_mongodb: {
      enabled: form.hostMongodb,
      os_user: textOrNull(form.hostMongodbOsUser),
      port: portOrNull(form.hostMongodbPort),
      config_file: textOrNull(form.hostMongodbConfigFile),
      bin_dir: textOrNull(form.hostMongodbBinDir),
    },
  }
}

function searchSummary(
  label: string,
  on: boolean,
  path: string | null,
  fallback: string,
): RuntimeSummary {
  if (!on) return { label, value: 'Not searched', mono: false }
  if (path === null) return { label, value: `Searched with the ${fallback}`, mono: false }
  return { label, value: path, mono: true }
}

function hostSummary(runtimes: DatabaseRuntimes): RuntimeSummary {
  const label = 'PostgreSQL on this host'
  if (!runtimes.host) return { label, value: 'Not dumped', mono: false }
  const parts = [`Dumped as ${runtimes.host_os_user ?? DEFAULT_HOST_OS_USER}`]
  if (runtimes.host_socket_dir !== null) parts.push(`through ${runtimes.host_socket_dir}`)
  if (runtimes.host_port !== null) parts.push(`on port ${runtimes.host_port}`)
  if (runtimes.host_bin_dir !== null) parts.push(`with ${runtimes.host_bin_dir}`)
  return { label, value: parts.join(' '), mono: false }
}

function hostMysqlSummary(runtimes: DatabaseRuntimes): RuntimeSummary {
  const mysql = runtimes.host_mysql
  const parts = [`Dumped as ${mysql.os_user ?? DEFAULT_HOST_ROOT_USER}`]
  if (mysql.socket !== null) parts.push(`through ${mysql.socket}`)
  if (mysql.port !== null) parts.push(`on port ${mysql.port}`)
  if (mysql.bin_dir !== null) parts.push(`with ${mysql.bin_dir}`)
  return {
    label: `${MYSQL_ENGINE_LABELS[mysql.engine]} on this host`,
    value: parts.join(' '),
    mono: false,
  }
}

function hostMongodbSummary(runtimes: DatabaseRuntimes): RuntimeSummary {
  const mongodb = runtimes.host_mongodb
  const parts = [`Dumped as ${mongodb.os_user ?? DEFAULT_HOST_ROOT_USER}`]
  if (mongodb.config_file !== null) parts.push(`with the server named in ${mongodb.config_file}`)
  else if (mongodb.port !== null) parts.push(`on port ${mongodb.port}`)
  if (mongodb.bin_dir !== null) parts.push(`with the tools in ${mongodb.bin_dir}`)
  return { label: 'MongoDB on this host', value: parts.join(' '), mono: false }
}

/**
 * How the host reaches each runtime, one line per runtime. MySQL, MariaDB and
 * MongoDB on the host get a line once they are turned on.
 */
export function runtimeSummaries(runtimes: DatabaseRuntimes): RuntimeSummary[] {
  const extra: RuntimeSummary[] = [
    ...(runtimes.host_mysql.enabled ? [hostMysqlSummary(runtimes)] : []),
    ...(runtimes.host_mongodb.enabled ? [hostMongodbSummary(runtimes)] : []),
  ]
  return [
    searchSummary(
      'Kubernetes cluster',
      runtimes.kubernetes,
      runtimes.kubeconfig_path,
      'agent default kubeconfig',
    ),
    searchSummary(
      'Docker or Podman containers',
      runtimes.docker,
      runtimes.docker_socket,
      'agent default socket',
    ),
    hostSummary(runtimes),
    ...extra,
  ]
}

function isDatabaseEngine(name: string): name is DatabaseEngine {
  return Object.hasOwn(ENGINE_LABELS, name)
}

/**
 * The engine a server installed on the host runs, which names its source. A
 * name that names no engine is PostgreSQL's: it was the only server on the
 * host before the others joined it.
 */
export function hostEngineLabel(name: string): string {
  return ENGINE_LABELS[isDatabaseEngine(name) ? name : 'postgresql']
}

/** How a server installed on the host is named: after its engine. */
export function hostServerName(name: string): string {
  return `${hostEngineLabel(name)} on this host`
}

/** What tells a source apart from every other one of the host. */
export function sourceKey(source: AgentDatabaseSourceResponse): string {
  return `${source.runtime}:${source.namespace}/${source.name}`
}

/** How the table names a source. */
export function sourceName(source: DatabaseSourceRef): string {
  switch (source.runtime) {
    case 'kubernetes':
      return `${source.namespace}/${source.name}`
    case 'docker':
      return source.namespace === '' ? source.name : `${source.namespace}/${source.name}`
    case 'host':
      return hostServerName(source.name)
  }
}

/** Where a dump of the source runs as of the last scan, or null before one. */
export function sourcePlace(source: AgentDatabaseSourceResponse): string | null {
  if (source.pod === null) return null
  const detail = source.container ?? ''
  switch (source.runtime) {
    case 'kubernetes':
      return `${source.pod} (${detail})`
    case 'docker':
      return `container ${source.pod}`
    case 'host':
      return source.pod === '' ? `as ${detail}` : `as ${detail} through ${source.pod}`
  }
}
