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
  breadcrumbs: ComputedRef<BreadcrumbSegment[]>
  dirs: ComputedRef<DirDisplayEntry[]>
  files: ComputedRef<ContentEntry[]>
  loadArchives: () => Promise<void>
  selectArchive: (archive: ArchiveEntry) => Promise<void>
  loadContents: (path: string) => Promise<void>
  navigateTo: (path: string) => void
  entryName: (entry: ContentEntry) => string
  downloadEntry: (entry: ContentEntry) => void
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

    if (currentPath.value !== '/') {
      const parentPath = currentPath.value.replace(/\/[^/]+$/, '') || '/'
      const currentEntry = contents.value.find((e) => e.type === 'd' && e.path === currentDir)
      if (currentEntry) {
        entries.push({ ...currentEntry, displayName: '.' })
      }

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
      .filter((e) => e.type === 'd' && e.path !== currentDir)
      .sort((a, b) => a.path.localeCompare(b.path))
    return [
      ...entries,
      ...childDirs.map((e) => ({ ...e, displayName: e.path.split('/').pop() ?? e.path })),
    ]
  })

  const files = computed<ContentEntry[]>(() =>
    contents.value.filter((e) => e.type !== 'd').sort((a, b) => a.path.localeCompare(b.path)),
  )

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
      const res = await apiClient.get<ContentEntry[]>(
        `/repos/${repoId.value}/archives/${encodeURIComponent(selectedArchive.value.name)}/contents`,
        { params: apiPath ? { path: apiPath } : {} },
      )
      contents.value = res.data
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
      ? `/api/repos/${repoId.value}/archives/${archiveName}/export?path=${encodedPath}`
      : `/api/repos/${repoId.value}/archives/${archiveName}/extract?path=${encodedPath}`
    const a = document.createElement('a')
    a.href = url
    a.download = isDir ? `${entryName(entry)}.tar.lz4` : entryName(entry)
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
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
    breadcrumbs,
    dirs,
    files,
    loadArchives,
    selectArchive,
    loadContents,
    navigateTo,
    entryName,
    downloadEntry,
  }
}
