// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Mobile navigation toggle and copy buttons for code blocks.

const toggle = document.querySelector('.nav-toggle')
const nav = document.getElementById('site-nav')

if (toggle && nav) {
  toggle.addEventListener('click', () => {
    const open = nav.classList.toggle('open')
    toggle.setAttribute('aria-expanded', String(open))
  })
}

for (const button of document.querySelectorAll('[data-copy]')) {
  // Read once: a second click before the reset would otherwise take
  // "Copied" for the button's own label and leave it stuck there.
  const label = button.textContent
  let reset
  button.addEventListener('click', async () => {
    const target = document.getElementById(button.dataset.copy)
    if (!target) return
    try {
      await navigator.clipboard.writeText(target.innerText.trim())
      button.textContent = 'Copied'
    } catch {
      const range = document.createRange()
      range.selectNodeContents(target)
      const selection = window.getSelection()
      selection.removeAllRanges()
      selection.addRange(range)
      button.textContent = 'Press Ctrl+C'
    }
    clearTimeout(reset)
    reset = setTimeout(() => {
      button.textContent = label
    }, 1600)
  })
}

// Scroll reveal. Elements already on screen when the page loads stay
// visible; only those further down start hidden and ease in once scrolled
// to. Skipped entirely for reduced-motion visitors and old browsers.
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
if (!reduceMotion && 'IntersectionObserver' in window) {
  document.documentElement.classList.add('js')
  const observer = new IntersectionObserver(
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
  for (const el of document.querySelectorAll('[data-reveal]')) {
    if (el.getBoundingClientRect().top > window.innerHeight) {
      el.classList.add('pending')
      observer.observe(el)
    }
  }
}
