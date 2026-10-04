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
  button.addEventListener('click', async () => {
    const target = document.getElementById(button.dataset.copy)
    if (!target) return
    const label = button.textContent
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
    setTimeout(() => {
      button.textContent = label
    }, 1600)
  })
}
