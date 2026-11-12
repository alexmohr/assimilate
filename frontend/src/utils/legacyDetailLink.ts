// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { LocationQuery, RouteLocationRaw } from 'vue-router'
import { runLogLink } from './activityLink'

/**
 * Bookmarks and old notification links still name places the redesign moved:
 * an agent's or schedule's Logs tab (now the Activity log), and the agent
 * settings sections that became the Sources tab (VMs, databases) or the
 * header (tags). Each detail page asks this on arrival and replaces the URL
 * with where that place lives now, so an old link lands somewhere sensible
 * instead of on the first tab.
 */

/** The page an old link was opened on. */
export interface LegacyLinkPage {
  hostname?: string
  scheduleId?: number
}

function isLogsTab(value: unknown): value is 'logs' {
  return value === 'logs'
}

function isSettingsTab(value: unknown): value is 'settings' {
  return value === 'settings'
}

function isSourcesSection(value: unknown): value is 'vms' | 'databases' {
  return value === 'vms' || value === 'databases'
}

function isTagsSection(value: unknown): value is 'tags' {
  return value === 'tags'
}

function isRunStatus(value: unknown): value is 'failed' | 'warning' {
  return value === 'failed' || value === 'warning'
}

function reportIdOf(value: unknown): number | null {
  if (typeof value !== 'string') return null
  const id = Number(value)
  return Number.isInteger(id) && id > 0 ? id : null
}

/** Where an old detail-page link lives now, or null when it is current. */
export function legacyDetailLink(
  query: LocationQuery,
  page: LegacyLinkPage,
): RouteLocationRaw | null {
  if (isLogsTab(query.tab)) {
    return runLogLink({
      hostname: page.hostname,
      scheduleId: page.scheduleId,
      status: isRunStatus(query.status) ? query.status : null,
      reportId: reportIdOf(query.report),
    })
  }
  if (page.hostname === undefined || !isSettingsTab(query.tab)) return null
  if (isSourcesSection(query.section)) {
    return { query: { ...query, tab: 'sources' } }
  }
  if (isTagsSection(query.section)) {
    const rest: LocationQuery = { ...query, tab: 'overview' }
    delete rest.section
    return { query: rest }
  }
  return null
}
