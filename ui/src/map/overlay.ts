// A change set laid over a map: which boxes changed and how, what each
// container holds, what moved where, and the map cut down to the change.

import type { Change } from '../api'
import type { Cause, MapData, Overlay } from '../layout/types.ts'
import { changeId, isReviewable, reviewKey } from '../model'
import type { Scene } from './scene.ts'

/** How a changed box is coloured. A rename counts as modified. */
export type Tone = 'added' | 'removed' | 'modified' | 'moved'

export function toneOf(change: Change): Tone {
  switch (change.state) {
    case 'added':
      return 'added'
    case 'removed':
      return 'removed'
    case 'moved_in':
    case 'moved_out':
      return 'moved'
    default:
      return 'modified'
  }
}

export interface Mark {
  /** The change of the box's own scope, as a position in the change set. */
  own: number | null
  /**
   * The colour of the box: from its own change, from changes this level
   * draws in it, or from a box around it that was added, removed or moved
   * whole. Null for a box that only holds changed boxes.
   */
  tone: Tone | null
  /** Changes to review in this box and in the boxes inside it. */
  inside: number[]
  counts: Record<Tone, number>
}

export interface Diff {
  changes: Change[]
  /** Only boxes that changed or hold a change have a mark. */
  marks: Record<string, Mark>
  /** The box each change is drawn in. */
  boxOf: Map<number, string>
  /** Changes to review, in reading order: those on the map, and those to tests. */
  order: number[]
  /** Changes to tests. Tests are not on the map, so these have no box. */
  tests: Set<number>
  near: Set<string>
  impacted: Record<string, Cause[]>
  /** A scope that moved: the ghost where it was, the box where it is. */
  moves: { from: string; to: string }[]
}

/** A ghost whose ID was taken by a new scope carries a mark in its ID. */
function bare(id: string): string {
  return id.endsWith(' (before)') ? id.slice(0, -' (before)'.length) : id
}

export function buildDiff(scene: Scene, overlay: Overlay, changes: Change[]): Diff {
  const boxOf = new Map<number, string>()
  for (const [id, positions] of Object.entries(overlay.changes)) {
    if (!scene.boxes[id]) continue
    for (const position of positions) boxOf.set(position, id)
  }

  const marks: Record<string, Mark> = {}
  const blank = (): Mark => ({
    own: null,
    tone: null,
    inside: [],
    counts: { added: 0, removed: 0, modified: 0, moved: 0 },
  })
  // `boxes` is in map order: a parent before its children.
  const ids = Object.keys(scene.boxes)
  const whole = new Map<string, Tone>()
  for (const id of ids) {
    const box = scene.boxes[id]
    const drawn = overlay.changes[id] ?? []
    const own = drawn.find((position) => changeId(changes[position]) === bare(id)) ?? null
    const inherited = box.parent === null ? undefined : whole.get(box.parent)
    let tone: Tone | null = null
    if (own !== null) tone = toneOf(changes[own])
    else if (inherited) tone = inherited
    else if (box.presence !== 'both') tone = box.presence
    else if (drawn.length > 0) tone = 'modified'
    // What is inside a box that was added, removed or moved whole is too.
    if (tone !== null && tone !== 'modified' && (own !== null || inherited || box.presence !== 'both')) {
      whole.set(id, tone)
    }
    if (tone === null && drawn.length === 0) continue
    const mark = blank()
    mark.own = own
    mark.tone = tone
    marks[id] = mark
  }
  // Count upwards: a change is in its box and in every box around it.
  for (const [position, id] of boxOf) {
    const change = changes[position]
    if (!isReviewable(changes, change)) continue
    for (let at: string | null = id; at !== null; at = scene.boxes[at]?.parent ?? null) {
      const mark = (marks[at] ??= blank())
      mark.inside.push(position)
      mark.counts[toneOf(change)]++
    }
  }
  for (const mark of Object.values(marks)) mark.inside.sort((a, b) => a - b)

  const arrivals = new Map<string, number>()
  changes.forEach((change, position) => {
    if (change.state === 'moved_in' && change.new_id) arrivals.set(change.new_id, position)
  })
  const moves: Diff['moves'] = []
  changes.forEach((change, position) => {
    if (change.state !== 'moved_out' || change.counterpart === null) return
    const [from, to] = [boxOf.get(position), boxOf.get(arrivals.get(change.counterpart) ?? -1)]
    if (from && to && from !== to) moves.push({ from, to })
  })

  const tests = new Set(overlay.tests ?? [])
  return {
    changes,
    marks,
    boxOf,
    order: overlay.order.filter(
      (position) =>
        (boxOf.has(position) || tests.has(position)) && isReviewable(changes, changes[position]),
    ),
    tests,
    near: new Set(overlay.near.filter((id) => scene.boxes[id])),
    impacted: Object.fromEntries(
      Object.entries(overlay.impacted).filter(([id]) => scene.boxes[id]),
    ),
    moves,
  }
}

/** How many of the changes in a box, and inside it, are reviewed. */
export function reviewedIn(diff: Diff, id: string, reviewed: Set<string>): { done: number; of: number } {
  const inside = diff.marks[id]?.inside ?? []
  return {
    done: inside.filter((position) => reviewed.has(reviewKey(diff.changes[position]))).length,
    of: inside.length,
  }
}

/** What a container's chip says: how many scopes of each kind of change. */
export function chipOf(mark: Mark): string {
  const parts: string[] = []
  if (mark.counts.added > 0) parts.push(`+${mark.counts.added}`)
  if (mark.counts.modified > 0) parts.push(`~${mark.counts.modified}`)
  if (mark.counts.moved > 0) parts.push(`→${mark.counts.moved}`)
  if (mark.counts.removed > 0) parts.push(`−${mark.counts.removed}`)
  return parts.join(' ')
}

/**
 * The map cut down to a change: the boxes that changed and the boxes
 * around them. With `near`, also the unchanged boxes that use or are used
 * by something that changed.
 *
 * What is inside a changed box stays only if it changed too. A file with
 * one changed import does not bring all of its functions along; a class
 * that was added does, since everything in it was added.
 */
export function isolate(map: MapData, mode: 'changed' | 'near'): MapData {
  const overlay = map.overlay
  if (!overlay) return map
  const parent = new Map(map.nodes.map((node) => [node.id, node.parent]))
  const core = new Set<string>()
  for (const node of map.nodes) {
    if (node.presence !== 'both' || (overlay.changes[node.id]?.length ?? 0) > 0) core.add(node.id)
  }
  const keep = new Set<string>(core)
  if (mode === 'near') {
    for (const id of [...overlay.near, ...Object.keys(overlay.impacted)]) {
      if (parent.has(id)) keep.add(id)
    }
  }
  for (const id of [...keep]) {
    for (let at = parent.get(id) ?? null; at !== null && !keep.has(at); at = parent.get(at) ?? null) {
      keep.add(at)
    }
  }
  const containers: MapData['containers'] = {}
  for (const [id, container] of Object.entries(map.containers)) {
    if (!keep.has(id)) continue
    containers[id] = {
      wires: container.wires.filter((wire) => keep.has(wire.from) && keep.has(wire.to)),
      entries: container.entries.filter((door) => keep.has(door.id)),
      exits: container.exits.filter((door) => keep.has(door.id)),
    }
  }
  return { ...map, nodes: map.nodes.filter((node) => keep.has(node.id)), containers }
}
