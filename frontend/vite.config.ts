// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import istanbul from 'vite-plugin-istanbul'

const withCoverage = process.env.VITE_COVERAGE === 'true'

export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
    ...(withCoverage
      ? [
          istanbul({
            include: ['src/**/*'],
            exclude: ['node_modules', '**/*.spec.ts', '**/*.test.ts'],
            forceBuildInstrument: true,
          }),
        ]
      : []),
  ],
  build: {
    sourcemap: withCoverage ? 'inline' : false,
  },
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
      '/ws': { target: 'ws://localhost:8080', ws: true },
    },
  },
  test: {
    environment: 'happy-dom',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    coverage: {
      provider: 'v8',
      reporter: ['lcov', 'text'],
      reportsDirectory: 'coverage',
    },
  },
})
