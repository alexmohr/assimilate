// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Finds the schedules whose next runs really contend for a repository lock.
 *
 * Borg holds an exclusive lock on a repository while it writes, so two runs
 * into the same repository at the same time make one of them wait, or fail
 * once the lock timeout runs out. That only happens across agents: one agent
 * runs its own jobs one after another, so two schedules on the same agent
 * never fight over the lock, and two runs into different repositories never
 * do either - sharing a storage host is not a conflict.
 *
 * Each schedule's next run is taken to occupy `[next start, next start +
 * estimated duration)`, where the estimate is the median duration of the
 * schedule's recent runs. The interval is half-open, so a run that starts the
 * moment another one is expected to end is not an overlap.
 */

/**
 * The run length assumed for a schedule with no recent runs to estimate from:
 * five minutes, long enough that two schedules started at the same minute on
 * one repository are still flagged, short enough that runs a quarter of an
 * hour apart are not.
 */
export const DEFAULT_RUN_ESTIMATE_SECS = 5 * 60

/**
 * The shortest estimate ever used. A history of runs that all finished within
 * the same second would otherwise give a zero-length interval, and two
 * zero-length runs started together would read as not overlapping.
 */
export const MIN_RUN_ESTIMATE_SECS = 60

/** One schedule as the overlap check needs it. */
export interface OverlapSchedule {
  id: number
  /** Hostnames of the agents the schedule runs on. */
  agents: readonly string[]
  /** Every repository the schedule writes into. */
  repoIds: readonly number[]
  /** When the next run starts, in epoch milliseconds; null when none is due. */
  nextStartMs: number | null
  /** Durations, in seconds, of the schedule's recent finished runs. */
  recentDurationsSecs: readonly number[]
}

/** How long a schedule's next run is expected to take. */
export interface RunEstimate {
  secs: number
  /** False when there was no history and the default was assumed. */
  fromHistory: boolean
}

/** One side of an overlap: which schedule, on which agent, and when. */
export interface OverlapSide {
  scheduleId: number
  agent: string
  startMs: number
  endMs: number
  estimate: RunEstimate
}

/** Two schedules on different agents writing into one repository at once. */
export interface ScheduleOverlap {
  repoId: number
  /** The run that starts first (the lower schedule id on a tie). */
  first: OverlapSide
  second: OverlapSide
}

/** The median of the durations, or the default when there are none. */
export function estimateRunSecs(durationsSecs: readonly number[]): RunEstimate {
  const usable = durationsSecs.filter((d) => Number.isFinite(d) && d >= 0).sort((a, b) => a - b)
  if (usable.length === 0) return { secs: DEFAULT_RUN_ESTIMATE_SECS, fromHistory: false }
  const mid = Math.floor(usable.length / 2)
  const median = usable.length % 2 === 1 ? usable[mid] : (usable[mid - 1] + usable[mid]) / 2
  return { secs: Math.max(MIN_RUN_ESTIMATE_SECS, median), fromHistory: true }
}

/**
 * The first pair of distinct agents, one from each schedule. A schedule that
 * shares an agent with the other may still run on a second one, and that
 * second agent does contend with the other schedule's run.
 */
function distinctAgentPair(a: readonly string[], b: readonly string[]): [string, string] | null {
  for (const agentA of a) {
    for (const agentB of b) {
      if (agentA !== agentB) return [agentA, agentB]
    }
  }
  return null
}

interface Run {
  schedule: OverlapSchedule
  startMs: number
  endMs: number
  estimate: RunEstimate
}

/**
 * Every real conflict, one per pair of schedules and shared repository,
 * ordered by when the first run starts. A schedule overlapping two others
 * appears in two entries.
 */
export function findScheduleOverlaps(schedules: readonly OverlapSchedule[]): ScheduleOverlap[] {
  const runs: Run[] = schedules
    .filter(
      (s): s is OverlapSchedule & { nextStartMs: number } =>
        s.nextStartMs !== null && Number.isFinite(s.nextStartMs),
    )
    .map((s) => {
      const estimate = estimateRunSecs(s.recentDurationsSecs)
      return {
        schedule: s,
        startMs: s.nextStartMs,
        endMs: s.nextStartMs + estimate.secs * 1000,
        estimate,
      }
    })
    .sort((a, b) => a.startMs - b.startMs || a.schedule.id - b.schedule.id)

  const overlaps: ScheduleOverlap[] = []
  for (let i = 0; i < runs.length; i++) {
    for (let j = i + 1; j < runs.length; j++) {
      const first = runs[i]
      const second = runs[j]
      if (!(first.startMs < second.endMs && second.startMs < first.endMs)) continue
      const agents = distinctAgentPair(first.schedule.agents, second.schedule.agents)
      if (agents === null) continue
      const shared = [...new Set(first.schedule.repoIds)].filter((id) =>
        second.schedule.repoIds.includes(id),
      )
      for (const repoId of shared) {
        overlaps.push({
          repoId,
          first: side(first, agents[0]),
          second: side(second, agents[1]),
        })
      }
    }
  }
  return overlaps
}

function side(run: Run, agent: string): OverlapSide {
  return {
    scheduleId: run.schedule.id,
    agent,
    startMs: run.startMs,
    endMs: run.endMs,
    estimate: run.estimate,
  }
}

/** For each schedule in an overlap, the agents of the runs it overlaps with. */
export function overlappingAgents(overlaps: readonly ScheduleOverlap[]): Map<number, string[]> {
  const byId = new Map<number, string[]>()
  const add = (scheduleId: number, agent: string): void => {
    const list = byId.get(scheduleId) ?? []
    if (!list.includes(agent)) list.push(agent)
    byId.set(scheduleId, list)
  }
  for (const overlap of overlaps) {
    add(overlap.first.scheduleId, overlap.second.agent)
    add(overlap.second.scheduleId, overlap.first.agent)
  }
  return byId
}
