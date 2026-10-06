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
 *
 * Text redaction finds a secret only when it sits next to a sensitive key
 * name in the same string: `passphrase: ...`, `token=...`, `Bearer ...`.
 * A secret passed as a separate argument, `logger.error('secret was', pw)`,
 * looks like any other word and can't be detected. Pass secrets as object
 * fields instead, `logger.error('unlock failed', { passphrase })`, which
 * `serialize()` masks by key name.
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

/**
 * The narrower set of keys whose value is also masked when only whitespace
 * separates them, as in `secret hunter2` or `admin_password hunter2`. The
 * word must end in the credential name, so `secretary` or `passwords` don't
 * qualify, and the generic prose words `cookie`, `credential` and
 * `authorization` are left out, so `authorization failed` keeps its next word.
 */
const SPACE_SEPARATED_KEY_SOURCE =
  '[\\w-]*(?:pass(?:word|phrase|wd)|secret|token|api[-_]?key|private[-_]?key|ssh[-_]?key|totp|session[-_]?id)\\b'

interface Rule {
  pattern: RegExp
  replacement: string
}

/**
 * A value an earlier rule already masked: the marker itself, a quoted
 * marker from the quoted-value rule, or a scheme the scheme rule kept, as in
 * `Authorization: Bearer [REDACTED]`. Skipping these keeps the scheme and
 * avoids a doubled marker. Only a scheme word followed by the marker counts:
 * a bare `password: Basic` is still masked, as `Basic` may be the secret
 * itself. The rules' `i` flag makes the scheme match case-insensitive.
 */
const ALREADY_MASKED = `(?!["']?\\[REDACTED\\]|(?:Bearer|Basic|Token)\\s+\\[REDACTED\\])`

/**
 * Masks the value matched by `value` after `keyAndSeparator`, which must
 * capture the key and the separator as its first two groups. The value may
 * not start with whitespace, so the separator takes all of it and can't
 * backtrack past a space to slip around `ALREADY_MASKED`.
 */
function keyValueRule(keyAndSeparator: string, value: string): Rule {
  return {
    pattern: new RegExp(`\\b${keyAndSeparator}(?!\\s)${ALREADY_MASKED}${value}`, 'gi'),
    replacement: `$1$2${REDACTED}`,
  }
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
  // Unquoted values. Each rule masks the value after a sensitive key; they
  // differ only in the separator and in where the value ends. Over-redacting
  // the rest of a line is preferred to leaking part of a multi-word secret.
  //
  // A quoted key with a bare value, "totp": 123456 or 'token': null, is JSON:
  // a bare JSON value has no spaces, so it ends at whitespace, `,`, `}` or
  // `]` and the surrounding JSON keeps its shape.
  keyValueRule(`(${SENSITIVE_KEY_SOURCE})(["']\\s*[:=]\\s*)`, `[^\\s,;&"'}\\]]+`),
  // key=value lists and query strings end a value at `&` so the next pair
  // survives, a=1&token=abc&b=2. `,` and `;` don't end it, as a secret may
  // contain them: `session_id=abc; theme=dark` loses `theme=dark` too.
  keyValueRule(`(${SENSITIVE_KEY_SOURCE})([ \\t]*=[ \\t]*)`, `[^&\\r\\n]+`),
  // Headers, prose and template literals, password: my favorite color, take
  // the rest of the line.
  keyValueRule(`(${SENSITIVE_KEY_SOURCE})([ \\t]*:[ \\t]*)`, `[^\\r\\n]+`),
  // A key from the narrower space-separated list also takes a run of spaces
  // or tabs as its separator, `secret hunter2`, and the rest of the line.
  // Newlines never count as a separator, so a key at the end of a line
  // leaves the next line alone. This over-redacts prose (`password reset
  // failed` loses `reset failed`).
  keyValueRule(`(${SPACE_SEPARATED_KEY_SOURCE})([ \\t]+)`, `[^\\r\\n]+`),
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
