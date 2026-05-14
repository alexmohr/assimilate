// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

(function () {
  const STORAGE_KEY = 'assimilate-theme'
  const root = document.documentElement

  function applyTheme(dark) {
    root.classList.toggle('dark', dark)
  }

  function savedTheme() {
    return localStorage.getItem(STORAGE_KEY)
  }

  function prefersDark() {
    return window.matchMedia('(prefers-color-scheme: dark)').matches
  }

  const initial = savedTheme() === 'dark' || (savedTheme() === null && prefersDark())
  applyTheme(initial)

  document.addEventListener('DOMContentLoaded', function () {
    const btn = document.getElementById('themeToggle')
    if (!btn) return

    btn.addEventListener('click', function () {
      const isDark = root.classList.toggle('dark')
      localStorage.setItem(STORAGE_KEY, isDark ? 'dark' : 'light')
    })

    const tabs = document.querySelectorAll('.qs-tab')
    const panels = document.querySelectorAll('.qs-panel')

    tabs.forEach(function (tab) {
      tab.addEventListener('click', function () {
        const target = tab.getAttribute('data-tab')
        tabs.forEach(function (t) { t.classList.remove('active') })
        panels.forEach(function (p) { p.classList.remove('active') })
        tab.classList.add('active')
        document.querySelector('[data-panel="' + target + '"]').classList.add('active')
      })
    })
  })
})()
