// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { scanRepoHostKey } from '../api/repoHosts'
import { useHostKeyCheck, type HostKeyCheckTarget } from './useHostKeyCheck'

vi.mock('../api/repoHosts', () => ({ scanRepoHostKey: vi.fn() }))

const PINNED = 'ssh-ed25519 AAAAPINNED'
const CHANGED = 'ssh-ed25519 AAAACHANGED'

describe('useHostKeyCheck', () => {
  let target: HostKeyCheckTarget

  beforeEach(() => {
    vi.mocked(scanRepoHostKey).mockReset()
    target = { hostId: 5, pinnedKey: PINNED, enabled: true }
  })

  it('stays quiet when the host presents the pinned key', async () => {
    vi.mocked(scanRepoHostKey).mockResolvedValue({ ssh_host_key: PINNED })
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(scanRepoHostKey).toHaveBeenCalledWith(5)
    expect(changedKey.value).toBeNull()
  })

  it('offers a different key for review', async () => {
    vi.mocked(scanRepoHostKey).mockResolvedValue({ ssh_host_key: CHANGED })
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(changedKey.value).toBe(CHANGED)
  })

  it('scans nothing for a viewer who could not act on it', async () => {
    target.enabled = false
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(scanRepoHostKey).not.toHaveBeenCalled()
    expect(changedKey.value).toBeNull()
  })

  // The host may simply be asleep: that is not a changed key.
  it('treats a failed scan as no mismatch known', async () => {
    vi.mocked(scanRepoHostKey).mockRejectedValue(new Error('timed out'))
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(changedKey.value).toBeNull()
  })

  it('drops a reply for a host it has moved away from', async () => {
    vi.mocked(scanRepoHostKey).mockImplementation(async () => {
      target = { hostId: 9, pinnedKey: PINNED, enabled: true }
      return { ssh_host_key: CHANGED }
    })
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(changedKey.value).toBeNull()
  })

  it('clears an earlier finding when it checks again', async () => {
    vi.mocked(scanRepoHostKey).mockResolvedValueOnce({ ssh_host_key: CHANGED })
    vi.mocked(scanRepoHostKey).mockResolvedValueOnce({ ssh_host_key: PINNED })
    const { changedKey, check } = useHostKeyCheck(() => target)
    await check()
    expect(changedKey.value).toBe(CHANGED)
    await check()
    expect(changedKey.value).toBeNull()
  })
})
