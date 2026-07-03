// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { PrimeVuePTOptions } from 'primevue/config'

// PrimeVue's own `severity`/`variant` prop values (an external, open-ended
// contract -- PrimeVue supports more values than these two), not app-owned
// domain state.
const SEVERITY_DANGER = 'danger'
const SEVERITY_SECONDARY = 'secondary'
const VARIANT_TEXT = 'text'
const VARIANT_LINK = 'link'

function buttonRootClasses(options: {
  props: { severity?: string; variant?: string; text?: boolean }
}): string {
  const severity = options.props.severity ?? ''
  const variant = options.props.variant ?? ''

  if (severity === SEVERITY_DANGER) return 'btn btn-danger'
  if (
    severity === SEVERITY_SECONDARY ||
    variant === VARIANT_TEXT ||
    variant === VARIANT_LINK ||
    options.props.text
  ) {
    return 'btn btn-ghost'
  }
  return 'btn btn-primary'
}

function selectRootClasses(): string {
  return 'inline-flex min-h-9 w-full items-center gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg-input)] px-3 py-2 text-sm text-[var(--text-primary)] transition-colors hover:bg-[var(--bg-hover)] focus-within:border-[var(--accent)] focus-within:shadow-[0_0_0_3px_var(--accent-subtle)]'
}

function selectOverlayClasses(): string {
  return 'mt-1 rounded-[var(--radius)] border border-[var(--border)] bg-[var(--bg-card)] shadow-lg'
}

function paginatorControlClasses(): string {
  return 'inline-flex h-8 min-w-8 items-center justify-center rounded-sm border border-[var(--border)] bg-[var(--bg-input)] px-2 text-sm text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] disabled:cursor-not-allowed disabled:opacity-50'
}

export const globalPrimeVuePT: PrimeVuePTOptions = {
  button: {
    root: buttonRootClasses,
  },
  datatable: {
    root: 'overflow-hidden rounded-[var(--radius)] border border-[var(--border)] bg-[var(--bg-base)]',
    tableContainer: 'overflow-x-auto',
    table: 'w-full text-sm',
    thead: 'border-b border-[var(--border)]',
    headerRow:
      'border-b border-[var(--border)] text-xs uppercase tracking-[0.05em] text-[var(--text-muted)]',
    tbody: 'divide-y divide-[var(--border-subtle)]',
    bodyRow: 'transition-colors hover:bg-[var(--bg-hover)]',
    emptyMessageCell: 'px-4 py-8 text-center text-sm text-[var(--text-muted)]',
  },
  dialog: {
    root: 'fixed inset-0 z-50 flex items-center justify-center p-4',
    mask: 'fixed inset-0 z-50 bg-black/50',
    content: 'w-full max-w-2xl rounded-lg bg-[var(--bg-card)] p-6 shadow-xl',
    header: 'mb-4 flex items-start justify-between gap-4 p-0',
    title: 'text-lg font-semibold text-[var(--text-primary)]',
    footer: 'mt-4 flex items-center justify-end gap-3 p-0',
    pcCloseButton: {
      root: 'btn btn-ghost btn-sm border-0 p-0',
    },
  },
  inputtext: {
    root: 'input',
  },
  select: {
    root: selectRootClasses,
    label: 'flex-1 truncate text-sm text-[var(--text-primary)]',
    dropdown: 'ml-auto inline-flex h-5 w-5 items-center justify-center text-[var(--text-muted)]',
    dropdownIcon: 'text-[var(--text-muted)]',
    overlay: selectOverlayClasses(),
    header: 'border-b border-[var(--border)] p-2',
    listContainer: 'max-h-64 overflow-auto p-1',
    list: 'flex flex-col gap-0.5',
    option:
      'cursor-pointer rounded-sm px-3 py-2 text-sm text-[var(--text-primary)] transition-colors hover:bg-[var(--bg-hover)]',
    optionLabel: 'truncate',
    emptyMessage: 'px-3 py-2 text-sm text-[var(--text-muted)]',
  },
  dropdown: {
    root: selectRootClasses,
    label: 'flex-1 truncate text-sm text-[var(--text-primary)]',
    dropdown: 'ml-auto inline-flex h-5 w-5 items-center justify-center text-[var(--text-muted)]',
    dropdownIcon: 'text-[var(--text-muted)]',
    overlay: selectOverlayClasses(),
    header: 'border-b border-[var(--border)] p-2',
    listContainer: 'max-h-64 overflow-auto p-1',
    list: 'flex flex-col gap-0.5',
    option:
      'cursor-pointer rounded-sm px-3 py-2 text-sm text-[var(--text-primary)] transition-colors hover:bg-[var(--bg-hover)]',
    optionLabel: 'truncate',
    emptyMessage: 'px-3 py-2 text-sm text-[var(--text-muted)]',
  },
  paginator: {
    root: 'flex items-center justify-between gap-4 text-sm text-[var(--text-secondary)]',
    content: 'flex items-center gap-2',
    contentStart: 'flex items-center gap-2',
    contentEnd: 'flex items-center gap-2',
    pages: 'flex items-center gap-1',
    page: paginatorControlClasses,
    current:
      'inline-flex h-8 min-w-8 items-center justify-center rounded-sm border border-[var(--accent)] bg-[var(--accent)] px-2 text-sm text-[var(--text-on-accent)]',
    prev: paginatorControlClasses,
    next: paginatorControlClasses,
    first: paginatorControlClasses,
    last: paginatorControlClasses,
  },
}
