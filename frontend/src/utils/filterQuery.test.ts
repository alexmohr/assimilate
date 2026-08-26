// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import {
  parseFilterQuery,
  matchesFilterQuery,
  FILTER_FIELDS,
  FILTER_FIELD_HELP,
  type FilterSubject,
} from './filterQuery'

function subject(overrides: Partial<FilterSubject> = {}): FilterSubject {
  return {
    name: ['Nightly backup'],
    agent: ['k3s-node-01', 'Kubernetes node (k3s-node-01)'],
    host: ['borg-backup.example.com'],
    repo: ['server-daily'],
    ...overrides,
  }
}

function matches(input: string, s: FilterSubject = subject()): boolean {
  return matchesFilterQuery(parseFilterQuery(input), s)
}

describe('parseFilterQuery', () => {
  it('returns an empty query for blank input', () => {
    expect(parseFilterQuery('')).toEqual([])
    expect(parseFilterQuery('   ')).toEqual([])
  })

  it('parses a bare term as an unscoped clause', () => {
    expect(parseFilterQuery('nightly')).toEqual([[{ field: null, value: 'nightly' }]])
  })

  it('lowercases values so matching never has to', () => {
    expect(parseFilterQuery('Agent:K3S')).toEqual([[{ field: 'agent', value: 'k3s' }]])
  })

  it('separates whitespace-delimited terms into AND clauses', () => {
    expect(parseFilterQuery('agent:k3s host:borg')).toEqual([
      [{ field: 'agent', value: 'k3s' }],
      [{ field: 'host', value: 'borg' }],
    ])
  })

  it('joins terms around a pipe into one OR clause, with or without spaces', () => {
    const glued = parseFilterQuery('agent:k3s|host:borg')
    const spaced = parseFilterQuery('agent:k3s | host:borg')
    expect(glued).toEqual([
      [
        { field: 'agent', value: 'k3s' },
        { field: 'host', value: 'borg' },
      ],
    ])
    expect(spaced).toEqual(glued)
  })

  it('accepts field aliases', () => {
    expect(parseFilterQuery('repository:daily')).toEqual([[{ field: 'repo', value: 'daily' }]])
    expect(parseFilterQuery('client:k3s')).toEqual([[{ field: 'agent', value: 'k3s' }]])
  })

  it('keeps an unknown prefix as bare text rather than dropping the term', () => {
    // A schedule named "db:primary" must still filter on itself.
    expect(parseFilterQuery('db:primary')).toEqual([[{ field: null, value: 'db:primary' }]])
  })

  it('ignores a prefix with no value yet', () => {
    expect(parseFilterQuery('host:')).toEqual([])
  })

  it('keeps a quoted value together, spaces and pipes included', () => {
    expect(parseFilterQuery('agent:"web server"')).toEqual([
      [{ field: 'agent', value: 'web server' }],
    ])
    expect(parseFilterQuery('name:"a|b"')).toEqual([[{ field: 'name', value: 'a|b' }]])
  })

  it('starts a clause on a leading pipe instead of dropping the term', () => {
    expect(parseFilterQuery('| agent:k3s')).toEqual([[{ field: 'agent', value: 'k3s' }]])
  })

  it('ignores a trailing pipe', () => {
    expect(parseFilterQuery('agent:k3s |')).toEqual([[{ field: 'agent', value: 'k3s' }]])
  })
})

describe('matchesFilterQuery', () => {
  it('matches everything when the query is empty', () => {
    expect(matches('')).toBe(true)
  })

  it('matches a bare term against any field', () => {
    expect(matches('nightly')).toBe(true)
    expect(matches('k3s')).toBe(true)
    expect(matches('borg-backup')).toBe(true)
    expect(matches('server-daily')).toBe(true)
    expect(matches('nothing-here')).toBe(false)
  })

  it('scopes a field term to that field alone', () => {
    expect(matches('agent:k3s')).toBe(true)
    // The agent hostname is not the storage host, so scoping rules it out.
    expect(matches('host:k3s')).toBe(false)
    expect(matches('host:borg-backup')).toBe(true)
  })

  it('matches case-insensitively on substrings', () => {
    expect(matches('AGENT:K3S-NODE')).toBe(true)
    expect(matches('name:NIGHT')).toBe(true)
  })

  it('requires every whitespace-separated clause to match', () => {
    expect(matches('agent:k3s host:borg-backup')).toBe(true)
    expect(matches('agent:k3s host:other-host')).toBe(false)
  })

  it('requires only one term of an OR clause to match', () => {
    expect(matches('agent:k3s|host:other-host')).toBe(true)
    expect(matches('agent:nope|host:nope')).toBe(false)
  })

  it('combines OR clauses with AND', () => {
    expect(matches('agent:k3s|agent:nas repo:server-daily')).toBe(true)
    expect(matches('agent:k3s|agent:nas repo:other-repo')).toBe(false)
  })

  it('tolerates a field whose value is unknown', () => {
    const unknownHost = subject({ host: [null] })
    expect(matches('host:borg', unknownHost)).toBe(false)
    expect(matches('agent:k3s', unknownHost)).toBe(true)
  })
})

describe('FILTER_FIELD_HELP', () => {
  it('documents every field the parser accepts', () => {
    expect(FILTER_FIELD_HELP.map((row) => row.field)).toEqual([...FILTER_FIELDS])
  })

  it('gives every row an example the parser scopes to that field', () => {
    for (const row of FILTER_FIELD_HELP) {
      expect(parseFilterQuery(row.example)).toEqual([
        [{ field: row.field, value: expect.any(String) }],
      ])
    }
  })
})
