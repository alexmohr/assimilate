// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Borg exclude patterns are matched against paths stored without their leading
// slash (`home/alex/.cache`). Every style except `re:` is anchored at the start
// of that path, so a pattern such as `.cache` only matches a top-level `.cache`
// and silently excludes nothing under `/home/<user>`. The pattern has to start
// with `/` (an absolute path) or `*` (any prefix) to reach deeper.

export const PatternStyle = {
  Fnmatch: 'fm',
  Shell: 'sh',
  Regex: 're',
  PathPrefix: 'pp',
  PathFull: 'pf',
} as const

export type PatternStyle = (typeof PatternStyle)[keyof typeof PatternStyle]

export interface ExcludeWarning {
  /** 1-based line number in the raw text. */
  line: number
  pattern: string
  message: string
}

const STYLE_PREFIX = /^([a-z]{2}):/

function isPatternStyle(value: string): value is PatternStyle {
  return (Object.values(PatternStyle) as string[]).includes(value)
}

/** Splits an optional `xx:` style prefix off a pattern; no prefix means fnmatch. */
export function parsePatternStyle(pattern: string): { style: PatternStyle; body: string } {
  const match = STYLE_PREFIX.exec(pattern)
  const prefix = match?.[1]
  if (prefix !== undefined && isPatternStyle(prefix)) {
    return { style: prefix, body: pattern.slice(prefix.length + 1) }
  }
  return { style: PatternStyle.Fnmatch, body: pattern }
}

// Top-level directories of a Linux root. A pattern that begins with one of them
// spells out the whole path from the backup root (`home/*/Downloads`), which is
// how an anchored pattern is meant to be written, so it is not a mistake.
const ROOT_DIRS: ReadonlySet<string> = new Set([
  'bin',
  'boot',
  'dev',
  'etc',
  'home',
  'lib',
  'lib32',
  'lib64',
  'media',
  'mnt',
  'opt',
  'proc',
  'root',
  'run',
  'sbin',
  'snap',
  'srv',
  'sys',
  'tmp',
  'usr',
  'var',
])

function startsAtRootDir(body: string): boolean {
  return ROOT_DIRS.has(body.split('/')[0] ?? '')
}

// Regex is the only style that is not anchored at the start of the path, so it
// never needs a warning or a hint.
type AnchoredStyle = Exclude<PatternStyle, typeof PatternStyle.Regex>

/** Whether `body` can match below the first path component, or is spelled from the root. */
function reachesBelowRoot(style: AnchoredStyle, body: string): boolean {
  if (body.startsWith('/') || startsAtRootDir(body)) {
    return true
  }
  switch (style) {
    case PatternStyle.Fnmatch:
      return body.startsWith('*')
    case PatternStyle.Shell:
      return body.startsWith('**')
    case PatternStyle.PathPrefix:
    case PatternStyle.PathFull:
      return false
  }
}

function fixHint(style: AnchoredStyle, body: string): string {
  switch (style) {
    case PatternStyle.Fnmatch:
      return `use "*/${body}" to match it under every user, or "/${body}" for a path from the backup root`
    case PatternStyle.Shell: {
      // In sh: a single "*" stops at "/", so only "**/" reaches any depth.
      const name = body.replace(/^\*\//, '')
      return `use "**/${name}" to match it under every user, or "/${name}" for a path from the backup root`
    }
    case PatternStyle.PathPrefix:
    case PatternStyle.PathFull:
      return `this style takes no wildcards, so write the whole path from the backup root, such as "/${body}"`
  }
}

function warningFor(pattern: string): string | null {
  const { style, body } = parsePatternStyle(pattern)
  if (style === PatternStyle.Regex || body === '' || reachesBelowRoot(style, body)) {
    return null
  }
  return `"${pattern}" only matches at the very top of the backup root (borg paths have no leading slash); ${fixHint(style, body)}.`
}

/** Flags patterns that look like they match anywhere but are anchored at the root. */
export function excludePatternWarnings(raw: string): ExcludeWarning[] {
  return raw.split('\n').flatMap((rawLine, index) => {
    const pattern = rawLine.trim()
    if (pattern === '' || pattern.startsWith('#')) {
      return []
    }
    const message = warningFor(pattern)
    return message === null ? [] : [{ line: index + 1, pattern, message }]
  })
}
