// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { highlightShell, type ShellTokenKind } from './shellHighlight'

/**
 * The kind covering the first occurrence of `text`. Located by offset rather
 * than by an exact token match, since adjacent runs of one kind are merged
 * into a single token - `stop app` arrives as one `text` run, not two.
 */
function kindOf(source: string, text: string): ShellTokenKind | undefined {
  const target = source.indexOf(text)
  let offset = 0
  for (const token of highlightShell(source)) {
    if (target >= offset && target < offset + token.text.length) return token.kind
    offset += token.text.length
  }
  return undefined
}

describe('highlightShell', () => {
  // The one property everything else rests on: highlighting is presentation,
  // so it must never be able to change, drop or reorder what the user wrote.
  it('reproduces the source exactly when the tokens are concatenated', () => {
    const script = [
      '# remount before dumping',
      'STORAGE=truenas-gremlin-backup',
      'if ! mountpoint -q "/mnt/pve/$STORAGE"; then',
      '    umount -f -l "/mnt/pve/$STORAGE" 2>/dev/null || true',
      'fi',
      '',
      'vzdump --all 1 --storage "$STORAGE" --mode snapshot',
    ].join('\n')
    expect(
      highlightShell(script)
        .map((token) => token.text)
        .join(''),
    ).toBe(script)
  })

  it('returns nothing for an empty script', () => {
    expect(highlightShell('')).toEqual([])
  })

  it('marks the first word of a command', () => {
    expect(kindOf('systemctl stop app', 'systemctl')).toBe('command')
    expect(kindOf('systemctl stop app', 'stop')).toBe('text')
  })

  it('starts a new command after a newline, a pipe and a semicolon', () => {
    expect(kindOf('echo a\ngrep b', 'grep')).toBe('command')
    expect(kindOf('echo a | grep b', 'grep')).toBe('command')
    expect(kindOf('echo a; grep b', 'grep')).toBe('command')
  })

  it('marks a comment to the end of its line only', () => {
    const tokens = highlightShell('rm -f x # why\nls')
    expect(tokens.find((t) => t.kind === 'comment')?.text).toBe('# why')
    expect(kindOf('rm -f x # why\nls', 'ls')).toBe('command')
  })

  // `#` is only a comment at a word boundary: inside a word it is an ordinary
  // character, and colouring the rest of the line as a comment would make a
  // command that runs look like one that does not.
  it('leaves a mid-word hash as part of the word', () => {
    expect(highlightShell('curl http://host/a#b').some((t) => t.kind === 'comment')).toBe(false)
  })

  it('marks single- and double-quoted strings', () => {
    expect(kindOf("echo 'one two'", "'one two'")).toBe('string')
    expect(kindOf('echo "one two"', '"one two"')).toBe('string')
  })

  it('does not run an unterminated quote past the end of the script', () => {
    const tokens = highlightShell("echo 'oops")
    expect(tokens.map((t) => t.text).join('')).toBe("echo 'oops")
  })

  it('marks variables in both bare and braced form', () => {
    expect(kindOf('echo $STORAGE', '$STORAGE')).toBe('variable')
    expect(kindOf('echo ${f%.zst}', '${f%.zst}')).toBe('variable')
  })

  it('splits a leading assignment into its name and the equals sign', () => {
    const tokens = highlightShell('STORAGE=truenas')
    expect(tokens[0]).toEqual({ kind: 'variable', text: 'STORAGE' })
    expect(tokens[1]).toEqual({ kind: 'operator', text: '=' })
  })

  // Only at command position: `--set foo=bar` is an argument, and colouring
  // half of it as a variable would suggest an assignment that is not one.
  it('leaves an equals sign inside an argument alone', () => {
    expect(highlightShell('mount -o rw,uid=0 /mnt')[0]?.kind).toBe('command')
    expect(kindOf('mount -o rw,uid=0 /mnt', 'rw,uid=0')).toBe('text')
  })

  it('marks shell keywords and treats the word after them as a command', () => {
    expect(kindOf('if true; then echo hi; fi', 'if')).toBe('keyword')
    expect(kindOf('if true; then echo hi; fi', 'then')).toBe('keyword')
    expect(kindOf('if true; then echo hi; fi', 'echo')).toBe('command')
  })

  it('marks a bare number as a number rather than a command', () => {
    expect(kindOf('sleep 2', '2')).toBe('number')
  })

  it('marks redirection and control operators', () => {
    expect(kindOf('a > b', '>')).toBe('operator')
    expect(kindOf('a || b', '|')).toBe('operator')
  })

  it('treats a command substitution as opening a fresh command', () => {
    expect(kindOf('for id in $(ls -1); do :; done', 'ls')).toBe('command')
  })

  // A trailing backslash is how a long command wraps onto the next line, so
  // it turns up in exactly the multi-line hook scripts this renders.
  it('keeps an escaped character with the surrounding text', () => {
    const script = 'vzdump --all 1 \\\n    --storage backup-store'
    expect(
      highlightShell(script)
        .map((token) => token.text)
        .join(''),
    ).toBe(script)
    expect(kindOf(script, '\\')).toBe('text')
  })

  // Without the escape branch the backslash would fall through to the quote
  // matcher on the next character and open a string that never closes.
  it('does not let an escaped quote open a string', () => {
    expect(highlightShell('echo \\" still text').some((t) => t.kind === 'string')).toBe(false)
  })

  // A bare `$` starts no expansion and is excluded from a word, so it leaves
  // the loop only through the single-character fallback - without which the
  // cursor would never advance past it and the render would hang.
  it('advances past a character no matcher claims, such as a bare $', () => {
    expect(
      highlightShell('echo $')
        .map((token) => token.text)
        .join(''),
    ).toBe('echo $')
  })

  it('merges adjacent runs of the same kind into one token', () => {
    const tokens = highlightShell('a && b')
    expect(tokens.filter((t) => t.kind === 'operator').map((t) => t.text)).toEqual(['&&'])
  })
})
