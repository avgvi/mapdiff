// A laid-out map, indexed the ways the viewer needs it.

import { absolute } from '../layout/layout.ts'
import type { Box, Level, MapData, MapLayout, MapNode } from '../layout/types.ts'

export interface Scene {
  level: Level
  layout: MapLayout
  /** Every box with its corner measured from the map's top left corner. */
  boxes: Record<string, Box>
  nodes: Record<string, MapNode>
  children: Record<string, string[]>
  /** For each container, the size of a typical child: the average, leaving small marks out. */
  childSize: Record<string, { width: number; height: number }>
}

export function buildScene(map: MapData, layout: MapLayout): Scene {
  const nodes: Record<string, MapNode> = {}
  const children: Record<string, string[]> = {}
  for (const node of map.nodes) {
    if (!layout.boxes[node.id]) continue
    nodes[node.id] = node
    if (node.parent !== null) (children[node.parent] ??= []).push(node.id)
  }
  const boxes = absolute(layout)
  const childSize: Scene['childSize'] = {}
  for (const [id, kids] of Object.entries(children)) {
    const real = kids.map((kid) => boxes[kid]).filter((box) => box.detail !== 'pill')
    if (real.length === 0) continue
    childSize[id] = {
      width: real.reduce((sum, box) => sum + box.width, 0) / real.length,
      height: real.reduce((sum, box) => sum + box.height, 0) / real.length,
    }
  }
  return { level: map.level, layout, boxes, nodes, children, childSize }
}

/** The boxes around a box, from the root down to its parent. */
export function ancestorsOf(scene: Scene, id: string): string[] {
  const chain: string[] = []
  for (let at = scene.boxes[id]?.parent ?? null; at !== null; at = scene.boxes[at]?.parent ?? null) {
    chain.push(at)
  }
  return chain.reverse()
}

/** The file a box is in: itself, or the nearest file around it. */
export function fileOf(scene: Scene, id: string): string | null {
  for (let at: string | null = id; at !== null; at = scene.boxes[at]?.parent ?? null) {
    const kind = scene.nodes[at]?.kind
    if (kind === 'file' || kind === 'document') return at
  }
  return null
}

/** The deepest box under a point of the map, or null outside the map. */
export function boxAt(scene: Scene, x: number, y: number): string | null {
  const inside = (id: string) => {
    const box = scene.boxes[id]
    return x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height
  }
  let at = scene.layout.root
  if (!inside(at)) return null
  for (;;) {
    const next = (scene.children[at] ?? []).find(inside)
    if (!next) return at
    at = next
  }
}
