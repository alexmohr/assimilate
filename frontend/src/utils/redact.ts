// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Turns arbitrary values handed to the logger into bounded, secret-free text.
 *
 * The client log buffer keeps what it is given for as long as the tab is
 * open and lets an admin copy it out, so nothing that reaches it may carry a
 * passphrase, password, token, SSH key or Authorization header. Redaction
 * runs once, when an entry is recorded: the buffer never holds the original
 * value, only the redacted string.
 */

export const REDACTED = '[REDACTED]'

/** Longest message or stack a single entry keeps, in characters. */
export const MAX_TEXT_LENGTH = 4000

const MAX_DEPTH = 4
const MAX_KEYS = 50
const MAX_ITEMS = 50

/**
 * A key whose value is a credential, wherever it appears: an object
 * property, a `key=value` pair in a URL or a `key: value` header line.
 */
const SENSITIVE_KEY_SOURCE =
  '[\\w-]*(?:pass(?:word|phrase|wd)|secret|token|authori[sz]ation|cookie|api[-_]?key|private[-_]?key|ssh[-_]?key|totp|credential|session[-_]?id)[\\w-]*'

const SENSITIVE_KEY = new RegExp(`^${SENSITIVE_KEY_SOURCE}$`, 'i')

interface Rule {
  pattern: RegExp
  replacement: string
}

/**
 * Applied in order. The scheme rule runs before the key/value rules so that
 * `Authorization: Bearer abc` loses the credential rather than the word
 * `Bearer`.
 */
const RULES: readonly Rule[] = [
  // PEM private keys, including a truncated block with no END line.
  {
    pattern:
      /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)/g,
    replacement: REDACTED,
  },
  // OpenSSH key lines: keep the algorithm, drop the key material.
  {
    pattern: /\b(ssh-(?:rsa|dss|ed25519)|ecdsa-sha2-nistp\d+|sk-[\w.@-]+)\s+[A-Za-z0-9+/=]{16,}/g,
    replacement: `$1 ${REDACTED}`,
  },
  // HTTP auth schemes.
  {
    pattern: /\b(Bearer|Basic|Token)\s+[A-Za-z0-9._~+/=-]+/gi,
    replacement: `$1 ${REDACTED}`,
  },
  // JSON-style quoted values: "password": "two words".
  {
    pattern: new RegExp(
      `(["']?)(${SENSITIVE_KEY_SOURCE})\\1(\\s*[:=]\\s*)(["'])(?:\\\\.|(?!\\4)[^\\\\])*\\4`,
      'gi',
    ),
    replacement: `$1$2$1$3$4${REDACTED}$4`,
  },
  // Unquoted values: token=abc&next, Authorization: abc, password: abc.
  // Skips a value the scheme rule above already masked, so
  // `Authorization: Bearer [REDACTED]` keeps its scheme instead of turning
  // into a doubled marker. Only a scheme word followed by the marker is
  // skipped: a bare `password: Basic` is still masked, as `Basic` may be the
  // secret itself. The `i` flag makes the scheme match case-insensitive, like
  // the scheme rule.
  {
    pattern: new RegExp(
      `\\b(${SENSITIVE_KEY_SOURCE})(\\s*[:=]\\s*)(?!\\[REDACTED\\]|(?:Bearer|Basic|Token)\\s+\\[REDACTED\\])[^\\s,;&"'}\\]]+`,
      'gi',
    ),
    replacement: `$1$2${REDACTED}`,
  },
  // JSON Web Tokens.
  {
    pattern: /\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*/g,
    replacement: REDACTED,
  },
  // Bare hex runs the length of an API or agent token (32+ random bytes).
  {
    pattern: /\b[0-9a-fA-F]{32,}\b/g,
    replacement: REDACTED,
  },
]

export function isSensitiveKey(key: string): boolean {
  return SENSITIVE_KEY.test(key)
}

function truncate(text: string): string {
  if (text.length <= MAX_TEXT_LENGTH) return text
  return `${text.slice(0, MAX_TEXT_LENGTH)}... (${text.length - MAX_TEXT_LENGTH} more characters)`
}

/** Removes every credential pattern from `text` and caps its length. */
export function redactText(text: string): string {
  let out = text
  for (const rule of RULES) out = out.replace(rule.pattern, rule.replacement)
  return truncate(out)
}

function isPlainObject(value: object): boolean {
  const proto: unknown = Object.getPrototypeOf(value)
  return proto === Object.prototype || proto === null
}

function describeInstance(value: object): string {
  const name = value.constructor?.name
  return name ? `[${name}]` : '[object]'
}

/**
 * A JSON-ish rendering of `value` with sensitive keys masked. Only plain
 * objects and arrays are walked - a DOM node, an event or a class instance
 * is named, not traversed, which keeps the cost bounded and stops a large
 * graph from being copied into the buffer.
 */
function serialize(value: unknown, depth: number, seen: WeakSet<object>): string {
  if (value === null) return 'null'
  switch (typeof value) {
    case 'string':
      return depth === 0 ? value : JSON.stringify(value)
    case 'number':
    case 'boolean':
    case 'bigint':
    case 'undefined':
      return String(value)
    case 'symbol':
      return value.toString()
    case 'function':
      return `[Function ${value.name || 'anonymous'}]`
    default:
      break
  }
  const obj = value as object
  if (obj instanceof Error) return `${obj.name}: ${obj.message}`
  if (seen.has(obj)) return '[Circular]'
  if (depth >= MAX_DEPTH) return Array.isArray(obj) ? '[Array]' : '[Object]'
  if (Array.isArray(obj)) {
    seen.add(obj)
    const items = obj.slice(0, MAX_ITEMS).map((item) => serialize(item, depth + 1, seen))
    if (obj.length > MAX_ITEMS) items.push(`... ${obj.length - MAX_ITEMS} more`)
    return `[${items.join(', ')}]`
  }
  if (!isPlainObject(obj)) return describeInstance(obj)
  seen.add(obj)
  const entries = Object.entries(obj)
  const parts = entries
    .slice(0, MAX_KEYS)
    .map(([key, val]) =>
      isSensitiveKey(key) ? `${key}: ${REDACTED}` : `${key}: ${serialize(val, depth + 1, seen)}`,
    )
  if (entries.length > MAX_KEYS) parts.push(`... ${entries.length - MAX_KEYS} more`)
  return `{${parts.join(', ')}}`
}

/** One logger argument as redacted text. */
export function redactValue(value: unknown): string {
  return redactText(serialize(value, 0, new WeakSet()))
}

/** The arguments of one logger call joined into a single redacted message. */
export function formatLogArgs(args: readonly unknown[]): string {
  return redactText(args.map((arg) => serialize(arg, 0, new WeakSet())).join(' '))
}

/** The stack of the first `Error` among `args`, redacted, or `null`. */
export function firstErrorStack(args: readonly unknown[]): string | null {
  for (const arg of args) {
    if (arg instanceof Error && arg.stack) return redactText(arg.stack)
  }
  return null
}
