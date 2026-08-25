// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Field-scoped text filtering for list toolbars.
 *
 * A query is a space-separated list of terms that must all match (AND). A term
 * is either bare text - matched against every field - or `field:value`, matched
 * against that field alone. Terms joined by `|` match if any of them does (OR),
 * so `agent:k3s | host:borg-backup` keeps a row belonging to either. Values
 * containing spaces are quoted: `agent:"web server"`. Matching is
 * case-insensitive and on substrings, so `host:borg` finds
 * `borg-backup.example.com`.
 */

/** A field a term can be scoped to with `field:value`. */
export type FilterField = 'name' | 'agent' | 'host' | 'repo'

/** Every scopable field, in the order the help panel lists them. */
export const FILTER_FIELDS: readonly FilterField[] = ['name', 'agent', 'host', 'repo']

/**
 * Spellings accepted for each field. The aliases exist because "host" is
 * ambiguous in a backup tool - the machine being backed up is the *agent*, the
 * machine holding the repository is the *host* - and somebody will type the
 * other word first.
 */
const FIELD_ALIASES = new Map<string, FilterField>([
  ['name', 'name'],
  ['schedule', 'name'],
  ['agent', 'agent'],
  ['client', 'agent'],
  ['host', 'host'],
  ['server', 'host'],
  ['storage', 'host'],
  ['repo', 'repo'],
  ['repository', 'repo'],
])

/** One row of the filter-syntax help panel. */
export interface FilterFieldHelp {
  field: FilterField
  /** A complete term the user could type. */
  example: string
  /** What that term matches. */
  description: string
}

export const FILTER_FIELD_HELP: readonly FilterFieldHelp[] = [
  { field: 'name', example: 'name:nightly', description: 'Name of the entry' },
  { field: 'agent', example: 'agent:k3s', description: 'Agent hostname or display name' },
  {
    field: 'host',
    example: 'host:borg-backup',
    description: 'Storage host the repository lives on',
  },
  { field: 'repo', example: 'repo:server-daily', description: 'Repository name' },
]

/** A single term. `field` is null for bare text, which matches any field. */
export interface FilterTerm {
  field: FilterField | null
  /** Already lowercased, so matching does not lowercase it per candidate. */
  value: string
}

/** Terms joined by `|`: the clause matches when any one of them does. */
export type FilterClause = FilterTerm[]

/** Clauses separated by whitespace: every clause must match. */
export type FilterQuery = FilterClause[]

/**
 * The candidate values a row offers per field. Nulls are allowed so callers can
 * pass an optional field straight through (`host: [repo?.ssh_host ?? null]`).
 */
export type FilterSubject = Record<FilterField, readonly (string | null)[]>

type RawToken = { kind: 'or' } | { kind: 'term'; text: string }

/**
 * Splits the raw input into terms and `|` separators.
 *
 * The alternation keeps a quoted run together, so neither whitespace nor a `|`
 * inside quotes splits a term, and a `field:` prefix stays glued to the quoted
 * value that follows it (`agent:"web server"` is one term). Which alternative
 * matched is read off the capture group rather than by comparing the match
 * text, so no string literal drives control flow here.
 */
function tokenize(input: string): RawToken[] {
  const tokens: RawToken[] = []
  for (const match of input.matchAll(/\||((?:[^\s|"]+|"[^"]*")+)/g)) {
    const text = match[1]
    if (text === undefined) {
      tokens.push({ kind: 'or' })
      continue
    }
    const cleaned = text.replace(/"/g, '').trim()
    if (cleaned) tokens.push({ kind: 'term', text: cleaned })
  }
  return tokens
}

/** The field a `field:` prefix names, or null when it names none. */
function parseFilterField(raw: string): FilterField | null {
  return FIELD_ALIASES.get(raw.trim().toLowerCase()) ?? null
}

/**
 * One term. A `prefix:` that names no known field is not a field at all - the
 * whole token stays bare text, so a name or path containing a colon still
 * filters on itself instead of silently matching nothing.
 *
 * A prefix with an empty value (`host:`, mid-typing) yields no term rather than
 * a term that matches everything or nothing.
 */
function parseTerm(text: string): FilterTerm | null {
  const colon = text.indexOf(':')
  if (colon > 0) {
    const field = parseFilterField(text.slice(0, colon))
    if (field) {
      const value = text
        .slice(colon + 1)
        .trim()
        .toLowerCase()
      return value ? { field, value } : null
    }
  }
  const value = text.trim().toLowerCase()
  return value ? { field: null, value } : null
}

/** Parses raw toolbar input into clauses. Empty input yields an empty query. */
export function parseFilterQuery(input: string): FilterQuery {
  const clauses: FilterQuery = []
  let pendingOr = false

  for (const token of tokenize(input)) {
    if (token.kind === 'or') {
      // A leading `|` has nothing to attach to, so it starts a clause instead.
      pendingOr = clauses.length > 0
      continue
    }
    const term = parseTerm(token.text)
    if (!term) continue
    const last = clauses[clauses.length - 1]
    if (pendingOr && last) last.push(term)
    else clauses.push([term])
    pendingOr = false
  }

  return clauses
}

function matchesTerm(term: FilterTerm, subject: FilterSubject): boolean {
  const fields = term.field ? [term.field] : FILTER_FIELDS
  return fields.some((field) =>
    subject[field].some((candidate) => candidate?.toLowerCase().includes(term.value) ?? false),
  )
}

/** Whether a row matches every clause of the query. An empty query matches all. */
export function matchesFilterQuery(query: FilterQuery, subject: FilterSubject): boolean {
  return query.every((clause) => clause.some((term) => matchesTerm(term, subject)))
}
