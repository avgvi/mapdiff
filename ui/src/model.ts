// Pure logic over a change set: which rows the tree shows, what counts as
// reviewable, and how a diff is laid out. No React, no fetching.

import type { Change, ChangeSet, Row } from './api'

export interface Filters {
  kind: 'all' | 'signatures' | 'bodies'
  access: 'all' | 'public'
  hideFormatting: boolean
}

export const NO_FILTERS: Filters = { kind: 'all', access: 'all', hideFormatting: false }

export function changeId(change: Change): string {
  return change.new_id ?? change.old_id ?? ''
}

function isScope(change: Change): boolean {
  return change.kind !== 'directory' && change.kind !== 'file' && change.kind !== 'document'
}

/** True when the change is part of a scope that was added or removed whole. */
function impliedByParent(changes: Change[], change: Change): boolean {
  if (change.parent === null) return false
  if (change.state !== 'added' && change.state !== 'removed') return false
  const parent = changes[change.parent]
  return isScope(parent) && parent.state === change.state
}

/**
 * A change the reviewer has to look at and can mark as reviewed: something
 * that changed itself, as opposed to a directory, or a scope that only holds
 * changed scopes.
 */
export function isReviewable(changes: Change[], change: Change): boolean {
  if (change.kind === 'directory') return false
  if (impliedByParent(changes, change)) return false
  const ownLines = change.own.added + change.own.removed > 0
  return ownLines || change.state !== 'modified' || change.children.length === 0
}

/** The key a review mark is stored under. Editing the scope again changes it. */
export function reviewKey(change: Change): string {
  return `${changeId(change)}\u001f${change.fingerprint}`
}

/**
 * Every key to set when one change is marked: its own, the other end of a
 * move, and for a container everything reviewable inside it.
 */
export function keysToMark(changes: Change[], index: number): string[] {
  const keys = new Set<string>()
  const visit = (position: number) => {
    const change = changes[position]
    if (isReviewable(changes, change)) {
      keys.add(reviewKey(change))
      if (change.counterpart !== null) {
        const other = changes.find(
          (candidate) =>
            candidate.fingerprint === change.fingerprint &&
            changeId(candidate) === change.counterpart,
        )
        if (other) keys.add(reviewKey(other))
      }
    }
    for (const child of change.children) visit(child)
  }
  visit(index)
  return [...keys]
}

export interface Progress {
  reviewed: number
  total: number
}

/** How much of a change and everything inside it is reviewed. */
export function progressOf(changes: Change[], index: number, reviewed: Set<string>): Progress {
  const progress = { reviewed: 0, total: 0 }
  const visit = (position: number) => {
    const change = changes[position]
    // The arriving end of a move is counted; the leaving end is the same change.
    if (isReviewable(changes, change) && change.state !== 'moved_out') {
      progress.total += 1
      if (reviewed.has(reviewKey(change))) progress.reviewed += 1
    }
    for (const child of change.children) visit(child)
  }
  visit(index)
  return progress
}

function passesFilters(change: Change, filters: Filters): boolean {
  if (filters.hideFormatting && change.kinds.length === 1 && change.kinds[0] === 'formatting') {
    return false
  }
  if (filters.access === 'public' && change.access !== 'public') return false
  switch (filters.kind) {
    case 'all':
      return true
    case 'signatures':
      // What other code can see: signatures, and anything that appeared,
      // disappeared, or changed its name or place.
      return change.kinds.includes('signature') || change.state !== 'modified'
    case 'bodies':
      return change.kinds.includes('body')
  }
}

export interface TreeRow {
  /** Position of the change in the change set. */
  index: number
  depth: number
  /** The label to show. Directories with one child are joined: `a/b/c`. */
  label: string
  hasChildren: boolean
  collapsed: boolean
  reviewable: boolean
}

/**
 * The tree as a flat list of rows, in display order. A change is listed when
 * it passes the filters or holds one that does. The root is left out.
 */
