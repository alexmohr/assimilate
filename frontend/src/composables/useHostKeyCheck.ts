// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { scanRepoHostKey } from '../api/repoHosts'
import { logger } from '../utils/logger'

/** The host a check asks about, read afresh each time it runs. */
export interface HostKeyCheckTarget {
  /** Repository host to scan. */
  hostId: number
  /** The key pinned for it, or null when none is pinned yet. */
  pinnedKey: string | null
  /** Whether this viewer may act on a changed key; otherwise nothing is scanned. */
  enabled: boolean
}

interface UseHostKeyCheckReturn {
  /** The key the host presents now, when it differs from the pinned one. */
  changedKey: Ref<string | null>
  check: () => Promise<void>
}

/**
 * Whether a repository host now presents another SSH key than the one pinned
 * for it - the signature of a reinstall or of a man-in-the-middle, so it
 * surfaces where the host is shown rather than only as the next failed
 * backup. Which of the two it is takes a human, so the key is only offered
 * for review. A failed scan says nothing about the key: the host may simply
 * be asleep. A reply for a host the caller has since moved away from is
 * dropped.
 */
export function useHostKeyCheck(target: () => HostKeyCheckTarget): UseHostKeyCheckReturn {
  const changedKey = ref<string | null>(null)

  async function check(): Promise<void> {
    changedKey.value = null
    const { hostId, enabled } = target()
    if (!enabled) return
    try {
      const { ssh_host_key: key } = await scanRepoHostKey(hostId)
      const current = target()
      if (current.hostId !== hostId) return
      if (key !== current.pinnedKey) changedKey.value = key
    } catch (e: unknown) {
      logger.debug('host key scan failed', e)
    }
  }

  return { changedKey, check }
}
