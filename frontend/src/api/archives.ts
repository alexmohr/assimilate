// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ArchiveEntryResponse,
  ArchiveIndexStatusResponse,
  ContentsResponse,
  CrossSearchResponse,
  DeleteArchiveResponse,
  DiffResponse,
  RestoreFilesResponse,
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

export async function restoreArchiveFiles(
  repoId: number,
  archiveName: string,
  data: RestoreArchiveFilesRequest,
): Promise<RestoreFilesResponse> {
  const response = await apiClient.post<RestoreFilesResponse>(
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
