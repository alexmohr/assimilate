// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, computed, type Ref, type ComputedRef } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'

export interface ArchiveEntry {
  name: string
  start: string
  hostname: string
  comment: string
  original_size: number
  deduplicated_size: number
  matched: boolean | null
  client_hostname: string | null
}

export interface ContentEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
}

interface ContentsResponse {
  index_status: 'pending' | 'indexing' | 'done' | 'failed'
  entries: ContentEntry[]
}

export interface BreadcrumbSegment {
  label: string
  path: string
}

export interface DirDisplayEntry extends ContentEntry {
  displayName: string
}

interface UseArchiveBrowserReturn {
  archives: Ref<ArchiveEntry[]>
  archivesLoading: Ref<boolean>
  archivesError: Ref<string | null>
  sortedArchives: ComputedRef<ArchiveEntry[]>
  selectedArchive: Ref<ArchiveEntry | null>
  currentPath: Ref<string>
  contents: Ref<ContentEntry[]>
  contentsLoading: Ref<boolean>
  contentsError: Ref<string | null>
  indexing: Ref<boolean>
  breadcrumbs: ComputedRef<BreadcrumbSegment[]>
  dirs: ComputedRef<DirDisplayEntry[]>
  files: ComputedRef<ContentEntry[]>
  loadArchives: () => Promise<void>
  selectArchive: (archive: ArchiveEntry) => Promise<void>
  loadContents: (path: string) => Promise<void>
  navigateTo: (path: string) => void
  entryName: (entry: ContentEntry) => string
  downloadEntry: (entry: ContentEntry) => void
  restoreEntry: (entry: ContentEntry) => Promise<boolean>
  deleteArchive: (entry: ContentEntry) => Promise<boolean>
}

