// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref } from 'vue'
import type { Ref } from 'vue'
import { deployAgent, dismissAgentDeploy, getAgentDeployStatus } from '../api/agents'
import type { DeployAgentRequest, DeployAgentResult } from '../api/agents'
import { extractError } from '../utils/error'
import { useTimeout } from './useTimeout'

/** How often a running deploy is asked how far it has got. */
const POLL_INTERVAL_MS = 1000
/** Polls in a row that may fail before the dialog stops asking. */
const MAX_POLL_FAILURES = 5

export interface UseAgentDeployReturn {
  /** The deploy as the server last reported it; `null` before one starts. */
  status: Ref<DeployAgentResult | null>
  /** True from the request that starts a deploy until the server has answered. */
  starting: Ref<boolean>
  /** Why a request failed, as opposed to why the deploy did. */
  error: Ref<string | null>
  /** Starts a deploy and follows it until it ends. */
  start: (request: DeployAgentRequest) => Promise<void>
  /** Picks up a deploy that is already running or recent, as after reopening the dialog. */
  resume: () => Promise<void>
  /** Forgets the result and drops the token it holds, so the form can be used again. */
  dismiss: () => Promise<void>
}

/**
 * Drives one agent's deploy, which runs on the server. The request that
 * starts it answers at once; this asks for the deploy's status until it ends,
 * so no request lasts long enough for a proxy to cut it.
 *
 * `onSucceeded` is called once, with the deployed version, when a deploy this
 * dialog started ends well - not for one that was already finished when it
 * was picked up again.
 */
export function useAgentDeploy(
  hostname: () => string,
  domain: () => string | null | undefined,
  onSucceeded: (version: string | null) => void,
): UseAgentDeployReturn {
  const status = ref<DeployAgentResult | null>(null)
  const starting = ref(false)
  const error = ref<string | null>(null)
  const timer = useTimeout()
  let failedPolls = 0

  function isRunning(): boolean {
    return status.value?.state === 'running'
  }

  function schedulePoll(): void {
    if (isRunning()) timer.start(() => void poll(), POLL_INTERVAL_MS)
  }

  /** Takes in a status, telling `onSucceeded` when a deploy being followed has just ended well. */
  function apply(next: DeployAgentResult, notify: boolean): void {
    const wasRunning = isRunning()
    status.value = next
    if (notify && wasRunning && next.state === 'succeeded') onSucceeded(next.available_version)
  }

  async function poll(): Promise<void> {
    try {
      const next = await getAgentDeployStatus(hostname(), domain())
      failedPolls = 0
      if (next === null) {
        status.value = null
        error.value = 'The server no longer has this deploy. Check the host before trying again.'
        return
      }
      apply(next, true)
    } catch (e: unknown) {
      failedPolls++
      if (failedPolls >= MAX_POLL_FAILURES) {
        error.value = extractError(
          e,
          'Lost contact with the server; the deploy carries on without it',
        )
        return
      }
    }
    schedulePoll()
  }

  async function start(request: DeployAgentRequest): Promise<void> {
    starting.value = true
    error.value = null
    failedPolls = 0
    try {
      const started = await deployAgent(hostname(), request, domain())
      // A deploy already finished at the start - the agent was current - has
      // nothing to wait for, so it is not one this dialog saw succeed.
      status.value = started
      schedulePoll()
    } catch (e: unknown) {
      error.value = extractError(e)
    } finally {
      starting.value = false
    }
  }

  async function resume(): Promise<void> {
    try {
      const found = await getAgentDeployStatus(hostname(), domain())
      if (found === null) return
      apply(found, false)
      schedulePoll()
    } catch {
      // Nothing to pick up is the same as nothing to show; the form works as usual.
    }
  }

  async function dismiss(): Promise<void> {
    timer.clear()
    const finished = status.value !== null && !isRunning()
    status.value = null
    error.value = null
    if (!finished) return
    try {
      await dismissAgentDeploy(hostname(), domain())
    } catch {
      // The server forgets a finished deploy on its own; failing to say so early is harmless.
    }
  }

  return { status, starting, error, start, resume, dismiss }
}
