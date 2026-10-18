// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ArchiveEntryResponse,
  ArchiveIndexStatusResponse,
  ArchiveTreeSearchResponse,
  ArchiveTreeTotalsResponse,
  ContentsResponse,
  CrossSearchResponse,
  DeleteArchiveResponse,
  DiffResponse,
  RestoreRun,
} from '../types/generated'

// The `/repos/:id/archives/:archive/search` endpoint (single-archive search)
// serializes its own local response type, distinct from - and not matching
// the field names of - the generated `SearchResponse`/`SearchEntry` (those
// are derived from an unrelated shared type whose `entry_type` field isn't
// renamed to `type` on the wire). Defined locally here to match the real
// response shape.
export interface ArchiveSearchEntry {
  path: string
  size: number
  mtime: string
  type: string
}

export interface ArchiveSearchResponse {
  items: ArchiveSearchEntry[]
  total_matched: number
  limit: number
  offset: number
}

export async function listRepoArchives(repoId: number): Promise<ArchiveEntryResponse[]> {
  const response = await apiClient.get<ArchiveEntryResponse[]>(`/repos/${repoId}/archives`)
  return response.data
}

export async function getArchiveIndexStatus(
  repoId: number,
  archiveName: string,
): Promise<ArchiveIndexStatusResponse> {
  const response = await apiClient.get<ArchiveIndexStatusResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/index-status`,
  )
  return response.data
}

export interface GetArchiveContentsOptions {
  path?: string
  /** Most entries to return; the server's default is 100. */
  limit?: number
}

export async function getArchiveContents(
  repoId: number,
  archiveName: string,
  options: GetArchiveContentsOptions = {},
): Promise<ContentsResponse> {
  const params: Record<string, string | number> = {}
  if (options.path) params.path = options.path
  if (options.limit !== undefined) params.limit = options.limit
  const response = await apiClient.get<ContentsResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/contents`,
    { params },
  )
  return response.data
}

/** Which entries a tree search keeps; an unset field does not narrow it. */
export interface ArchiveTreeSearchOptions {
  /** Text the path must contain, ignoring case. */
  query?: string
  kind?: 'file' | 'dir'
  minSize?: number
  maxSize?: number
  /** `YYYY-MM-DD`, inclusive. */
  modifiedFrom?: string
  /** `YYYY-MM-DD`, inclusive. */
  modifiedTo?: string
  limit?: number
}

/** Searches the archive's content index, across the whole archive. */
export async function searchArchiveTree(
  repoId: number,
  archiveName: string,
  options: ArchiveTreeSearchOptions,
  signal?: AbortSignal,
): Promise<ArchiveTreeSearchResponse> {
  const response = await apiClient.get<ArchiveTreeSearchResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/tree-search`,
    {
      params: {
        q: options.query,
        kind: options.kind,
        min_size: options.minSize,
        max_size: options.maxSize,
        modified_from: options.modifiedFrom,
        modified_to: options.modifiedTo,
        limit: options.limit,
      },
      signal,
    },
  )
  return response.data
}

/** Counts the files and bytes at or under each path, from the content index. */
export async function getArchiveTreeTotals(
  repoId: number,
  archiveName: string,
  paths: string[],
): Promise<ArchiveTreeTotalsResponse> {
  const response = await apiClient.post<ArchiveTreeTotalsResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/tree-totals`,
    { paths },
  )
  return response.data
}

/**
 * A link that downloads `paths` as one tar.lz4. The browser saves the
 * stream as it arrives, so a large basket never has to fit in memory.
 */
export function archiveFilesDownloadUrl(
  repoId: number,
  archiveName: string,
  paths: string[],
): string {
  const query = new URLSearchParams({ paths: JSON.stringify(paths) })
  return `/api/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/download?${query.toString()}`
}

export interface SearchArchiveOptions {
  pattern: string
  limit?: number
  offset?: number
}

export async function searchArchive(
  repoId: number,
  archiveName: string,
  options: SearchArchiveOptions,
): Promise<ArchiveSearchResponse> {
  const response = await apiClient.get<ArchiveSearchResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/search`,
    { params: { pattern: options.pattern, limit: options.limit, offset: options.offset } },
  )
  return response.data
}

export interface SearchAcrossArchivesOptions {
  pattern: string
  maxArchives?: number
}

export async function searchAcrossArchives(
  repoId: number,
  options: SearchAcrossArchivesOptions,
): Promise<CrossSearchResponse> {
  const response = await apiClient.get<CrossSearchResponse>(`/repos/${repoId}/search`, {
    params: { pattern: options.pattern, max_archives: options.maxArchives },
  })
  return response.data
}

export interface DiffArchivesOptions {
  archive1: string
  archive2: string
}

export async function diffArchives(
  repoId: number,
  options: DiffArchivesOptions,
): Promise<DiffResponse> {
  const response = await apiClient.get<DiffResponse>(`/repos/${repoId}/archives/diff`, {
    params: { archive1: options.archive1, archive2: options.archive2 },
  })
  return response.data
}

export async function downloadArchiveFiles(
  repoId: number,
  archiveName: string,
  paths: string[],
  signal?: AbortSignal,
): Promise<Blob> {
  const response = await apiClient.post<Blob>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/download`,
    { paths },
    { responseType: 'blob', signal },
  )
  return response.data
}

export interface RestoreArchiveFilesRequest {
  /** Archive paths restored by one borg run; empty restores the whole archive. */
  paths: string[]
  /** Absolute directory everything is restored under; `/` is the original locations. */
  target_path: string
  hostname: string
  /** Tells apart agents sharing `hostname`. */
  domain?: string | null
}

/**
 * Records a restore onto an agent and returns it. The restore runs in the
 * background: it goes to the agent at once if it is connected, otherwise
 * when it next connects. Follow it with `useRestoreRun`.
 */
export async function restoreArchiveFiles(
  repoId: number,
  archiveName: string,
  data: RestoreArchiveFilesRequest,
): Promise<RestoreRun> {
  const response = await apiClient.post<RestoreRun>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/restore`,
    data,
  )
  return response.data
}

export async function deleteArchive(
  repoId: number,
  archiveName: string,
): Promise<DeleteArchiveResponse> {
  const response = await apiClient.delete<DeleteArchiveResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}`,
  )
  return response.data
}
