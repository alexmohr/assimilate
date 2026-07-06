// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import eslint from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import pluginVue from 'eslint-plugin-vue'
import eslintConfigPrettier from 'eslint-config-prettier'
import { noStringLiteralControlFlow } from './eslint-rules/no-string-literal-control-flow.js'

export default tseslint.config(
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],
  eslintConfigPrettier,
  {
    languageOptions: {
      globals: {
        ...globals.browser,
      },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: ['.vue'],
      },
    },
  },
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
      },
    },
  },
  {
    plugins: {
      local: {
        rules: {
          'no-string-literal-control-flow': noStringLiteralControlFlow,
        },
      },
    },
    rules: {
      'no-console': 'warn',
      'no-debugger': 'error',
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
      '@typescript-eslint/explicit-function-return-type': [
        'error',
        {
          allowExpressions: true,
          allowTypedFunctionExpressions: true,
        },
      ],
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/consistent-type-imports': 'error',
      'vue/multi-word-component-names': 'off',
      'vue/require-default-prop': 'off',
      'vue/no-v-html': 'warn',
      // Mirrors the Rust workspace's no_string_control_flow dylint lint: control flow
      // must branch on a narrowed string-literal union/enum, not a plain `string`.
      // Type-aware (unlike a syntax-only no-restricted-syntax rule), so it doesn't
      // flag code that already compares an established literal union to its own member.
      'local/no-string-literal-control-flow': 'error',
    },
  },
  {
    files: ['**/*.test.ts', 'src/test-utils/**', 'e2e/**'],
    rules: {
      'vue/one-component-per-file': 'off',
      'local/no-string-literal-control-flow': 'off',
    },
  },
  {
    // Files outside the type-checked app project (tsconfig.app.json /
    // tsconfig.node.json): parse without a TS program so they don't need an
    // entry in either tsconfig, and skip the type-aware rule accordingly.
    files: [
      '*.js',
      '*.ts',
      'eslint-rules/**/*.js',
      'public/**/*.js',
      'src/**/*.test.ts',
      'e2e/**/*.ts',
    ],
    languageOptions: {
      parserOptions: {
        projectService: false,
        project: false,
      },
    },
    rules: {
      'local/no-string-literal-control-flow': 'off',
      '@typescript-eslint/explicit-function-return-type': 'off',
    },
  },
  {
    ignores: ['dist/', 'node_modules/', 'coverage/', 'src/types/generated/'],
  },
)
