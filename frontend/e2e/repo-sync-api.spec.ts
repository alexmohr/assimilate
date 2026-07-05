// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

interface RepoListEntry {
  id: number
  name: string
  importing: boolean
}

interface RepoDetail {
  importing: boolean
  import_error: string | null
}

interface SyncResponse {
  imported: number
  removed: number
  duration_secs: number
}

async function findNonImportingRepoId(request: {
  get: (url: string) => Promise<{ ok: () => boolean; json: () => Promise<unknown> }>
}): Promise<number> {
  const listResp = await request.get('/api/repos')
  expect(listResp.ok()).toBe(true)
  const repos = (await listResp.json()) as RepoListEntry[]
  const repo = repos.find((r) => !r.importing)
  expect(repo).toBeDefined()
  return repo!.id
}

test.describe('Repo sync API', () => {
  test('sync is accepted and eventually clears the importing flag', async ({ page, request }) => {
    await loginAsAdmin(page)
    const repoId = await findNonImportingRepoId(request)

    // sync_repo runs in a background task and responds immediately.
    const syncResp = await request.post(`/api/repos/${repoId}/sync`)
    expect(syncResp.status()).toBe(202)
    const syncBody = (await syncResp.json()) as SyncResponse
    expect(typeof syncBody.imported).toBe('number')
    expect(typeof syncBody.removed).toBe('number')
    expect(typeof syncBody.duration_secs).toBe('number')

    await expect(async () => {
      const detail = (await (await request.get(`/api/repos/${repoId}`)).json()) as RepoDetail
      expect(detail.importing).toBe(false)
      expect(detail.import_error).toBeNull()
    }).toPass({ timeout: 60_000 })
  })

  test('sync can be run again once the previous sync has completed', async ({ page, request }) => {
    await loginAsAdmin(page)
    const repoId = await findNonImportingRepoId(request)

    const sync1 = await request.post(`/api/repos/${repoId}/sync`)
    expect(sync1.status()).toBe(202)

    await expect(async () => {
      const detail = (await (await request.get(`/api/repos/${repoId}`)).json()) as RepoDetail
      expect(detail.importing).toBe(false)
    }).toPass({ timeout: 60_000 })

    const sync2 = await request.post(`/api/repos/${repoId}/sync`)
    expect(sync2.status()).toBe(202)

    await expect(async () => {
      const detail = (await (await request.get(`/api/repos/${repoId}`)).json()) as RepoDetail
      expect(detail.importing).toBe(false)
      expect(detail.import_error).toBeNull()
    }).toPass({ timeout: 60_000 })
  })

  test('reset-import clears the importing flag', async ({ page, request }) => {
    await loginAsAdmin(page)
    const repoId = await findNonImportingRepoId(request)

    const resetResp = await request.post(`/api/repos/${repoId}/reset-import`)
    expect(resetResp.status()).toBe(204)

    const detail = (await (await request.get(`/api/repos/${repoId}`)).json()) as RepoDetail
    expect(detail.importing).toBe(false)
    expect(detail.import_error).toBeNull()
  })
})
