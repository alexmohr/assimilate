// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { AxiosError, AxiosHeaders } from 'axios'
import type { ArchiveEntryResponse } from '../types/generated'

vi.mock('../api/archives', () => ({ getArchiveFile: vi.fn() }))
vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}))

import { getArchiveFile } from '../api/archives'
import BaseModal from './BaseModal.vue'
import ImageDiffDialog from './ImageDiffDialog.vue'

function archive(name: string, start: string): ArchiveEntryResponse {
  return {
    name,
    start,
    hostname: 'web-01',
    comment: '',
    original_size: 0,
    deduplicated_size: 0,
    matched: true,
    agent_hostname: 'web-01',
    agent_domain: null,
  }
}

const BEFORE = archive('web-01-mon', '2026-10-13T03:00:00Z')
const AFTER = archive('web-01-tue', '2026-10-14T03:00:00Z')

/** What each archive holds at the path: its bytes and the image's pixel size, or no image. */
interface FakeFile {
  bytes: number
  width: number
  height: number
  broken?: boolean
}

let files: Record<string, FakeFile> = {}
/** Object URL -> the file it was made for, read by the fake `Image`. */
const urls = new Map<string, FakeFile>()
let urlCount = 0

/** jsdom never decodes images, so `Image` reports the size the fake file says. */
class FakeImage {
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  naturalWidth = 0
  naturalHeight = 0

  set src(url: string) {
    const file = urls.get(url)
    queueMicrotask(() => {
      if (file === undefined || file.broken === true) {
        this.onerror?.()
        return
      }
      this.naturalWidth = file.width
      this.naturalHeight = file.height
      this.onload?.()
    })
  }
}

const createObjectURL = vi.fn((blob: Blob) => {
  const url = `blob:${++urlCount}`
  urls.set(url, (blob as Blob & { fake: FakeFile }).fake)
  return url
})
const revokeObjectURL = vi.fn()

let wrapper: VueWrapper | null = null

async function open(props: Record<string, unknown> = {}): Promise<VueWrapper> {
  wrapper = mount(ImageDiffDialog, {
    props: {
      open: true,
      repoId: 3,
      archive: AFTER,
      base: BEFORE,
      path: 'srv/www/logo.png',
      ...props,
    },
    global: { stubs: { teleport: true } },
  })
  await flushPromises()
  return wrapper
}

async function pickMode(w: VueWrapper, label: string): Promise<void> {
  await w
    .findAll('.segmented-option')
    .find((b) => b.text() === label)!
    .trigger('click')
}

/**
 * The summary line's parts as read aloud: the arrow between before and after
 * is an icon (labelled "to"), shown here as `→`.
 */
function summary(w: VueWrapper): string[] {
  return w.findAll('.image-diff-summary > span').map((span) =>
    Array.from(span.element.childNodes)
      .map((n) => (n.nodeName.toLowerCase() === 'svg' ? '→' : (n.textContent ?? '')))
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim(),
  )
}

