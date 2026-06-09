// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

import { apiClient } from '../api/client'
import { useArchiveBrowser, type ArchiveEntry, type ContentEntry } from './useArchiveBrowser'

const ARCHIVE: ArchiveEntry = {
  name: 'web-server-01-backup-2026-06-05T02:00:00',
  start: '2026-06-05T02:00:00',
  hostname: 'imported-hostname',
  comment: '',
  original_size: 1,
  deduplicated_size: 1,
  matched: true,
  client_hostname: 'web-server-01',
}

const ROOT_ENTRY: ContentEntry = {
  type: 'd',
  path: '',
  size: 0,
  mtime: '',
  mode: '',
}

describe('useArchiveBrowser', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubGlobal(
      'confirm',
      vi.fn(() => true),
    )
  })

  it('downloads the root entry as the whole archive without an empty path query', () => {
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    const anchor = document.createElement('a')
    vi.spyOn(document, 'createElement').mockReturnValue(anchor)
    vi.spyOn(anchor, 'click').mockImplementation(() => undefined)

    browser.downloadEntry(ROOT_ENTRY)

    expect(anchor.getAttribute('href')).toBe(
      '/api/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00/export',
    )
  })

  it('restores the root entry as the whole archive to its matched host', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE

    await browser.restoreEntry(ROOT_ENTRY)

    expect(apiClient.post).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00/restore',
      {
        paths: [],
        target_path: '/',
        hostname: 'web-server-01',
      },
    )
  })

  it('restores one entry by passing its archive path', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })
    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    const entry = { ...ROOT_ENTRY, type: '-', path: 'etc/nginx/nginx.conf' }

    await browser.restoreEntry(entry)

    expect(apiClient.post).toHaveBeenCalledWith(expect.any(String), {
      paths: ['etc/nginx/nginx.conf'],
      target_path: '/',
      hostname: 'web-server-01',
    })
  })

  it('deletes the whole archive from the root entry', async () => {
    const confirm = vi.mocked(window.confirm)
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const browser = useArchiveBrowser(ref(5))
    browser.selectedArchive.value = ARCHIVE
    browser.archives.value = [ARCHIVE]

    await browser.deleteArchive(ROOT_ENTRY)

    expect(apiClient.delete).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00',
    )
    expect(browser.selectedArchive.value).toBeNull()
    expect(browser.archives.value).toHaveLength(0)
    expect(confirm).not.toHaveBeenCalled()
  })

  it('deleteArchiveByName deletes by archive name without requiring selectedArchive', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const SECOND: ArchiveEntry = { ...ARCHIVE, name: 'web-server-01-backup-2026-06-06T02:00:00' }
    const browser = useArchiveBrowser(ref(5))
    browser.archives.value = [ARCHIVE, SECOND]

    await browser.deleteArchiveByName(ARCHIVE)

    expect(apiClient.delete).toHaveBeenCalledWith(
      '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00',
    )
    expect(browser.archives.value).toEqual([SECOND])
  })

  it('deleteArchiveByName clears selectedArchive when it matches the deleted archive', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE.name },
    })

    const browser = useArchiveBrowser(ref(5))
    browser.archives.value = [ARCHIVE]
    browser.selectedArchive.value = ARCHIVE

    await browser.deleteArchiveByName(ARCHIVE)

    expect(browser.selectedArchive.value).toBeNull()
    expect(browser.archives.value).toHaveLength(0)
  })
})
