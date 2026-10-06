// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Mobile navigation toggle, copy buttons for code blocks, and scroll reveal.
// Each behaviour is an exported function so the unit tests in
// frontend/src/website/site.test.ts can run it against their own document;
// the page runs all three at the bottom of this file.

/** Opens and closes the mobile navigation from its toggle button. */
export function initNavToggle(doc) {
  const toggle = doc.querySelector('.nav-toggle')
  const nav = doc.getElementById('site-nav')
  if (!toggle || !nav) return
  toggle.addEventListener('click', () => {
    const open = nav.classList.toggle('open')
    toggle.setAttribute('aria-expanded', String(open))
  })
}

/**
 * Copies the element a `data-copy` button names to the clipboard, or selects
 * it when the clipboard is unavailable (a non-secure page has none), and
 * shows that on the button for `resetMs` before restoring its label.
 */
export function initCopyButtons(doc, clipboard, resetMs = 1600) {
  for (const button of doc.querySelectorAll('[data-copy]')) {
    // Read once: a second click before the reset would otherwise take
    // "Copied" for the button's own label and leave it stuck there.
    const label = button.textContent
    let reset
    button.addEventListener('click', async () => {
      const target = doc.getElementById(button.dataset.copy)
      if (!target) return
      try {
        await clipboard.writeText(target.innerText.trim())
        button.textContent = 'Copied'
      } catch {
        const range = doc.createRange()
        range.selectNodeContents(target)
        const selection = doc.getSelection()
        selection.removeAllRanges()
        selection.addRange(range)
        button.textContent = 'Press Ctrl+C'
      }
      clearTimeout(reset)
      reset = setTimeout(() => {
        button.textContent = label
      }, resetMs)
    })
  }
}

/**
 * Scroll reveal. Elements already on screen when the page loads stay
 * visible; only those further down start hidden and ease in once scrolled
 * to. Skipped entirely for reduced-motion visitors and old browsers.
 */
export function initReveal(doc, win) {
  const reduceMotion = win.matchMedia('(prefers-reduced-motion: reduce)').matches
  if (reduceMotion || !('IntersectionObserver' in win)) return
  doc.documentElement.classList.add('js')
  const observer = new win.IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.remove('pending')
          observer.unobserve(entry.target)
        }
      }
    },
    { rootMargin: '0px 0px -8% 0px' },
  )
  for (const el of doc.querySelectorAll('[data-reveal]')) {
    if (el.getBoundingClientRect().top > win.innerHeight) {
      el.classList.add('pending')
      observer.observe(el)
    }
  }
}

initNavToggle(document)
initCopyButtons(document, navigator.clipboard)
initReveal(document, window)
