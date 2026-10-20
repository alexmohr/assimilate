// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  deleteArchive,
  diffArchiveFile,
  diffArchives,
  downloadArchiveFiles,
  getArchiveContents,
  getArchiveIndexStatus,
  listArchiveDatabases,
  listRepoArchives,
  restoreArchiveDatabase,
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

    await expect(listRepoArchives(1)).resolves.toEqual([])

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/archives')
  })

  it('gets the archive index status', async () => {
    const data = { status: 'done', file_count: 12, error: null }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(getArchiveIndexStatus(1, ARCHIVE_NAME)).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/index-status`,
    )
  })

  it('gets archive contents at the root when no path is given', async () => {
    const data = { index_status: 'done', entries: [] }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(getArchiveContents(1, ARCHIVE_NAME)).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/contents`,
      {
        params: {},
      },
    )
  })

  it('gets archive contents at a given path', async () => {
    const data = { index_status: 'done', entries: [] }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(getArchiveContents(1, ARCHIVE_NAME, { path: 'etc/nginx' })).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/contents`,
      {
        params: { path: 'etc/nginx' },
      },
    )
  })

  it('searches within a single archive', async () => {
    const data = { items: [], total_matched: 0, limit: 100, offset: 0 }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(
      searchArchive(1, ARCHIVE_NAME, { pattern: '*.conf', limit: 100, offset: 0 }),
    ).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith(`/repos/1/archives/${ENCODED_ARCHIVE_NAME}/search`, {
      params: { pattern: '*.conf', limit: 100, offset: 0 },
    })
  })

  it('searches across archives', async () => {
    const data = { items: [], total_archives_searched: 0, limit: 200, offset: 0 }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(searchAcrossArchives(1, { pattern: '*.conf', maxArchives: 20 })).resolves.toEqual(
      data,
    )

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '*.conf', max_archives: 20 },
    })
  })

  it('diffs two archives', async () => {
    const data = { added: [], removed: [], modified: [], total_changes: 0, limit: 100, offset: 0 }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(diffArchives(1, { archive1: 'a1', archive2: 'a2' })).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/1/archives/diff', {
      params: { archive1: 'a1', archive2: 'a2' },
    })
  })

  it('compares one file between two archives', async () => {
    const data = { status: 'text', added: 1, removed: 1, hunks: [] }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(
      diffArchiveFile(1, ARCHIVE_NAME, { path: 'etc/nginx/nginx.conf', against: 'older' }),
    ).resolves.toEqual(data)

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/file-diff`,
      {
        params: {
          path: 'etc/nginx/nginx.conf',
          against: 'older',
          against_repo_id: undefined,
          whole_file: undefined,
        },
      },
    )
  })

  it('asks for the whole file, and for an archive in another repository', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { status: 'binary', side: 'base' } })

    await diffArchiveFile(1, ARCHIVE_NAME, {
      path: 'etc/hosts',
      against: 'older',
      againstRepoId: 2,
      wholeFile: true,
    })

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/file-diff`,
      {
        params: { path: 'etc/hosts', against: 'older', against_repo_id: 2, whole_file: true },
      },
    )
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
    const data = { success: true, files_restored: 2, error_message: null }
    vi.mocked(apiClient.post).mockResolvedValue({ data })

    await expect(
      restoreArchiveFiles(1, ARCHIVE_NAME, {
        paths: ['/etc/nginx/nginx.conf'],
        target_path: '/tmp/restore',
        hostname: 'web-server-01',
      }),
    ).resolves.toEqual(data)

    expect(apiClient.post).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/restore`,
      {
        paths: ['/etc/nginx/nginx.conf'],
        target_path: '/tmp/restore',
        hostname: 'web-server-01',
      },
    )
  })

  it("lists an archive's database dumps", async () => {
    const dump = {
      manifest_path: 'var/lib/assimilate/database-dumps/manifest.json',
      archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
      runtime: 'kubernetes',
      namespace: 'shop',
      name: 'orders-db',
      kind: 'cnpg_cluster',
      engine: 'postgresql',
      pod: 'orders-db-1',
      database: 'orders',
      path: 'shop/orders-db/orders.dump',
      format: 'pg_custom',
    }
    vi.mocked(apiClient.get).mockResolvedValue({ data: { dumps: [dump] } })

    await expect(listArchiveDatabases(1, ARCHIVE_NAME)).resolves.toEqual([dump])

    expect(apiClient.get).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/databases`,
    )
  })

  it('starts restoring a database dump', async () => {
    const data = { id: 'restore-2', status: 'running' }
    vi.mocked(apiClient.post).mockResolvedValue({ data })
    const body = {
      archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
      target: { mode: 'new_database' as const, name: 'orders_restored' },
      hostname: 'db-01',
      domain: null,
    }

    await expect(restoreArchiveDatabase(1, ARCHIVE_NAME, body)).resolves.toEqual(data)

    expect(apiClient.post).toHaveBeenCalledWith(
      `/repos/1/archives/${ENCODED_ARCHIVE_NAME}/databases/restore`,
      body,
    )
  })

  it('deletes an archive', async () => {
    const data = { success: true, archive_name: ARCHIVE_NAME }
    vi.mocked(apiClient.delete).mockResolvedValue({ data })

    await expect(deleteArchive(1, ARCHIVE_NAME)).resolves.toEqual(data)

    expect(apiClient.delete).toHaveBeenCalledWith(`/repos/1/archives/${ENCODED_ARCHIVE_NAME}`)
  })
})