describe('ImageDiffDialog', () => {
  beforeEach(() => {
    vi.mocked(getArchiveFile).mockReset()
    files = {
      [BEFORE.name]: { bytes: 1024, width: 800, height: 600 },
      [AFTER.name]: { bytes: 2048, width: 1024, height: 768 },
    }
    urls.clear()
    urlCount = 0
    createObjectURL.mockClear()
    revokeObjectURL.mockClear()
    vi.mocked(getArchiveFile).mockImplementation((_repo, name) => {
      const file = files[name]!
      const blob = new Blob([new Uint8Array(file.bytes)]) as Blob & { fake: FakeFile }
      blob.fake = file
      return Promise.resolve(blob)
    })
    vi.stubGlobal('Image', FakeImage)
    URL.createObjectURL = createObjectURL
    URL.revokeObjectURL = revokeObjectURL
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    vi.unstubAllGlobals()
  })

  it('reads the file from both archives, the base as before', async () => {
    const w = await open()

    expect(getArchiveFile).toHaveBeenCalledTimes(2)
    expect(getArchiveFile).toHaveBeenCalledWith(3, 'web-01-mon', 'srv/www/logo.png')
    expect(getArchiveFile).toHaveBeenCalledWith(3, 'web-01-tue', 'srv/www/logo.png')
    expect(w.text()).toContain('Compare logo.png')
    expect(w.get('.image-diff-path').text()).toBe('srv/www/logo.png')
  })

  it('shows both images side by side by default, each labelled', async () => {
    const w = await open()

    const modes = w.findAll('.segmented-option')
    expect(modes.map((m) => m.text())).toEqual(['2-up', 'Swipe', 'Onion skin'])
    expect(modes[0]!.attributes('aria-checked')).toBe('true')

    const before = w.get('.image-diff-side--before')
    const after = w.get('.image-diff-side--after')
    expect(before.get('img').attributes('alt')).toBe('logo.png before')
    expect(after.get('img').attributes('alt')).toBe('logo.png after')
    expect(before.get('img').attributes('src')).not.toBe(after.get('img').attributes('src'))
    expect(before.text()).toContain('Before')
    expect(before.text()).toContain('web-01-mon')
    expect(before.text()).toContain('800 x 600 · 1.0 KB')
    expect(after.text()).toContain('After')
    expect(after.text()).toContain('web-01-tue')
    expect(after.text()).toContain('1024 x 768 · 2.0 KB')
  })

  it('sums up how the size and dimensions changed', async () => {
    const w = await open()

    expect(summary(w)).toEqual(['800 x 600 → 1024 x 768', '1.0 KB → 2.0 KB', '+1.0 KB'])
  })

  it('marks a shrink with a minus and leaves an unchanged size out', async () => {
    files[AFTER.name] = { bytes: 512, width: 800, height: 600 }
    let w = await open()
    expect(summary(w).at(-1)).toBe('−512.0 B')
    w.unmount()

    files[AFTER.name] = { bytes: 1024, width: 400, height: 300 }
    w = await open()
    expect(summary(w)).toEqual(['800 x 600 → 400 x 300', '1.0 KB → 1.0 KB'])
  })

  it('wipes the new image across the old one in Swipe mode', async () => {
    const w = await open()
    await pickMode(w, 'Swipe')

    expect(w.find('.image-diff-two').exists()).toBe(false)
    // The stage fits the larger of the two images.
    const stage = w.get('.image-diff-stage')
    expect(stage.attributes('style')).toContain('aspect-ratio: 1024 / 768')
    expect(stage.attributes('style')).toContain('max-width: 1024px')
    expect(w.get('.image-diff-wipe').attributes('style')).toContain('inset(0 50% 0 0)')
    expect(w.get('.image-diff-handle').attributes('style')).toContain('left: 50%')

    const slider = w.get('input[aria-label="How much of the new image to show"]')
    await slider.setValue('80')
    expect(w.get('.image-diff-wipe').attributes('style')).toContain('inset(0 20% 0 0)')
    expect(w.get('.image-diff-handle').attributes('style')).toContain('left: 80%')
    expect(w.find('input[aria-label="Opacity of the new image"]').exists()).toBe(false)
  })

  it('fades the new image over the old one in Onion skin mode', async () => {
    const w = await open()
    await pickMode(w, 'Onion skin')

    expect(w.find('.image-diff-wipe').exists()).toBe(false)
    expect(w.find('.image-diff-handle').exists()).toBe(false)
    const after = w.get('img[alt="logo.png after"]')
    expect(after.attributes('style')).toContain('opacity: 0.5')

    await w.get('input[aria-label="Opacity of the new image"]').setValue('25')
    expect(w.get('img[alt="logo.png after"]').attributes('style')).toContain('opacity: 0.25')
  })

  it('keeps the swipe and opacity apart, and goes back to 2-up', async () => {
    const w = await open()
    await pickMode(w, 'Swipe')
    await w.get('input[aria-label="How much of the new image to show"]').setValue('10')
    await pickMode(w, 'Onion skin')
    expect(w.get('img[alt="logo.png after"]').attributes('style')).toContain('opacity: 0.5')

    await pickMode(w, '2-up')
    expect(w.find('.image-diff-two').exists()).toBe(true)
    expect(w.find('.image-diff-stage').exists()).toBe(false)
  })

  it('starts again at 2-up and halfway for another file', async () => {
    const w = await open()
    await pickMode(w, 'Swipe')
    await w.get('input[aria-label="How much of the new image to show"]').setValue('90')

    await w.setProps({ path: 'srv/www/icon.png' })
    await flushPromises()

    expect(getArchiveFile).toHaveBeenLastCalledWith(3, expect.any(String), 'srv/www/icon.png')
    expect(w.find('.image-diff-two').exists()).toBe(true)
    await pickMode(w, 'Swipe')
    expect(w.get('.image-diff-handle').attributes('style')).toContain('left: 50%')
  })

  it('shows a spinner until both sides are read', async () => {
    vi.mocked(getArchiveFile).mockReturnValue(new Promise(() => undefined))
    const w = await open()

    expect(w.find('.spinner, [role="status"]').exists()).toBe(true)
    expect(w.find('img').exists()).toBe(false)
  })

  it("shows the server's reason for a refused read, not the bare status", async () => {
    const body = new Blob([JSON.stringify({ error: 'archive is locked' })], {
      type: 'application/json',
    })
    vi.mocked(getArchiveFile).mockRejectedValue(
      new AxiosError('Request failed with status code 409', 'ERR_BAD_REQUEST', undefined, null, {
        status: 409,
        statusText: 'Conflict',
        data: body,
        headers: {},
        config: { headers: new AxiosHeaders() },
      }),
    )
    const w = await open()

    expect(w.get('.form-error').text()).toContain('archive is locked')
  })

  it('says when the file cannot be read', async () => {
    vi.mocked(getArchiveFile).mockRejectedValue(new Error('borg extract failed'))
    const w = await open()

    expect(w.get('.form-error').text()).toBe('borg extract failed')
    expect(w.find('img').exists()).toBe(false)
  })

  it('says when the file is not an image the browser can show', async () => {
    files[BEFORE.name] = { bytes: 10, width: 0, height: 0, broken: true }
    const w = await open()

    expect(w.get('.form-error').text()).toBe('The file could not be shown as an image.')
  })

  it('frees both images when one of them cannot be shown', async () => {
    files[BEFORE.name] = { bytes: 10, width: 0, height: 0, broken: true }
    await open()

    // One side decoded and the other did not: neither is shown, so neither
    // object URL may stay alive for the rest of the page.
    const made = createObjectURL.mock.results.map((r) => r.value as string)
    expect(made).toHaveLength(2)
    expect(revokeObjectURL.mock.calls.map((c) => c[0]).sort()).toEqual([...made].sort())
  })

  it('drops a comparison that finishes after another file was opened', async () => {
    let answerFirst: () => void = () => undefined
    const firstCalls: Blob[] = []
    vi.mocked(getArchiveFile).mockImplementationOnce(
      (_repo, name) =>
        new Promise((resolve) => {
          answerFirst = () => {
            const blob = new Blob([new Uint8Array(files[name]!.bytes)]) as Blob & {
              fake: FakeFile
            }
            blob.fake = files[name]!
            firstCalls.push(blob)
            resolve(blob)
          }
        }),
    )
    const w = await open()

    files[BEFORE.name] = { bytes: 100, width: 10, height: 10 }
    files[AFTER.name] = { bytes: 200, width: 20, height: 20 }
    await w.setProps({ path: 'srv/www/icon.png' })
    await flushPromises()
    expect(summary(w)[0]).toBe('10 x 10 → 20 x 20')

    revokeObjectURL.mockClear()
    answerFirst()
    await flushPromises()

    // The late answer is thrown away, its object URLs with it.
    expect(summary(w)[0]).toBe('10 x 10 → 20 x 20')
    expect(revokeObjectURL).toHaveBeenCalledTimes(2)
  })

  it('asks to close, and lets go of the images once closed or unmounted', async () => {
    const w = await open()
    w.findComponent(BaseModal).vm.$emit('close')
    expect(w.emitted('close')).toHaveLength(1)

    const made = createObjectURL.mock.results.map((r) => r.value as string)
    expect(made).toHaveLength(2)
    await w.setProps({ open: false })
    expect(revokeObjectURL.mock.calls.map((c) => c[0])).toEqual(made)

    await w.setProps({ open: true })
    await flushPromises()
    revokeObjectURL.mockClear()
    w.unmount()
    wrapper = null
    expect(revokeObjectURL).toHaveBeenCalledTimes(2)
  })
})
