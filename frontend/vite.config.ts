// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { coverageConfigDefaults, defineConfig } from 'vitest/config'
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
            // wasm-bindgen glue is generated; the code behind it is covered
            // through src/wasm/domain.ts's tests and crates/domain's.
            exclude: ['node_modules', '**/*.spec.ts', '**/*.test.ts', 'src/wasm/generated/**'],
            forceBuildInstrument: true,
          }),
        ]
      : []),
  ],
  // Lets src/wasm/domain.ts import the WebAssembly module as a `?inline` data URL.
  assetsInclude: ['**/*.wasm'],
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
    setupFiles: ['src/test-utils/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    coverage: {
      provider: 'v8',
      reporter: ['lcov', 'text'],
      reportsDirectory: 'coverage',
      exclude: [...coverageConfigDefaults.exclude, 'src/wasm/generated/**'],
    },
  },
})
