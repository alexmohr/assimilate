// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * A deliberately small POSIX-shell tokenizer, used to colour hook commands
 * where they are shown read-only.
 *
 * It is not a shell parser and does not need to be: nothing here decides
 * behaviour, it only decides which span a run of characters lands in, so an
 * exotic construct degrades to plain text rather than to a wrong result. The
 * alternative was a highlighting library - a dependency and a bundle for one
 * read-only block.
 */

/** The span classes a run of characters can land in. */
export type ShellTokenKind =
  | 'comment'
  | 'string'
  | 'variable'
  | 'keyword'
  | 'command'
  | 'operator'
  | 'number'
  | 'text'

/** A contiguous run of source characters sharing one kind. */
export interface ShellToken {
  kind: ShellTokenKind
  text: string
}

/** Words that introduce or continue a shell control structure. */
const KEYWORDS = new Set([
  'if',
  'then',
  'else',
  'elif',
  'fi',
  'for',
  'while',
  'until',
  'do',
  'done',
  'case',
  'esac',
  'in',
  'function',
  'select',
  'return',
  'break',
  'continue',
])

/** Keywords after which the next word is a command again, not an argument. */
const KEYWORDS_INTRODUCING_A_COMMAND = new Set(['then', 'else', 'do', 'elif', 'in'])

/** Operators after which the next word starts a fresh command. */
const OPERATORS_INTRODUCING_A_COMMAND = new Set(['|', '&', ';', '(', '{', '`'])

// Sticky rather than anchored-on-a-slice so the scan stays linear in the
// length of the script: each is re-pointed at the cursor and matched in place.
const WHITESPACE = /\s+/y
const COMMENT = /#[^\n]*/y
const QUOTED = /'[^']*'?|"(?:[^"\\]|\\[\s\S])*"?/y
const ESCAPE = /\\[\s\S]?/y
const BRACED_VARIABLE = /\$\{[^}]*\}?/y
const COMMAND_SUBSTITUTION = /\$\(/y
const VARIABLE = /\$(?:[A-Za-z_][A-Za-z0-9_]*|[0-9]+|[?$!#*@-])/y
const OPERATOR = /[|&;<>(){}`]/y
const WORD = /[^\s|&;<>(){}`'"$\\]+/y
/** Any single character, so an unrecognised one still advances the cursor. */
const ANY = /[\s\S]/y

/** What may precede a `#` for it to open a comment rather than sit in a word. */
const COMMENT_BOUNDARY = /[\s|&;<>(){}`]/
const NUMBER = /^[0-9]+$/
/** A leading `NAME=` assignment, which precedes rather than is a command. */
const ASSIGNMENT_PREFIX = /^[A-Za-z_][A-Za-z0-9_]*(?==)/

/** Matches `pattern` at exactly `index`, returning the matched text. */
function matchAt(pattern: RegExp, source: string, index: number): string | null {
  pattern.lastIndex = index
  return pattern.exec(source)?.[0] ?? null
}

/**
 * Splits `source` into display tokens. Concatenating every token's `text`
 * reproduces `source` exactly, so nothing the user typed is dropped, reordered
 * or re-indented on the way to the screen.
 */
export function highlightShell(source: string): ShellToken[] {
  const tokens: ShellToken[] = []
  // A word is a command name at the start of the script and after anything
  // that ends one; everywhere else it is an argument.
  let atCommandStart = true
  let index = 0

  const push = (kind: ShellTokenKind, text: string): void => {
    if (text.length === 0) return
    const previous = tokens.at(-1)
    if (previous?.kind === kind) {
      previous.text += text
      return
    }
    tokens.push({ kind, text })
  }

  while (index < source.length) {
    const whitespace = matchAt(WHITESPACE, source, index)
    if (whitespace !== null) {
      if (whitespace.includes('\n')) atCommandStart = true
      push('text', whitespace)
      index += whitespace.length
      continue
    }

    const previous = source[index - 1]
    if (previous === undefined || COMMENT_BOUNDARY.test(previous)) {
      const comment = matchAt(COMMENT, source, index)
      if (comment !== null) {
        push('comment', comment)
        index += comment.length
        continue
      }
    }

    const quoted = matchAt(QUOTED, source, index)
    if (quoted !== null) {
      push('string', quoted)
      index += quoted.length
      atCommandStart = false
      continue
    }

    const escape = matchAt(ESCAPE, source, index)
    if (escape !== null) {
      push('text', escape)
      index += escape.length
      continue
    }

    const variable = matchAt(BRACED_VARIABLE, source, index) ?? matchAt(VARIABLE, source, index)
    if (variable !== null) {
      push('variable', variable)
      index += variable.length
      atCommandStart = false
      continue
    }

    const substitution = matchAt(COMMAND_SUBSTITUTION, source, index)
    if (substitution !== null) {
      push('operator', substitution)
      index += substitution.length
      atCommandStart = true
      continue
    }

    const operator = matchAt(OPERATOR, source, index)
    if (operator !== null) {
      push('operator', operator)
      index += operator.length
      if (OPERATORS_INTRODUCING_A_COMMAND.has(operator)) atCommandStart = true
      continue
    }

    const word = matchAt(WORD, source, index)
    if (word === null) {
      const fallback = matchAt(ANY, source, index) ?? ''
      push('text', fallback)
      index += fallback.length || 1
      continue
    }
    index += word.length

    const assignment = atCommandStart ? ASSIGNMENT_PREFIX.exec(word)?.[0] : undefined
    if (assignment !== undefined) {
      push('variable', assignment)
      push('operator', '=')
      push('text', word.slice(assignment.length + 1))
      continue
    }

    if (KEYWORDS.has(word)) {
      push('keyword', word)
      atCommandStart = KEYWORDS_INTRODUCING_A_COMMAND.has(word)
      continue
    }

    if (NUMBER.test(word)) {
      push('number', word)
      atCommandStart = false
      continue
    }

    push(atCommandStart ? 'command' : 'text', word)
    atCommandStart = false
  }

  return tokens
}
