// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import {
  MAX_TEXT_LENGTH,
  REDACTED,
  firstErrorStack,
  formatLogArgs,
  isSensitiveKey,
  redactText,
  redactValue,
} from './redact'

/**
 * Assembled at run time so the source holds no literal PEM header for the
 * repository's secret scanner to flag; the key bodies below are not keys.
 */
function pemMarker(edge: 'BEGIN' | 'END', kind: string): string {
  return ['-----', edge, ' ', kind, ' PRIVATE', ' KEY', '-----'].join('')
}

const HEX_TOKEN = 'a'.repeat(32) + '0123456789abcdef'.repeat(2)

describe('redactText', () => {
  it('leaves ordinary text alone', () => {
    expect(redactText('fetchLogs failed: network error')).toBe('fetchLogs failed: network error')
  })

  it('masks bearer and basic credentials but keeps the scheme', () => {
    expect(redactText('Authorization: Bearer abc.def-123')).not.toContain('abc.def-123')
    expect(redactText('Authorization: Bearer abc.def-123')).toBe(
      `Authorization: Bearer ${REDACTED}`,
    )
    expect(redactText('sent Bearer abc.def-123')).toBe(`sent Bearer ${REDACTED}`)
    expect(redactText('Basic dXNlcjpwYXNz')).toBe(`Basic ${REDACTED}`)
  })

  it('masks a scheme credential after a sensitive key with a single marker', () => {
    expect(redactText('Authorization: Basic dXNlcjpwYXNz')).toBe(`Authorization: Basic ${REDACTED}`)
    expect(redactText('token: Token xyz')).toBe(`token: Token ${REDACTED}`)
    expect(redactText('authorization: bearer abc.def-123')).toBe(
      `authorization: bearer ${REDACTED}`,
    )
    expect(redactText('AUTHORIZATION=BASIC dXNlcjpwYXNz')).toBe(`AUTHORIZATION=BASIC ${REDACTED}`)
  })

  it('still masks a bare scheme word used as the value itself', () => {
    expect(redactText('password: Basic')).toBe(`password: ${REDACTED}`)
  })

  it('masks bare values after quoted JSON keys and keeps the JSON shape', () => {
    const cases: readonly (readonly [string, string, readonly string[]])[] = [
      ['{"totp_code":123456}', `{"totp_code":${REDACTED}}`, ['123456']],
      [
        '{"session_id":42,"api_key":987654321}',
        `{"session_id":${REDACTED},"api_key":${REDACTED}}`,
        ['42', '987654321'],
      ],
      ['{"password": true}', `{"password": ${REDACTED}}`, ['true']],
      ["{'token': null}", `{'token': ${REDACTED}}`, ['null']],
      [
        '{"user":{"name":"ann","secret":-12.5e3},"page":2}',
        `{"user":{"name":"ann","secret":${REDACTED}},"page":2}`,
        ['-12.5e3'],
      ],
      ['{"token":7,"next":1}', `{"token":${REDACTED},"next":1}`, ['"token":7']],
    ]
    for (const [input, expected, secrets] of cases) {
      const out = redactText(input)
      expect(out).toBe(expected)
      for (const secret of secrets) expect(out).not.toContain(secret)
    }
  })

  it('masks a value separated from its key by spaces or tabs only', () => {
    const cases: readonly (readonly [string, string, string])[] = [
      ['login with secret hunter2', `login with secret ${REDACTED}`, 'hunter2'],
      ['password hunter2', `password ${REDACTED}`, 'hunter2'],
      ['totp 123456', `totp ${REDACTED}`, '123456'],
      ['secret   hunter2', `secret   ${REDACTED}`, 'hunter2'],
      ['passphrase\thunter2', `passphrase\t${REDACTED}`, 'hunter2'],
      ['PassWord Hunter2', `PassWord ${REDACTED}`, 'Hunter2'],
      ['refresh_token abc123', `refresh_token ${REDACTED}`, 'abc123'],
    ]
    for (const [input, expected, secret] of cases) {
      const out = redactText(input)
      expect(out).toBe(expected)
      expect(out).not.toContain(secret)
    }
  })

  it('keeps the next word after a generic or longer word', () => {
    expect(redactText('authorization failed for cookie consent')).toBe(
      'authorization failed for cookie consent',
    )
    expect(redactText('the secretary passwords list')).toBe('the secretary passwords list')
    expect(redactText('enter the secret\nnext line')).toBe('enter the secret\nnext line')
  })

  it('masks an Authorization header value without a scheme', () => {
    expect(redactText('Authorization: s3cr3t')).toBe(`Authorization: ${REDACTED}`)
  })

  it('masks key=value pairs in a URL', () => {
    const out = redactText('GET /api/x?token=abc123&page=2&api_key=zzz')
    expect(out).toBe(`GET /api/x?token=${REDACTED}&page=2&api_key=${REDACTED}`)
  })

  it('masks quoted JSON values, including ones with spaces', () => {
    const out = redactText('{"username":"admin","password":"correct horse battery"}')
    expect(out).toBe(`{"username":"admin","password":"${REDACTED}"}`)
  })

  it('masks passphrase, secret and TOTP fields', () => {
    expect(redactText('passphrase: hunter2')).toBe(`passphrase: ${REDACTED}`)
    expect(redactText("client_secret='x y'")).toBe(`client_secret='${REDACTED}'`)
    expect(redactText('totp_code=123456')).toBe(`totp_code=${REDACTED}`)
  })

  it('masks a PEM private key block, even a truncated one', () => {
    const pem = `${pemMarker('BEGIN', 'OPENSSH')}\nb3BlbnNzaC1rZXktdjE\n${pemMarker('END', 'OPENSSH')}`
    expect(redactText(`key: ${pem} done`)).not.toContain('b3BlbnNzaC1rZXktdjE')
    expect(redactText(`${pemMarker('BEGIN', 'RSA')}\nMIIEow`)).toBe(REDACTED)
  })

  it('masks OpenSSH key material but keeps the algorithm', () => {
    const line = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
    expect(redactText(line)).toBe(`ssh-ed25519 ${REDACTED}`)
  })

  it('masks bare hex tokens and JWTs', () => {
    expect(redactText(`agent token ${HEX_TOKEN}`)).toBe(`agent token ${REDACTED}`)
    expect(redactText('jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig_part')).toBe(`jwt ${REDACTED}`)
  })

  it('caps very long text', () => {
    const out = redactText('x'.repeat(MAX_TEXT_LENGTH + 10))
    expect(out.startsWith('x'.repeat(MAX_TEXT_LENGTH))).toBe(true)
    expect(out).toContain('10 more characters')
  })
})

