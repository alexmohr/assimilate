// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { createApp } from 'vue'
import { createPinia } from 'pinia'
import PrimeVue from 'primevue/config'
import './style.css'
import App from './App.vue'
import { router } from './router'
import { globalPrimeVuePT } from './primevue-pt'
import { captureGlobalErrors } from './utils/clientLog'

// Uncaught errors land in the Activity page's Browser logs tab too. Installed
// once for the life of the page, so the returned remover is not needed.
captureGlobalErrors(window)

createApp(App)
  .use(createPinia())
  .use(router)
  .use(PrimeVue, { unstyled: true, pt: globalPrimeVuePT })
  .mount('#app')

if ('serviceWorker' in navigator) {
  navigator.serviceWorker.register('/sw.js').catch(() => undefined)
}
