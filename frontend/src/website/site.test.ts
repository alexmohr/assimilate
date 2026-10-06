// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Unit tests for the project website's script (website/assets/site.js), which
// sits outside the Vue app. frontend/e2e/website.spec.ts covers the same
// behaviours on the rendered pages in a real browser.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { initCopyButtons, initNavToggle, initReveal } from '../../../website/assets/site.js'

beforeEach(() => {
  document.body.innerHTML = ''
  document.documentElement.className = ''
})

afterEach(() => {
  vi.useRealTimers()
})

describe('initNavToggle', () => {
  it('opens and closes the navigation and mirrors it in aria-expanded', () => {
    document.body.innerHTML = `
      <button class="nav-toggle" aria-expanded="false">Menu</button>
      <nav id="site-nav"></nav>`
    initNavToggle(document)
    const toggle = document.querySelector<HTMLButtonElement>('.nav-toggle')!
    const nav = document.getElementById('site-nav')!

    toggle.click()
    expect(nav.classList.contains('open')).toBe(true)
    expect(toggle.getAttribute('aria-expanded')).toBe('true')

    toggle.click()
    expect(nav.classList.contains('open')).toBe(false)
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
  })

  it('does nothing on a page without the toggle', () => {
    document.body.innerHTML = '<nav id="site-nav"></nav>'
    expect(() => initNavToggle(document)).not.toThrow()
  })
})

describe('initCopyButtons', () => {
  function setUp(): { button: HTMLButtonElement; code: HTMLElement } {
    document.body.innerHTML = `
      <pre id="snippet">  docker compose up -d  </pre>
      <button data-copy="snippet">Copy</button>`
    return {
      button: document.querySelector<HTMLButtonElement>('[data-copy]')!,
      code: document.getElementById('snippet')!,
    }
  }

  async function click(button: HTMLButtonElement): Promise<void> {
    button.click()
    // Let the awaited clipboard write settle.
    await vi.advanceTimersByTimeAsync(0)
  }

  it('copies the trimmed text and restores the label after the reset delay', async () => {
    vi.useFakeTimers()
    const { button, code } = setUp()
    code.innerText = '  docker compose up -d  '
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    initCopyButtons(document, clipboard, 1000)

    await click(button)
    expect(clipboard.writeText).toHaveBeenCalledWith('docker compose up -d')
    expect(button.textContent).toBe('Copied')

    await vi.advanceTimersByTimeAsync(1000)
    expect(button.textContent).toBe('Copy')
  })

  it('restores the original label after a second click within the reset delay', async () => {
    vi.useFakeTimers()
    const { button } = setUp()
    initCopyButtons(document, { writeText: vi.fn().mockResolvedValue(undefined) }, 1000)

    await click(button)
    await vi.advanceTimersByTimeAsync(500)
    await click(button)
    expect(button.textContent).toBe('Copied')

    // The first click's reset no longer fires early...
    await vi.advanceTimersByTimeAsync(600)
    expect(button.textContent).toBe('Copied')
    // ...and the label comes back as "Copy", not the "Copied" seen mid-way.
    await vi.advanceTimersByTimeAsync(400)
    expect(button.textContent).toBe('Copy')
  })

  it('selects the text for a manual copy when the clipboard write fails', async () => {
    vi.useFakeTimers()
    const { button, code } = setUp()
    initCopyButtons(document, { writeText: vi.fn().mockRejectedValue(new Error('denied')) }, 1000)

    await click(button)
    expect(button.textContent).toBe('Press Ctrl+C')
    expect(document.getSelection()?.toString()).toBe(code.textContent)

    await vi.advanceTimersByTimeAsync(1000)
    expect(button.textContent).toBe('Copy')
  })

  it('falls back to selecting when there is no clipboard at all', async () => {
    vi.useFakeTimers()
    const { button } = setUp()
    initCopyButtons(document, undefined, 1000)

    await click(button)
    expect(button.textContent).toBe('Press Ctrl+C')
  })

  it('ignores a button whose target does not exist', async () => {
    vi.useFakeTimers()
    document.body.innerHTML = '<button data-copy="missing">Copy</button>'
    const clipboard = { writeText: vi.fn() }
    initCopyButtons(document, clipboard, 1000)

    await click(document.querySelector<HTMLButtonElement>('[data-copy]')!)
    expect(clipboard.writeText).not.toHaveBeenCalled()
    expect(document.querySelector('[data-copy]')!.textContent).toBe('Copy')
  })
})

describe('initReveal', () => {
  type ObserverCallback = (entries: { isIntersecting: boolean; target: Element }[]) => void

  /** A window stand-in whose IntersectionObserver records what it watches. */
  function fakeWindow(reduceMotion: boolean) {
    const observed: Element[] = []
    const unobserved: Element[] = []
    let callback: ObserverCallback = () => {}
    class FakeObserver {
      constructor(cb: ObserverCallback) {
        callback = cb
      }
      observe(el: Element): void {
        observed.push(el)
      }
      unobserve(el: Element): void {
        unobserved.push(el)
      }
    }
    const win = {
      innerHeight: 800,
      matchMedia: () => ({ matches: reduceMotion }),
      IntersectionObserver: FakeObserver,
    }
    return { win, observed, unobserved, fire: (e: Parameters<ObserverCallback>[0]) => callback(e) }
  }

  function revealAt(top: number): HTMLElement {
    const el = document.createElement('section')
    el.dataset.reveal = ''
    el.getBoundingClientRect = () => ({ top }) as DOMRect
    document.body.append(el)
    return el
  }

  it('hides only elements below the fold and shows them once they scroll in', () => {
    const above = revealAt(100)
    const below = revealAt(1200)
    const { win, observed, unobserved, fire } = fakeWindow(false)

    initReveal(document, win)

    expect(document.documentElement.classList.contains('js')).toBe(true)
    expect(above.classList.contains('pending')).toBe(false)
    expect(below.classList.contains('pending')).toBe(true)
    expect(observed).toEqual([below])

    fire([{ isIntersecting: false, target: below }])
    expect(below.classList.contains('pending')).toBe(true)

    fire([{ isIntersecting: true, target: below }])
    expect(below.classList.contains('pending')).toBe(false)
    expect(unobserved).toEqual([below])
  })

  it('leaves everything visible for reduced-motion visitors', () => {
    const below = revealAt(1200)
    const { win, observed } = fakeWindow(true)

    initReveal(document, win)

    expect(document.documentElement.classList.contains('js')).toBe(false)
    expect(below.classList.contains('pending')).toBe(false)
    expect(observed).toEqual([])
  })

  it('leaves everything visible in browsers without IntersectionObserver', () => {
    const below = revealAt(1200)
    const win = { innerHeight: 800, matchMedia: () => ({ matches: false }) }

    initReveal(document, win)

    expect(document.documentElement.classList.contains('js')).toBe(false)
    expect(below.classList.contains('pending')).toBe(false)
  })
})
