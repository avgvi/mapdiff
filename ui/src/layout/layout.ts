// The layout of a whole map, from the inside out: a container can only be
// placed once its size is known, and its size is the layout of what it
// holds.

import {
  finish,
  keyOf,
  plan,
  type ContainerInput,
  type ContainerResult,
  type Plan,
} from './container.ts'
import type { ElkNode, Engine } from './engine.ts'
import { detailOf, sizeOf, titleOf, titleWidth } from './sizes.ts'
import type { Box, ContainerLayout, Detail, MapData, MapLayout, MapNode } from './types.ts'

/**
 * Layouts of containers, by everything they depend on: the children, their
 * sizes, the wires. A container whose key has been seen is not laid out
 * again, so after a change only the containers around it are.
 */
export class LayoutCache {
  private readonly known = new Map<string, ContainerResult>()

  get(key: string): ContainerResult | undefined {
    return this.known.get(key)
  }

  set(key: string, result: ContainerResult): void {
    this.known.set(key, result)
  }

  get size(): number {
    return this.known.size
  }
}

interface Entry {
  node: MapNode
  children: string[]
  depth: number
  detail: Detail
  width: number
  height: number
}

/**
 * Lays out a map. Pass the same cache again to reuse what has not changed.
 * `whole` names the code boxes that show all of their text, not the first
 * lines of it.
 */
export async function layoutMap(
  map: MapData,
  engine: Engine,
  cache: LayoutCache = new LayoutCache(),
  whole: ReadonlySet<string> = new Set(),
): Promise<MapLayout> {
  const started = performance.now()
  const entries = new Map<string, Entry>()
  let root: string | null = null
  for (const node of map.nodes) {
    const parent = node.parent === null ? undefined : entries.get(node.parent)
    if (node.parent !== null && !parent) continue
    if (node.parent === null) {
      if (root !== null) continue
      root = node.id
    }
    parent?.children.push(node.id)
    entries.set(node.id, {
      node,
      children: [],
      depth: parent ? parent.depth + 1 : 0,
      detail: 'title',
      width: 0,
      height: 0,
    })
  }
  if (root === null) throw new Error('the map has no root')

  // Leaves have a size of their own. Containers wait, by depth.
  const levels: string[][] = []
  for (const [id, entry] of entries) {
    entry.detail = detailOf(entry.node, map.level, entry.children.length > 0)
    if (entry.detail === 'container') {
      ;(levels[entry.depth] ??= []).push(id)
    } else {
      Object.assign(entry, sizeOf(entry.node, entry.detail, whole.has(id)))
    }
  }

  const results = new Map<string, ContainerResult>()
  const stats = { containers: 0, laidOut: 0, engineCalls: 0, milliseconds: 0 }
  for (let depth = levels.length - 1; depth >= 0; depth--) {
    // Everything at one depth goes to the engine in one call.
    const waiting: { id: string; key: string; plan: Plan }[] = []
    for (const id of levels[depth] ?? []) {
      stats.containers++
      const entry = entries.get(id)!
      const wires = map.containers[id]
      const input: ContainerInput = {
        titleWidth: titleWidth(titleOf(entry.node)),
        children: entry.children.map((child) => {
          const { width, height, detail } = entries.get(child)!
          return { id: child, width, height, row: detail === 'row', pill: detail === 'pill' }
        }),
        wires: (wires?.wires ?? []).map((wire) => ({
          from: wire.from,
          to: wire.to,
          against: wire.kind === 'cycle',
        })),
        entries: (wires?.entries ?? []).map((door) => door.id),
        exits: (wires?.exits ?? []).map((door) => door.id),
      }
      const key = keyOf(input)
      const cached = cache.get(key)
      if (cached) {
        results.set(id, cached)
        entry.width = cached.width
        entry.height = cached.height
      } else {
        waiting.push({ id, key, plan: plan(input, `c${waiting.length}`) })
      }
    }

    const graphs = waiting.flatMap((item) => (item.plan.graph ? [item.plan.graph] : []))
    const flowed = new Map<string, ElkNode>()
    if (graphs.length > 0) {
      stats.engineCalls++
      const answer = await engine.layout({
        id: 'all',
        // Where the containers end up next to each other is not used.
        layoutOptions: { 'elk.algorithm': 'fixed' },
        children: graphs,
      })
      for (const graph of answer.children ?? []) flowed.set(graph.id, graph)
    }
    for (const item of waiting) {
      stats.laidOut++
      const graph = item.plan.graph
      const result = finish(item.plan, graph ? (flowed.get(graph.id) ?? null) : null)
      cache.set(item.key, result)
      results.set(item.id, result)
      const entry = entries.get(item.id)!
      entry.width = result.width
      entry.height = result.height
    }
  }

  const boxes: Record<string, Box> = {}
  const containers: Record<string, ContainerLayout> = {}
  for (const [id, entry] of entries) {
    const within = entry.node.parent === null ? undefined : results.get(entry.node.parent)
    const at = within?.positions[id] ?? { x: 0, y: 0 }
    boxes[id] = {
      id,
      parent: entry.node.parent,
      x: at.x,
      y: at.y,
      width: entry.detail === 'row' && within ? within.rowWidth : entry.width,
      height: entry.height,
      detail: entry.detail,
      presence: entry.node.presence,
    }
    const result = results.get(id)
    if (!result) continue
    containers[id] = {
      wires: (map.containers[id]?.wires ?? []).map((wire, position) => ({
        from: wire.from,
        to: wire.to,
        kind: wire.kind,
        count: wire.count,
        presence: wire.presence,
        route: result.wires[position] ?? null,
      })),
      entries: result.entries,
      exits: result.exits,
    }
  }

  stats.milliseconds = performance.now() - started
  const top = entries.get(root)!
  return { root, width: top.width, height: top.height, boxes, containers, stats }
}

/** Every box with its corner measured from the map's top left corner. */
export function absolute(layout: MapLayout): Record<string, Box> {
  const out: Record<string, Box> = {}
  // `boxes` keeps the order of the map: a parent before its children.
  for (const [id, box] of Object.entries(layout.boxes)) {
    const parent = box.parent === null ? undefined : out[box.parent]
    out[id] = { ...box, x: box.x + (parent?.x ?? 0), y: box.y + (parent?.y ?? 0) }
  }
  return out
}
