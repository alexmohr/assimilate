// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

interface RepoListEntry {
  id: number;
  name: string;
  importing: boolean;
  import_error: string | null;
}

interface RepoDetail {
  id: number;
  name: string;
  importing: boolean;
  import_error: string | null;
  import_progress: number;
  import_total: number;
  import_status_message: string | null;
  archive_count: number;
}

interface SyncResponse {
  imported: number;
  removed: number;
  duration_secs: number;
}

test.describe('Import / Sync lifecycle', () => {
  test('sync completes and clears importing flag', async ({ request }) => {
    // Find a repo that is NOT currently importing.
    const listResp = await request.get('/api/repos');
    expect(listResp.ok()).toBe(true);
    const repos = (await listResp.json()) as RepoListEntry[];
    expect(repos.length).toBeGreaterThan(0);

    const repo = repos.find((r) => !r.importing);
    expect(repo).toBeDefined();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const repoId = repo!.id;

    // sync_repo is synchronous -- it blocks until the sync is done.
    const syncResp = await request.post(`/api/repos/${repoId}/sync`);
    expect(syncResp.ok()).toBe(true);
    const syncBody = (await syncResp.json()) as SyncResponse;
    expect(syncBody.imported).toBeGreaterThanOrEqual(0);
    expect(syncBody.removed).toBeGreaterThanOrEqual(0);

    // After sync returns, the importing flag must be cleared.
    const detailResp = await request.get(`/api/repos/${repoId}`);
    expect(detailResp.ok()).toBe(true);
    const detail = (await detailResp.json()) as RepoDetail;
    expect(detail.importing).toBe(false);
    expect(detail.import_error).toBeNull();
  });

  test('sync is idempotent - second sync also succeeds', async ({
    request,
  }) => {
    const listResp = await request.get('/api/repos');
    expect(listResp.ok()).toBe(true);
    const repos = (await listResp.json()) as RepoListEntry[];
    const repo = repos.find((r) => !r.importing);
    expect(repo).toBeDefined();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const repoId = repo!.id;

    // Two consecutive syncs should both succeed.
    const sync1 = await request.post(`/api/repos/${repoId}/sync`);
    expect(sync1.ok()).toBe(true);

    const sync2 = await request.post(`/api/repos/${repoId}/sync`);
    expect(sync2.ok()).toBe(true);

    const detail = (
      await (await request.get(`/api/repos/${repoId}`)).json()
    ) as RepoDetail;
    expect(detail.importing).toBe(false);
    expect(detail.import_error).toBeNull();
  });

  test('sync returns expected response structure with imported and removed counts', async ({
    request,
  }) => {
    const listResp = await request.get('/api/repos');
    expect(listResp.ok()).toBe(true);
    const repos = (await listResp.json()) as RepoListEntry[];
    const repo = repos.find((r) => !r.importing);
    expect(repo).toBeDefined();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const repoId = repo!.id;

    const syncResp = await request.post(`/api/repos/${repoId}/sync`);
    expect(syncResp.ok()).toBe(true);
    const body = (await syncResp.json()) as SyncResponse;

    // The sync response must include these fields (they correspond to the granular
    // "Saving N backup reports..." substep messages added in issue #124).
    expect(typeof body.imported).toBe('number');
    expect(typeof body.removed).toBe('number');
    expect(typeof body.duration_secs).toBe('number');
    expect(body.duration_secs).toBeGreaterThanOrEqual(0);
  });

  test('reset-import clears importing flag', async ({ request }) => {
    const listResp = await request.get('/api/repos');
    expect(listResp.ok()).toBe(true);
    const repos = (await listResp.json()) as RepoListEntry[];
    const repo = repos.find((r) => !r.importing);
    expect(repo).toBeDefined();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const repoId = repo!.id;

    const resetResp = await request.post(
      `/api/repos/${repoId}/reset-import`,
    );
    expect(resetResp.status()).toBe(204);

    const detail = (
      await (await request.get(`/api/repos/${repoId}`)).json()
    ) as RepoDetail;
    expect(detail.importing).toBe(false);
    expect(detail.import_error).toBeNull();
  });
});
