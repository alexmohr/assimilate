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
    var btn = document.getElementById('themeToggle')
    if (btn) {
      btn.addEventListener('click', function () {
        var isDark = root.classList.toggle('dark')
        localStorage.setItem(STORAGE_KEY, isDark ? 'dark' : 'light')
      })
    }

    var tabs = document.querySelectorAll('.qs-tab')
    var panels = document.querySelectorAll('.qs-panel')

    tabs.forEach(function (tab) {
      tab.addEventListener('click', function () {
        var target = tab.getAttribute('data-tab')
        tabs.forEach(function (t) { t.classList.remove('active') })
        panels.forEach(function (p) { p.classList.remove('active') })
        tab.classList.add('active')
        document.querySelector('[data-panel="' + target + '"]').classList.add('active')
      })
    })

    var prefersReduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (!prefersReduced) {
      var observer = new IntersectionObserver(function (entries) {
        entries.forEach(function (entry) {
          if (entry.isIntersecting) {
            entry.target.classList.add('revealed')
            observer.unobserve(entry.target)
          }
        })
      }, { threshold: 0.15 })

      document.querySelectorAll('.reveal').forEach(function (el) {
        observer.observe(el)
      })
    } else {
      document.querySelectorAll('.reveal').forEach(function (el) {
        el.classList.add('revealed')
      })
    }
  })
})()
