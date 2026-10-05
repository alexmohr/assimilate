// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import { getRunEvents } from '../api/runs'
import { logger } from '../utils/logger'
import { useWebSocket } from './useWebSocket'
import type { ReportRow } from '../types/report'
import type { RunEventResponse } from '../types/generated'

export interface RunEventsState {
  runEvents: Ref<RunEventResponse[]>
  loadingEvents: Ref<boolean>
  /**
   * Most runs have wake/start disabled and record no power-management events
   * at all, so an empty result is the common case, not a failure - kept apart
   * from a fetch that errored so expanding a run always shows *something*
   * rather than a toggle that silently does nothing.
   */
  eventsFetched: Ref<boolean>
  eventsError: Ref<boolean>
}

/**
 * One report's slice of its run's power-management timeline, fetched the
 * first time the row is expanded and kept current by the live `RunEvent`
 * stream while it is open.
 *
 * Shared by every row that expands a run - the host's backup rows and the
 * schedule Overview's run detail - so the fetch, the buffering of an event
 * that lands mid-fetch and the target scoping cannot drift between them.
 */
export function useRunEvents(report: () => ReportRow, expanded: () => boolean): RunEventsState {
  const runEvents = ref<RunEventResponse[]>([])
  const loadingEvents = ref(false)
  const eventsFetched = ref(false)
  const eventsError = ref(false)

  // A live RunEvent that arrives while the initial fetch is still in flight
  // would otherwise be dropped: it is not in the fetch response (already sent
  // before the event happened) and the live handler below only appends once
  // fetched. Buffered here and merged in once the fetch resolves.
  const bufferedLiveEvents: RunEventResponse[] = []

  watch(
    expanded,
    (isExpanded) => {
      const current = report()
      const runId = current.run_id
      if (!isExpanded || !runId || eventsFetched.value || loadingEvents.value) return
      loadingEvents.value = true
      eventsError.value = false
      getRunEvents(runId, current.agent_id, current.repo_id)
        .then((events) => {
          runEvents.value = [...events, ...bufferedLiveEvents]
          bufferedLiveEvents.length = 0
          eventsFetched.value = true
        })
        .catch((e: unknown) => {
          logger.error('failed to load run events', e)
          eventsError.value = true
        })
        .finally(() => {
          loadingEvents.value = false
        })
    },
    // A report can arrive already expanded (a deep link pins a specific run),
    // and that first render deserves its timeline fetched too.
    { immediate: true },
  )

  // Keyed by a synthetic negative id (real rows are a positive bigserial)
  // since the WS payload carries no row id, only enough to render one.
  let nextLiveEventKey = -1
  const { onMessage } = useWebSocket()
  onMessage('RunEvent', (payload) => {
    // run_id alone is shared across every target of a multi-target schedule,
    // so a sibling target's event would otherwise bleed into this row's
    // timeline - the same (run_id, agent_id, repo_id) scoping the fetch uses.
    const current = report()
    if (
      payload.run_id !== current.run_id ||
      payload.agent_id !== current.agent_id ||
      payload.repo_id !== current.repo_id
    ) {
      return
    }
    const event: RunEventResponse = {
      id: nextLiveEventKey--,
      run_id: payload.run_id,
      target: payload.target,
      event_type: payload.event_type,
      message: payload.message,
      occurred_at: payload.occurred_at,
    }
    if (eventsFetched.value) {
      runEvents.value = [...runEvents.value, event]
    } else if (loadingEvents.value) {
      bufferedLiveEvents.push(event)
    }
    // Otherwise the row has never been expanded (or its fetch failed) - a
    // future expand's own fetch returns the full history, this event included.
  })

  return { runEvents, loadingEvents, eventsFetched, eventsError }
}
