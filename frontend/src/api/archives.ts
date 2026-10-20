// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ArchiveDatabaseDump,
  ArchiveDatabasesResponse,
  ArchiveEntryResponse,
  ArchiveIndexStatusResponse,
  ContentsResponse,
  CrossSearchResponse,
  DatabaseRestoreTarget,
  DeleteArchiveResponse,
  DiffResponse,
  FileDiffResponse,
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
}

export async function getArchiveContents(
  repoId: number,
  archiveName: string,
  options: GetArchiveContentsOptions = {},
): Promise<ContentsResponse> {
  const response = await apiClient.get<ContentsResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/contents`,
    { params: options.path ? { path: options.path } : {} },
  )
  return response.data
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

export interface DiffArchiveFileOptions {
  /** Path of the file within both archives. */
  path: string
  /** The older archive: the base whose lines the diff removes. */
  against: string
  /** Repository `against` is in, when it is not `repoId`. */
  againstRepoId?: number
  /** Every line of the file instead of only the changes. */
  wholeFile?: boolean
}

/** Compares one file of `archiveName` (the target) with the same path in `against`. */
export async function diffArchiveFile(
  repoId: number,
  archiveName: string,
  options: DiffArchiveFileOptions,
): Promise<FileDiffResponse> {
  const response = await apiClient.get<FileDiffResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/file-diff`,
    {
      params: {
        path: options.path,
        against: options.against,
        against_repo_id: options.againstRepoId,
        whole_file: options.wholeFile === true ? true : undefined,
      },
    },
  )
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
  paths: string[]
  target_path: string
  hostname: string
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

/** The database dumps an archive holds, in the order its manifests list them. */
export async function listArchiveDatabases(
  repoId: number,
  archiveName: string,
): Promise<ArchiveDatabaseDump[]> {
  const response = await apiClient.get<ArchiveDatabasesResponse>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/databases`,
  )
  return response.data.dumps
}

export interface RestoreArchiveDatabaseRequest {
  /** The dump's path in the archive, as the dump listing names it. */
  archive_path: string
  target: DatabaseRestoreTarget
  /** The agent that runs the restore. */
  hostname: string
  /** Tells apart agents sharing `hostname`; null when none needs telling apart. */
  domain: string | null
}

/**
 * Records a restore of one database dump and returns it. Like a file
 * restore it runs in the background; follow it with `useRestoreRun`.
 */
export async function restoreArchiveDatabase(
  repoId: number,
  archiveName: string,
  data: RestoreArchiveDatabaseRequest,
): Promise<RestoreRun> {
  const response = await apiClient.post<RestoreRun>(
    `/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/databases/restore`,
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