export function useArchiveBrowser(repoId: Ref<number>): UseArchiveBrowserReturn {
  const archives = ref<ArchiveEntry[]>([])
  const archivesLoading = ref(false)
  const archivesError = ref<string | null>(null)
  const selectedArchive = ref<ArchiveEntry | null>(null)

  const currentPath = ref('/')
  const contents = ref<ContentEntry[]>([])
  const contentsLoading = ref(false)
  const contentsError = ref<string | null>(null)
  const indexing = ref(false)

  let pollTimer: ReturnType<typeof setInterval> | null = null

  function stopPolling(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function startPolling(archiveName: string, pendingPath: string): void {
    stopPolling()
    pollTimer = setInterval(async () => {
      try {
        const res = await apiClient.get<{ status: string; file_count?: number; error?: string }>(
          `/repos/${repoId.value}/archives/${encodeURIComponent(archiveName)}/index-status`,
        )
        if (res.data.status === 'done') {
          stopPolling()
          indexing.value = false
          await loadContents(pendingPath)
        } else if (res.data.status === 'failed') {
          stopPolling()
          indexing.value = false
          contentsError.value = res.data.error ?? 'Archive indexing failed'
        }
      } catch (e: unknown) {
        stopPolling()
        indexing.value = false
        contentsError.value = extractError(e)
      }
    }, 2000)
  }

  const sortedArchives = computed<ArchiveEntry[]>(() =>
    [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
  )

  const breadcrumbs = computed<BreadcrumbSegment[]>(() => {
    const path = currentPath.value
    if (path === '/') return [{ label: '~', path: '/' }]
    const parts = path.replace(/^\//, '').split('/')
    const segments: BreadcrumbSegment[] = [{ label: '~', path: '/' }]
    let accumulated = ''
    for (const part of parts) {
      accumulated += `/${part}`
      segments.push({ label: part, path: accumulated })
    }
    return segments
  })

  const dirs = computed<DirDisplayEntry[]>(() => {
    const currentDir = currentPath.value.replace(/^\//, '')
    const entries: DirDisplayEntry[] = []

    const currentEntry = contents.value.find((e) => e.type === 'd' && e.path === currentDir)
    if (currentEntry) {
      entries.push({ ...currentEntry, displayName: '.' })
    } else if (currentPath.value === '/') {
      entries.push({ type: 'd', path: '', size: 0, mtime: '', mode: '', displayName: '.' })
    }

    if (currentPath.value !== '/') {
      const parentPath = currentPath.value.replace(/\/[^/]+$/, '') || '/'
      entries.push({
        type: 'd',
        path: parentPath,
        size: 0,
        mtime: '',
        mode: '',
        displayName: '..',
      })
    }

    const childDirs = contents.value
      .filter((e) => {
        if (e.type !== 'd' || e.path === currentDir) return false
        const parent = e.path.includes('/') ? e.path.substring(0, e.path.lastIndexOf('/')) : ''
        return parent === currentDir
      })
      .sort((a, b) => a.path.localeCompare(b.path))
    return [
      ...entries,
      ...childDirs.map((e) => ({ ...e, displayName: e.path.split('/').pop() ?? e.path })),
    ]
  })

  const files = computed<ContentEntry[]>(() => {
    const currentDir = currentPath.value.replace(/^\//, '')
    return contents.value
      .filter((e) => {
        if (e.type === 'd') return false
        const parent = e.path.includes('/') ? e.path.substring(0, e.path.lastIndexOf('/')) : ''
        return parent === currentDir
      })
      .sort((a, b) => a.path.localeCompare(b.path))
  })

  async function loadArchives(): Promise<void> {
    archivesLoading.value = true
    archivesError.value = null
    try {
      const res = await apiClient.get<ArchiveEntry[]>(`/repos/${repoId.value}/archives`)
      archives.value = res.data
    } catch (e: unknown) {
      archivesError.value = extractError(e)
    } finally {
      archivesLoading.value = false
    }
  }

  async function selectArchive(archive: ArchiveEntry): Promise<void> {
    stopPolling()
    indexing.value = false
    selectedArchive.value = archive
    currentPath.value = '/'
    contents.value = []
    contentsError.value = null
    await loadContents('/')
  }

  async function loadContents(path: string): Promise<void> {
    if (!selectedArchive.value) return
    contentsLoading.value = true
    contentsError.value = null
    const normalizedPath = path === '/' ? '/' : `/${path.replace(/^\//, '')}`
    currentPath.value = normalizedPath
    try {
      const apiPath = normalizedPath === '/' ? undefined : normalizedPath.replace(/^\//, '')
      const res = await apiClient.get<ContentsResponse>(
        `/repos/${repoId.value}/archives/${encodeURIComponent(selectedArchive.value.name)}/contents`,
        { params: apiPath ? { path: apiPath } : {} },
      )
      const { index_status, entries } = res.data
      if (index_status === 'done' || index_status === 'failed') {
        indexing.value = false
        contents.value = entries.filter((e) => e.path !== '.' && e.path !== '..')
      } else {
        // pending or indexing — show spinner and poll
        indexing.value = true
        contents.value = []
        startPolling(selectedArchive.value.name, path)
      }
    } catch (e: unknown) {
      contentsError.value = extractError(e)
    } finally {
      contentsLoading.value = false
    }
  }

  function navigateTo(path: string): void {
    loadContents(path)
  }

  function entryName(entry: ContentEntry): string {
    return entry.path.split('/').pop() ?? entry.path
  }

  function downloadEntry(entry: ContentEntry): void {
    if (!selectedArchive.value) return
    const archiveName = encodeURIComponent(selectedArchive.value.name)
    const encodedPath = encodeURIComponent(entry.path)
    const isDir = entry.type === 'd'
    const url = isDir
      ? entry.path.length > 0
        ? `/api/repos/${repoId.value}/archives/${archiveName}/export?path=${encodeURIComponent(entry.path)}`
        : `/api/repos/${repoId.value}/archives/${archiveName}/export`
      : `/api/repos/${repoId.value}/archives/${archiveName}/extract?path=${encodedPath}`
    const a = document.createElement('a')
    a.href = url
    a.download = isDir
      ? `${entry.path.length > 0 ? entryName(entry) : selectedArchive.value.name}.tar.lz4`
      : entryName(entry)
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
  }

  async function restoreEntry(entry: ContentEntry): Promise<boolean> {
    const archive = selectedArchive.value
    if (!archive) return false

    const hostname = archive.client_hostname ?? archive.hostname
    const name = entry.path.length > 0 ? entry.path : 'the whole archive'
    if (!window.confirm(`Restore ${name} to its original location on ${hostname}?`)) return false

    const response = await apiClient.post<{ success: boolean; error_message?: string }>(
      `/repos/${repoId.value}/archives/${encodeURIComponent(archive.name)}/restore`,
      {
        paths: entry.path.length > 0 ? [entry.path] : [],
        target_path: '/',
        hostname,
      },
    )
    if (!response.data.success) {
      throw new Error(response.data.error_message ?? 'Restore failed')
    }
    return true
  }

  async function deleteArchive(entry: ContentEntry): Promise<boolean> {
    const archive = selectedArchive.value
    if (!archive || entry.type !== 'd' || entry.path.length > 0) return false

    if (!window.confirm(`Delete archive ${archive.name}? This cannot be undone.`)) return false

    const response = await apiClient.delete<{ success: boolean; archive_name: string }>(
      `/repos/${repoId.value}/archives/${encodeURIComponent(archive.name)}`,
    )

    if (!response.data.success) {
      throw new Error('Archive delete failed')
    }

    archives.value = archives.value.filter((item) => item.name !== archive.name)

    if (selectedArchive.value?.name === archive.name) {
      stopPolling()
      selectedArchive.value = null
      currentPath.value = '/'
      contents.value = []
      contentsError.value = null
      indexing.value = false
    }

    return true
  }

  return {
    archives,
    archivesLoading,
    archivesError,
    sortedArchives,
    selectedArchive,
    currentPath,
    contents,
    contentsLoading,
    contentsError,
    indexing,
    breadcrumbs,
    dirs,
    files,
    loadArchives,
    selectArchive,
    loadContents,
    navigateTo,
    entryName,
    downloadEntry,
    restoreEntry,
    deleteArchive,
  }
}
