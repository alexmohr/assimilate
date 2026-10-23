// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The archive browser's selection: which archive paths are ticked, kept as
 * the smallest set of paths that covers everything ticked.
 *
 * Paths are archive paths as borg stores them: no leading slash, `''` for the
 * archive root. A ticked directory is one entry that covers everything under
 * it, so the selection and the restore request name the directory rather than
 * every file inside it, and borg restores it in one go.
 */

// borg's entry-type character for a directory; an external format contract.
const DIRECTORY_ENTRY_TYPE = 'd'

/** Whether an archive entry, by borg's entry type, is a directory. */
export function isDirectoryEntry(entry: { type: string }): boolean {
  return entry.type === DIRECTORY_ENTRY_TYPE
}

/** The tri-state shown by a row's checkbox. */
export type TickState = 'checked' | 'partial' | 'unchecked'

/**
 * The full list of a directory's children, or `null` while the browser has
 * not loaded all of them. Selection never guesses a directory's contents: an
 * operation that needs a listing it does not have asks for it instead.
 */
export type ChildrenOf = (dir: string) => readonly string[] | null

/** The result of unticking: the new selection, or the directories to load first. */
export type UntickResult = { selection: string[] } | { missing: string[] }

/** The directory holding `path`; the root holds itself. */
export function parentOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash < 0 ? '' : path.slice(0, slash)
}

/** Whether `path` is `ancestor` itself or lies anywhere beneath it. */
export function isUnder(path: string, ancestor: string): boolean {
  return ancestor === '' || path === ancestor || path.startsWith(`${ancestor}/`)
}

/** Whether `path` lies strictly beneath `ancestor`. */
function isStrictlyUnder(path: string, ancestor: string): boolean {
  return path !== ancestor && isUnder(path, ancestor)
}

/** `path`'s ancestors from the outermost (just below the root) down to its parent. */
function ancestorsOf(path: string): string[] {
  const parts = path.split('/')
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join('/'))
}

export function tickState(selection: readonly string[], path: string): TickState {
  if (selection.some((ticked) => isUnder(path, ticked))) return 'checked'
  if (selection.some((ticked) => isStrictlyUnder(ticked, path))) return 'partial'
  return 'unchecked'
}

/**
 * The same selection in its smallest form, in the order it was ticked:
 * nothing listed under something else listed, and a directory whose every
 * child is ticked listed in their place. The archive root is never collapsed into, so ticking everything at
 * the top stays a list of the top-level entries a restore can name.
 */
export function collapse(selection: readonly string[], childrenOf: ChildrenOf): string[] {
  let current = [...new Set(selection)].filter(
    (path, _, all) => !all.some((other) => isStrictlyUnder(path, other)),
  )
  for (;;) {
    const ticked = new Set(current)
    const full = [...new Set(current.map(parentOf))].find((dir) => {
      if (dir === '') return false
      const children = childrenOf(dir)
      return (
        children !== null && children.length > 0 && children.every((child) => ticked.has(child))
      )
    })
    if (full === undefined) return current
    current = [...current.filter((path) => !isUnder(path, full)), full]
  }
}

/** Ticks `path` and so everything beneath it. */
export function tick(selection: readonly string[], path: string, childrenOf: ChildrenOf): string[] {
  return collapse([...selection.filter((ticked) => !isUnder(ticked, path)), path], childrenOf)
}

/**
 * Unticks `path` and everything beneath it. When `path` was covered by a
 * ticked ancestor, that ancestor is split into the siblings along the way
 * down, which needs each of those directories' full listing.
 */
export function untick(
  selection: readonly string[],
  path: string,
  childrenOf: ChildrenOf,
): UntickResult {
  const remaining = selection.filter((ticked) => !isUnder(ticked, path))
  const covering = remaining.find((ticked) => isStrictlyUnder(path, ticked))
  if (covering === undefined) return { selection: collapse(remaining, childrenOf) }

  const chain = [...ancestorsOf(path).filter((dir) => isStrictlyUnder(dir, covering)), path]
  const dirs = [covering, ...chain.slice(0, -1)]
  const missing = dirs.filter((dir) => childrenOf(dir) === null)
  if (missing.length > 0) return { missing }

  const siblings = dirs.flatMap((dir, i) =>
    (childrenOf(dir) ?? []).filter((child) => child !== chain[i]),
  )
  return {
    selection: collapse(
      [...remaining.filter((ticked) => ticked !== covering), ...siblings],
      childrenOf,
    ),
  }
}

/** What clicking a row's checkbox does: a ticked row is unticked, anything else ticked. */
export function toggle(
  selection: readonly string[],
  path: string,
  childrenOf: ChildrenOf,
): UntickResult {
  return tickState(selection, path) === 'checked'
    ? untick(selection, path, childrenOf)
    : { selection: tick(selection, path, childrenOf) }
}

/** Ticks every one of `paths`: a folder's "select everything here". */
export function tickAll(
  selection: readonly string[],
  paths: readonly string[],
  childrenOf: ChildrenOf,
): string[] {
  return collapse(
    [...selection.filter((ticked) => !paths.some((path) => isUnder(ticked, path))), ...paths],
    childrenOf,
  )
}
