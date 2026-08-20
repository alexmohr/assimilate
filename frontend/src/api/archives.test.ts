// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  deleteArchive,
  diffArchives,
  downloadArchiveFiles,
  getArchiveContents,
  getArchiveIndexStatus,
  listRepoArchives,
  restoreArchiveFiles,
  searchAcrossArchives,
  searchArchive,
} from './archives'

const ARCHIVE_NAME = 'web-server-01-2026-06-05T02:00:00'
const ENCODED_ARCHIVE_NAME = 'web-server-01-2026-06-05T02%3A00%3A00'

describe('archives api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists repo archives', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listRepoArchives(1)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/archives')
  })

  it('gets the archive index status', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { status: 'done', file_count: 12, error: null },
    })

    await getArchiveIndexStatus(1, ARCHIVE_NAME)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/index-status`,
    )
  })

  it('gets archive contents at the root when no path is given', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { index_status: 'done', entries: [] } })

    await getArchiveContents(1, ARCHIVE_NAME)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/contents`,
      {
        params: {},
      },
    )
  })

  it('gets archive contents at a given path', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { index_status: 'done', entries: [] } })

    await getArchiveContents(1, ARCHIVE_NAME, { path: 'etc/nginx' })

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/contents`,
      {
        params: { path: 'etc/nginx' },
      },
    )
  })

  it('searches within a single archive', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { items: [], total_matched: 0, limit: 100, offset: 0 },
    })

    await searchArchive(1, ARCHIVE_NAME, { pattern: '*.conf', limit: 100, offset: 0 })

    expect(apiClient.get).toHaveBeenCalledWith(`/repos/1/archives/${ENCODED_ARCHIVE_NAME}/search`, {
      params: { pattern: '*.conf', limit: 100, offset: 0 },
    })
  })

  it('searches across archives', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { items: [], total_archives_searched: 0, limit: 200, offset: 0 },
    })

    await searchAcrossArchives(1, { pattern: '*.conf', maxArchives: 20 })

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '*.conf', max_archives: 20 },
    })
  })

  it('diffs two archives', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { added: [], removed: [], modified: [], total_changes: 0, limit: 100, offset: 0 },
    })

    await diffArchives(1, { archive1: 'a1', archive2: 'a2' })

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/archives/diff', {
      params: { archive1: 'a1', archive2: 'a2' },
    })
  })

  it('downloads archive files as a blob', async () => {
    const blob = new Blob(['tar-content'])
    vi.mocked(apiClient.post).mockResolvedValue({ data: blob })
    const controller = new AbortController()

    const result = await downloadArchiveFiles(
      1,
      ARCHIVE_NAME,
      ['/etc/nginx/nginx.conf'],
      controller.signal,
    )

    expect(apiClient.post).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/download`,
      { paths: ['/etc/nginx/nginx.conf'] },
      { responseType: 'blob', signal: controller.signal },
    )
    expect(result).toBe(blob)
  })

  it('restores archive files to an agent', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { success: true, files_restored: 2, error_message: null },
    })

    await restoreArchiveFiles(1, ARCHIVE_NAME, {
      paths: ['/etc/nginx/nginx.conf'],
      target_path: '/tmp/restore',
      hostname: 'web-server-01',
    })

    expect(apiClient.post).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/restore`,
      {
        paths: ['/etc/nginx/nginx.conf'],
        target_path: '/tmp/restore',
        hostname: 'web-server-01',
      },
    )
  })

  it('deletes an archive', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({
      data: { success: true, archive_name: ARCHIVE_NAME },
    })

    await deleteArchive(1, ARCHIVE_NAME)

    expect(apiClient.delete).toHaveBeenCalledWith(`/repos/1/archives/${ENCODED_ARCHIVE_NAME}`)
  })
})
