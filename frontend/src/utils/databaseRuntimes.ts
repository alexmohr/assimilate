// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { DatabaseSourceRef } from '../api/databases'
import type {
  AgentDatabaseSourceResponse,
  DatabaseRuntime,
  DatabaseRuntimes,
  DatabaseSourceKind,
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

export const KIND_LABELS: Record<DatabaseSourceKind, string> = {
  cnpg_cluster: 'CloudNativePG',
  annotated: 'Annotated pod',
  container: 'Labelled container',
  host: 'Host server',
}

export const RUNTIME_LABELS: Record<DatabaseRuntime, string> = {
  kubernetes: 'Kubernetes',
  docker: 'Docker',
  host: 'Host',
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
    hostPort: runtimes.host_port === null ? '' : String(runtimes.host_port),
    hostBinDir: runtimes.host_bin_dir ?? '',
  }
}

function textOrNull(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

/**
 * The port typed into the form, or null for the default. Whatever else was
 * typed is sent as it is, so the server can say what is wrong with it rather
 * than the form quietly dropping it.
 */
function portOrNull(value: string): number | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : Number(trimmed)
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

/** How the host reaches each runtime, one line per runtime. */
export function runtimeSummaries(runtimes: DatabaseRuntimes): RuntimeSummary[] {
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
  ]
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
      return 'PostgreSQL on this host'
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
