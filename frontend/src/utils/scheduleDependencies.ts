// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { relativeTime } from './format'
import type {
  DependencyWaitResponse,
  RepoCatchUpCheckResponse,
  ScheduleDependenciesResponse,
  ScheduleDependencyResponse,
} from '../types/generated'

/** A dependency as the schedule Overview lists it: once, with where it applies. */
export interface ScheduleDependencySummary {
  id: number
  name: string
  lastCheckReachable: boolean | null
  /** The agents that need it, each marked when the need is their backup defaults'. */
  appliesTo: string
}

/**
 * One entry per dependency, in the order they first appear, however many of
 * the schedule's agents need it. The per-agent list is what Settings edits;
 * a status screen wants to know whether the machine is up.
 */
export function summarizeScheduleDependencies(
  dependencies: readonly ScheduleDependencyResponse[],
  agentLabel: (id: number) => string,
): ScheduleDependencySummary[] {
  const byId = new Map<number, { summary: ScheduleDependencySummary; uses: string[] }>()
  for (const dep of dependencies) {
    const use =
      dep.source === 'agent_default'
        ? `${agentLabel(dep.agent_id)} (agent defaults)`
        : agentLabel(dep.agent_id)
    const entry = byId.get(dep.dependency_host_id)
    if (entry) {
      entry.uses.push(use)
      continue
    }
    byId.set(dep.dependency_host_id, {
      summary: {
        id: dep.dependency_host_id,
        name: dep.dependency_name,
        lastCheckReachable: dep.last_check_reachable,
        appliesTo: '',
      },
      uses: [use],
    })
  }
  return [...byId.values()].map(({ summary, uses }) => ({ ...summary, appliesTo: uses.join(', ') }))
}

/**
 * When a waiting run is next looked at and when it is given up on. A wait
 * whose dependency already answered is catching up, and has neither.
 */
export function dependencyWaitNote(wait: DependencyWaitResponse): string {
  if (wait.catching_up) return 'Catching up now'
  const parts: string[] = []
  if (wait.next_probe_at) parts.push(`Next check ${relativeTime(wait.next_probe_at)}`)
  if (wait.give_up_at) parts.push(`gives up ${relativeTime(wait.give_up_at)}`)
  const text = parts.join(' · ')
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** The header's badge text, or null when nothing is waiting. */
export function dependencyWaitBadge(waits: readonly DependencyWaitResponse[]): string | null {
  const names = [...new Set(waits.map((w) => w.dependency_name))]
  if (names.length === 0) return null
  if (names.length === 1) return `Waiting for ${names[0]}`
  return `Waiting for ${names.length} dependencies`
}

/**
 * Says what Check now found, because "done" leaves the one question worth
 * answering - is it back? - unanswered.
 */
export function dependencyCheckOutcomeText(
  name: string,
  outcome: RepoCatchUpCheckResponse,
): string {
  if (outcome.probed === 0) {
    if (outcome.abandoned > 0) {
      return outcome.abandoned === 1
        ? `Stopped waiting for ${name} - 1 run was past its window and is reported as failed`
        : `Stopped waiting for ${name} - ${outcome.abandoned} runs were past their window and are reported as failed`
    }
    return `Nothing is waiting on ${name}`
  }
  if (outcome.started > 0) {
    return outcome.started === 1
      ? `${name} is back - catching up 1 run`
      : `${name} is back - catching up ${outcome.started} runs`
  }
  if (outcome.reachable > 0) {
    return `${name} is back, but each schedule runs again soon enough on its own`
  }
  return `${name} is still not answering`
}

/**
 * The response as the page uses it. An answer without either list - a server
 * that predates dependencies - reads as one that has none.
 */
export function normalizeScheduleDependencies(
  response: Partial<ScheduleDependenciesResponse> | null | undefined,
): ScheduleDependenciesResponse {
  return {
    dependencies: Array.isArray(response?.dependencies) ? response.dependencies : [],
    waiting: Array.isArray(response?.waiting) ? response.waiting : [],
  }
}