export function treeRows(
  changeSet: ChangeSet,
  filters: Filters,
  collapsed: Set<number>,
): TreeRow[] {
  const { changes } = changeSet
  if (changes.length === 0) return []

  const shown: boolean[] = Array.from({ length: changes.length }, () => false)
  for (let index = changes.length - 1; index >= 0; index -= 1) {
    const change = changes[index]
    const itself = isReviewable(changes, change) && passesFilters(change, filters)
    // Members of an added or removed scope come along with it.
    const implied =
      impliedByParent(changes, change) && change.parent !== null && filters.kind === 'all'
    shown[index] = itself || implied || change.children.some((child) => shown[child])
  }

  const rows: TreeRow[] = []
  const visit = (start: number, depth: number) => {
    let index = start
    let label = changes[index].label
    // Join a chain of directories that each hold just one directory.
    for (;;) {
      const change = changes[index]
      const visible = change.children.filter((child) => shown[child])
      const only = visible.length === 1 ? changes[visible[0]] : null
      if (change.kind === 'directory' && only?.kind === 'directory') {
        index = visible[0]
        label = `${label}/${only.label}`
      } else {
        break
      }
    }
    const change = changes[index]
    const children = change.children.filter((child) => shown[child])
    const isCollapsed = collapsed.has(index)
    rows.push({
      index,
      depth,
      label,
      hasChildren: children.length > 0,
      collapsed: isCollapsed,
      reviewable: isReviewable(changes, change),
    })
    if (!isCollapsed) {
      for (const child of children) visit(child, depth + 1)
    }
  }
  for (const child of changes[0].children) {
    if (shown[child]) visit(child, 0)
  }
  return rows
}

/** Changes that start collapsed: scopes added or removed whole. */
export function initiallyCollapsed(changeSet: ChangeSet): Set<number> {
  const collapsed = new Set<number>()
  changeSet.changes.forEach((change, index) => {
    const whole = change.state === 'added' || change.state === 'removed'
    if (whole && isScope(change) && change.children.length > 0) collapsed.add(index)
  })
  return collapsed
}

/** One line of the before and after view. Either side can be empty. */
export interface DisplayRow {
  kind: 'pair'
  left: Row | null
  right: Row | null
}

/** A run of unchanged lines folded away until asked for. */
export interface FoldRow {
  kind: 'fold'
  /** Position of the first hidden row in the unfolded list. */
  key: number
  hidden: DisplayRow[]
}

/** Unchanged lines kept in view on each side of a change. */
const CONTEXT = 3

/**
 * Lays rows out side by side: an unchanged line on both sides, and each run
 * of deleted lines next to the inserted lines that replaced it.
 */
export function pairRows(rows: Row[]): DisplayRow[] {
  const out: DisplayRow[] = []
  let position = 0
  while (position < rows.length) {
    const row = rows[position]
    if (row.tag === 'equal') {
      out.push({ kind: 'pair', left: row, right: row })
      position += 1
      continue
    }
    const deleted: Row[] = []
    const inserted: Row[] = []
    while (position < rows.length && rows[position].tag === 'delete') {
      deleted.push(rows[position])
      position += 1
    }
    while (position < rows.length && rows[position].tag === 'insert') {
      inserted.push(rows[position])
      position += 1
    }
    const length = Math.max(deleted.length, inserted.length)
    for (let offset = 0; offset < length; offset += 1) {
      out.push({ kind: 'pair', left: deleted[offset] ?? null, right: inserted[offset] ?? null })
    }
  }
  return out
}

/**
 * Folds long runs of unchanged lines, keeping a few around each change.
 * Runs listed in `open` stay unfolded.
 */
export function foldRows(rows: DisplayRow[], open: Set<number>): (DisplayRow | FoldRow)[] {
  const isEqual = (row: DisplayRow) => row.left !== null && row.left === row.right
  // With nothing changed there is nothing to give context to.
  if (rows.every(isEqual)) return rows

  const out: (DisplayRow | FoldRow)[] = []
  let position = 0
  while (position < rows.length) {
    if (!isEqual(rows[position])) {
      out.push(rows[position])
      position += 1
      continue
    }
    let end = position
    while (end < rows.length && isEqual(rows[end])) end += 1
    // Keep context after the change before this run and before the one after it.
    const from = position === 0 ? position : position + CONTEXT
    const to = end === rows.length ? end : end - CONTEXT
    // Folding fewer than a few lines hides almost nothing and costs a click.
    if (to - from < 4 || open.has(from)) {
      out.push(...rows.slice(position, end))
    } else {
      out.push(...rows.slice(position, from))
      out.push({ kind: 'fold', key: from, hidden: rows.slice(from, to) })
      out.push(...rows.slice(to, end))
    }
    position = end
  }
  return out
}
