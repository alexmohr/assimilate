// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/** A run of text, and whether it is part of a search match. */
export interface TextPart {
  text: string
  match: boolean
}

/**
 * Splits `text` around every case-insensitive occurrence of `needle`, so a
 * search hit can be highlighted in place. An empty needle matches nothing.
 */
export function matchParts(text: string, needle: string): TextPart[] {
  const wanted = needle.trim().toLowerCase()
  if (wanted === '') return [{ text, match: false }]
  const lower = text.toLowerCase()
  const parts: TextPart[] = []
  let from = 0
  for (;;) {
    const at = lower.indexOf(wanted, from)
    if (at < 0) break
    if (at > from) parts.push({ text: text.slice(from, at), match: false })
    parts.push({ text: text.slice(at, at + wanted.length), match: true })
    from = at + wanted.length
  }
  if (from < text.length || parts.length === 0) parts.push({ text: text.slice(from), match: false })
  return parts
}