describe('isSensitiveKey', () => {
  it.each([
    'password',
    'new_password',
    'passphrase',
    'Authorization',
    'token',
    'api_key',
    'ssh_key',
    'private_key',
    'cookie',
    'totp_secret',
    'session_id',
  ])('treats %s as sensitive', (key) => {
    expect(isSensitiveKey(key)).toBe(true)
  })

  it.each(['username', 'hostname', 'author', 'session_expires_at', 'message'])(
    'leaves %s visible',
    (key) => {
      expect(isSensitiveKey(key)).toBe(false)
    },
  )
})

describe('redactValue', () => {
  it('masks sensitive keys at any depth', () => {
    const out = redactValue({
      user: 'admin',
      headers: { Authorization: 'Bearer xyz', Accept: 'json' },
      body: { repo: { passphrase: 'p@ss' } },
    })
    expect(out).not.toContain('xyz')
    expect(out).not.toContain('p@ss')
    expect(out).toContain('user: "admin"')
    expect(out).toContain('Accept: "json"')
    expect(out).toContain(`Authorization: ${REDACTED}`)
  })

  it('names class instances instead of walking them', () => {
    class Holder {
      password = 'leak'
    }
    expect(redactValue(new Holder())).toBe('[Holder]')
  })

  it('renders errors by name and message only', () => {
    const err = new TypeError('bad token=abc')
    expect(redactValue(err)).toBe(`TypeError: bad token=${REDACTED}`)
  })

  it('survives circular and deeply nested values', () => {
    const a: Record<string, unknown> = { name: 'a' }
    a.self = a
    expect(redactValue(a)).toContain('[Circular]')
    expect(redactValue({ l1: { l2: { l3: { l4: { l5: 1 } } } } })).toContain('[Object]')
  })

  it('bounds large arrays and objects', () => {
    expect(redactValue(Array.from({ length: 60 }, (_, i) => i))).toContain('... 10 more')
    const wide = Object.fromEntries(Array.from({ length: 55 }, (_, i) => [`k${i}`, i]))
    expect(redactValue(wide)).toContain('... 5 more')
  })

  it('renders primitives, functions and symbols', () => {
    expect(redactValue(null)).toBe('null')
    expect(redactValue(undefined)).toBe('undefined')
    expect(redactValue(42)).toBe('42')
    expect(redactValue(10n)).toBe('10')
    expect(redactValue(Symbol('s'))).toBe('Symbol(s)')
    expect(redactValue(function named(): void {})).toBe('[Function named]')
    expect(redactValue(['x', true])).toBe('["x", true]')
  })
})

describe('formatLogArgs', () => {
  it('joins arguments with spaces and redacts the result', () => {
    expect(formatLogArgs(['login failed', { password: 'pw' }, 401])).toBe(
      `login failed {password: ${REDACTED}} 401`,
    )
  })
})

describe('firstErrorStack', () => {
  it('returns the redacted stack of the first error', () => {
    const err = new Error('boom')
    err.stack = 'Error: boom token=abc\n    at f (app.js:1:1)'
    expect(firstErrorStack(['context', err])).toBe(
      `Error: boom token=${REDACTED}\n    at f (app.js:1:1)`,
    )
  })

  it('returns null without an error argument', () => {
    expect(firstErrorStack(['text', 1])).toBeNull()
  })
})
